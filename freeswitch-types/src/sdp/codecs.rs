//! Collection of codecs parsed from a complete SDP session description.

use std::collections::HashMap;

use crate::sdp::{
    codec::{SdpCodec, SdpDirection, SdpMediaType},
    codec_string::{CodecString, CodecStringEntry},
    error::{CodecStringError, SdpCodecError, SdpWarning, UnmappedPayload},
    options::CodecStringOptions,
    static_payload,
};

/// A single entry in a parsed SDP offer.
///
/// Telephone-event (DTMF) and CN (comfort noise) are excluded from entries;
/// they are available separately via [`SdpCodecs::telephone_event_rates`] and
/// [`SdpCodecs::has_comfort_noise`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SdpCodecEntry {
    /// An RTP codec negotiated via `a=rtpmap` or the RFC 3551 static table.
    Rtp(SdpCodec),
    /// T.38 fax relay, derived from any `m=image` section with a nonzero port.
    ///
    /// The proto and fmt fields are not inspected — FreeSWITCH negotiates T.38
    /// parameters independently of the codec string.
    T38,
}

/// Codecs and ancillary data extracted from an SDP session description.
///
/// Produced by [`SdpCodecs::parse`]. Reflects the same extraction logic used by
/// FreeSWITCH's `switch_core_media_set_r_sdp_codec_string` to build
/// `ep_codec_string`. See `docs/codec-string-format.md` for the mapping rules.
#[derive(Debug)]
pub struct SdpCodecs {
    entries: Vec<SdpCodecEntry>,
    unmapped: Vec<UnmappedPayload>,
    warnings: Vec<SdpWarning>,
    telephone_event_rates: Vec<u32>,
    has_comfort_noise: bool,
}

impl SdpCodecs {
    /// Parse an SDP session description from a UTF-8 string.
    pub fn parse(sdp: &str) -> Result<Self, SdpCodecError> {
        Self::parse_bytes(sdp.as_bytes())
    }

    /// Parse an SDP session description from bytes.
    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, SdpCodecError> {
        let session = sdp_types::Session::parse(bytes)
            .map_err(|e| SdpCodecError::parse_failure("failed to parse SDP", e))?;
        Self::from_session(&session)
    }

    fn from_session(session: &sdp_types::Session) -> Result<Self, SdpCodecError> {
        let mut result = Self {
            entries: Vec::new(),
            unmapped: Vec::new(),
            warnings: Vec::new(),
            telephone_event_rates: Vec::new(),
            has_comfort_noise: false,
        };

        // Session-level defaults; media-level attributes override these per section.
        let session_ptime = ptime_from_attrs(&session.attributes, "ptime", &mut result.warnings);
        let session_maxptime =
            ptime_from_attrs(&session.attributes, "maxptime", &mut result.warnings);
        let session_direction =
            direction_from_attrs(&session.attributes).unwrap_or(SdpDirection::SendRecv);

        for media in &session.medias {
            if media.port == 0 {
                continue;
            }

            // Any m=image with nonzero port yields T38 regardless of proto/fmt.
            if media
                .media
                .eq_ignore_ascii_case("image")
            {
                result
                    .entries
                    .push(SdpCodecEntry::T38);
                continue;
            }

            // SdpMediaType::from_str is infallible (Err = std::convert::Infallible).
            let media_type: SdpMediaType = match media
                .media
                .parse()
            {
                Ok(t) => t,
                Err(e) => match e {},
            };

            // Only audio and video sections contribute codec entries.
            match media_type {
                SdpMediaType::Audio | SdpMediaType::Video => {}
                _ => continue,
            }

            // A single structurally broken section (bad rtpmap/fmtp/payload type)
            // must not discard every other section already parsed. Stage this
            // section's output locally and only merge it in on success.
            match parse_media_section(
                media,
                media_type.clone(),
                session_ptime,
                session_maxptime,
                session_direction,
            ) {
                Ok(section) => {
                    result
                        .entries
                        .extend(section.entries);
                    result
                        .unmapped
                        .extend(section.unmapped);
                    result
                        .warnings
                        .extend(section.warnings);
                    for rate in section.telephone_event_rates {
                        if !result
                            .telephone_event_rates
                            .contains(&rate)
                        {
                            result
                                .telephone_event_rates
                                .push(rate);
                        }
                    }
                    result.has_comfort_noise |= section.has_comfort_noise;
                }
                Err(e) => {
                    result
                        .warnings
                        .push(SdpWarning::malformed_media_section(
                            media_type.to_string(),
                            e.to_string(),
                        ));
                }
            }
        }

        Ok(result)
    }

    /// All codec entries in SDP offer order.
    pub fn entries(&self) -> &[SdpCodecEntry] {
        &self.entries
    }

    /// Iterator over codec entries in SDP offer order.
    pub fn iter(&self) -> std::slice::Iter<'_, SdpCodecEntry> {
        self.entries
            .iter()
    }

    /// Consume this collection, returning the inner entry vector.
    pub fn into_entries(self) -> Vec<SdpCodecEntry> {
        self.entries
    }

    /// Number of codec entries (telephone-event and CN are not counted).
    pub fn len(&self) -> usize {
        self.entries
            .len()
    }

    /// `true` when the offer contained no codecs (after filtering telephone-event/CN).
    pub fn is_empty(&self) -> bool {
        self.entries
            .is_empty()
    }

    /// Iterator over audio RTP codecs only.
    pub fn audio(&self) -> impl Iterator<Item = &SdpCodec> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                SdpCodecEntry::Rtp(c) if c.media() == &SdpMediaType::Audio => Some(c),
                _ => None,
            })
    }

    /// Iterator over video RTP codecs only.
    pub fn video(&self) -> impl Iterator<Item = &SdpCodec> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                SdpCodecEntry::Rtp(c) if c.media() == &SdpMediaType::Video => Some(c),
                _ => None,
            })
    }

    /// Retain only the entries for which `f` returns `true`.
    pub fn retain(&mut self, f: impl FnMut(&SdpCodecEntry) -> bool) {
        self.entries
            .retain(f);
    }

    /// Payload types that could not be named (no `a=rtpmap`, no static-table entry).
    ///
    /// These are surfaced as data rather than an error so the caller can distinguish
    /// "never offered" from "offered but unresolvable".
    pub fn unmapped(&self) -> &[UnmappedPayload] {
        &self.unmapped
    }

    /// Recoverable parse warnings (e.g. unparseable numeric attributes).
    pub fn warnings(&self) -> &[SdpWarning] {
        &self.warnings
    }

    /// Clock rates (Hz) at which `telephone-event` was offered.
    ///
    /// DTMF is negotiated outside the codec string; these rates are never included
    /// in [`entries`](Self::entries) or [`unmapped`](Self::unmapped).
    pub fn telephone_event_rates(&self) -> &[u32] {
        &self.telephone_event_rates
    }

    /// `true` when `CN` (comfort noise) was offered in any media section.
    ///
    /// CN is negotiated outside the codec string and never appears in
    /// [`entries`](Self::entries) or [`unmapped`](Self::unmapped).
    pub fn has_comfort_noise(&self) -> bool {
        self.has_comfort_noise
    }

    /// Build a FreeSWITCH codec string from this offer's audio codecs.
    ///
    /// Emits a literal `t38` entry for each `m=image` section the offer carried, in
    /// its original m-line position among the audio entries — C writes `,t38` inside
    /// the same per-m-line loop that emits audio codecs (`switch_core_media.c:13651`),
    /// so an `m=image` section before an `m=audio` section puts `t38` first, not last.
    /// Does not deduplicate — an offer carrying the AMR octet-aligned/bandwidth-efficient
    /// pair yields two identical entries under default options (they differ only in
    /// fmtp, which is off by default for audio). Call [`CodecString::dedup`] after
    /// composing the final string.
    ///
    /// `warnings = None` is strict: any unrepresentable codec name or fmtp is `Err`.
    /// `warnings = Some(acc)` is lenient: an unrepresentable fmtp is cleared (the codec
    /// is still emitted) and an unrepresentable codec name is skipped (the codec is
    /// dropped); both push a warning to `acc` rather than failing the whole call.
    pub fn audio_codec_string(
        &self,
        options: &CodecStringOptions,
        mut warnings: Option<&mut Vec<SdpWarning>>,
    ) -> Result<CodecString, CodecStringError> {
        let mut out = CodecString::new();
        for entry in &self.entries {
            match entry {
                // The literal "t38" contains no codec-string grammar delimiter, so this
                // can't actually fail; propagating via `?` still means a mistaken future
                // rename of the literal surfaces as a returned Err, never a silently
                // dropped entry in release builds.
                SdpCodecEntry::T38 => out.push(CodecStringEntry::new("t38")?),
                SdpCodecEntry::Rtp(codec) if codec.media() == &SdpMediaType::Audio => {
                    if let Some(entry) =
                        codec_to_entry_lenient(codec, options, warnings.as_deref_mut())?
                    {
                        out.push(entry);
                    }
                }
                SdpCodecEntry::Rtp(_) => {}
            }
        }
        Ok(out)
    }

    /// Build a FreeSWITCH codec string from this offer's video codecs.
    ///
    /// No T.38 entry is appended — that belongs only to
    /// [`audio_codec_string`](Self::audio_codec_string). Does not deduplicate; see there
    /// for why. `warnings` follows the same strict/lenient convention.
    pub fn video_codec_string(
        &self,
        options: &CodecStringOptions,
        mut warnings: Option<&mut Vec<SdpWarning>>,
    ) -> Result<CodecString, CodecStringError> {
        let mut out = CodecString::new();
        for codec in self.video() {
            if let Some(entry) = codec_to_entry_lenient(codec, options, warnings.as_deref_mut())? {
                out.push(entry);
            }
        }
        Ok(out)
    }

    /// Look up the fmtp of an offered audio codec by name and clock rate.
    ///
    /// Drives `rtp_force_audio_fmtp` once the caller knows which codec ended up first
    /// in its final, composed, filtered codec string — "first codec in the offer" is
    /// routinely not that codec, so this takes the name and rate explicitly rather than
    /// exposing a "primary" accessor that would often name the wrong payload map.
    ///
    /// Several payload types commonly share one name+rate with different fmtp — the
    /// AMR octet-aligned/bandwidth-efficient pair is the recurring case. When they
    /// disagree, the first one carrying an fmtp wins; a caller driving
    /// `rtp_force_audio_fmtp` from this can otherwise pin octet-aligned when the
    /// switch negotiated bandwidth-efficient.
    pub fn fmtp_for(&self, name: &str, clock_rate: u32) -> Option<&str> {
        self.audio()
            .filter(|c| {
                c.name()
                    .eq_ignore_ascii_case(name)
                    && c.clock_rate() == clock_rate
            })
            .find_map(|c| c.fmtp())
    }
}

/// Convert one [`SdpCodec`] to a [`CodecStringEntry`], splitting the two failure classes
/// `audio_codec_string`/`video_codec_string` must handle differently: a name that cannot
/// be embedded at all (`Ok(None)` lenient / `Err` strict, the codec is skipped) versus an
/// fmtp that cannot be embedded (the entry is still emitted with fmtp cleared).
fn codec_to_entry_lenient(
    codec: &SdpCodec,
    options: &CodecStringOptions,
    mut warnings: Option<&mut Vec<SdpWarning>>,
) -> Result<Option<CodecStringEntry>, CodecStringError> {
    let mut entry = match CodecStringEntry::new(codec.name()) {
        Ok(e) => e,
        Err(e) => {
            return match warnings.as_deref_mut() {
                None => Err(e),
                Some(acc) => {
                    acc.push(SdpWarning::codec_name_unrepresentable(
                        codec.name(),
                        e.to_string(),
                    ));
                    Ok(None)
                }
            };
        }
    };

    if options.emits_fmtp() {
        if let Some(fmtp) = codec.fmtp() {
            match entry
                .clone()
                .with_fmtp(fmtp)
            {
                Ok(with_fmtp) => entry = with_fmtp,
                Err(e) => match warnings {
                    None => return Err(e),
                    Some(acc) => {
                        acc.push(SdpWarning::fmtp_unrepresentable(
                            codec.name(),
                            fmtp,
                            e.to_string(),
                        ));
                        // entry keeps no fmtp; qualifiers below still apply.
                    }
                },
            }
        }
    }

    Ok(Some(options.apply_qualifiers(entry, codec)))
}

// --- private helpers ---

/// One `m=` section's contribution, staged locally so a mid-section parse
/// failure can be discarded wholesale instead of polluting `SdpCodecs`.
#[derive(Default)]
struct MediaSection {
    entries: Vec<SdpCodecEntry>,
    unmapped: Vec<UnmappedPayload>,
    warnings: Vec<SdpWarning>,
    telephone_event_rates: Vec<u32>,
    has_comfort_noise: bool,
}

/// Parse one audio/video `m=` section into its codec entries.
///
/// Returns `Err` only for structural breakage within this section (an
/// unparseable `a=rtpmap`, `a=fmtp`, or `m=` format-list payload type) — the
/// caller records this as [`SdpWarning::MalformedMediaSection`] and skips the
/// section rather than aborting the whole parse.
fn parse_media_section(
    media: &sdp_types::Media,
    media_type: SdpMediaType,
    session_ptime: Option<u32>,
    session_maxptime: Option<u32>,
    session_direction: SdpDirection,
) -> Result<MediaSection, SdpCodecError> {
    let mut section = MediaSection::default();

    // Media-level attributes override session-level.
    let media_ptime =
        ptime_from_attrs(&media.attributes, "ptime", &mut section.warnings).or(session_ptime);
    let media_maxptime =
        ptime_from_attrs(&media.attributes, "maxptime", &mut section.warnings).or(session_maxptime);
    let media_direction = direction_from_attrs(&media.attributes).unwrap_or(session_direction);

    // Build per-section rtpmap and fmtp lookup tables.
    let mut rtpmap: HashMap<u8, (String, u32, Option<u8>)> = HashMap::new();
    let mut fmtp_map: HashMap<u8, String> = HashMap::new();

    // Attribute names are compared byte-exactly here and in ptime_from_attrs/
    // direction_from_attrs below, unlike FreeSWITCH's own attribute walk, which uses
    // strcasecmp (switch_core_media.c:13658). RFC 8866 makes attribute names
    // case-sensitive, so byte-exact stays the intended behaviour here; this is a
    // documented divergence, not an oversight.
    for attr in &media.attributes {
        match attr
            .attribute
            .as_str()
        {
            "rtpmap" => {
                if let Some(val) = &attr.value {
                    let (pt, name, rate, channels) = parse_rtpmap(val)?;
                    rtpmap.insert(pt, (name, rate, channels));
                }
            }
            "fmtp" => {
                if let Some(val) = &attr.value {
                    let (pt, params) = parse_fmtp_pt(val)?;
                    fmtp_map.insert(pt, params);
                }
            }
            _ => {}
        }
    }

    for pt_str in media
        .fmt
        .split_whitespace()
    {
        let pt = pt_str
            .parse::<u8>()
            .map_err(|_| SdpCodecError::NonNumericPayloadType(pt_str.to_string()))?;

        // Name, clock rate, and channel count from rtpmap or RFC 3551 static table.
        let (name, clock_rate, rtpmap_channels, has_rtpmap) =
            if let Some((n, r, c)) = rtpmap.get(&pt) {
                (n.as_str(), *r, *c, true)
            } else if let Some(st) = static_payload::rfc3551_payload_type(pt) {
                (st.encoding_name, st.clock_rate, st.channels, false)
            } else {
                section
                    .unmapped
                    .push(UnmappedPayload::new(pt, media_type.clone()));
                continue;
            };

        let canonical = static_payload::canonical_iananame(name);

        if canonical.eq_ignore_ascii_case("telephone-event") {
            if !section
                .telephone_event_rates
                .contains(&clock_rate)
            {
                section
                    .telephone_event_rates
                    .push(clock_rate);
            }
            continue;
        }

        if canonical.eq_ignore_ascii_case("CN") {
            section.has_comfort_noise = true;
            continue;
        }

        // Audio rtpmap with no channel field normalizes to mono.
        let channels = match (&media_type, rtpmap_channels) {
            (SdpMediaType::Audio, None) => Some(1),
            _ => rtpmap_channels,
        };

        let fmtp = fmtp_map
            .get(&pt)
            .cloned();

        // ptime/bitrate resolution — sequential overwrite; later steps beat earlier.

        // Step 1: resolved a=ptime (media-level overrides session-level).
        let mut ptime = media_ptime;

        // Step 2: per-codec default when no a=ptime is present at all.
        if ptime.is_none() {
            ptime = Some(default_ptime_ms(canonical));
        }

        // Step 4: bitrate from the static payload type table.
        let mut bitrate = static_payload::known_bitrate(pt);

        // Step 5: no fmtp and iLBC/iSAC override even an explicit a=ptime.
        if fmtp.is_none() {
            if canonical.eq_ignore_ascii_case("ilbc") {
                ptime = Some(30);
                bitrate = Some(13330);
            } else if canonical.eq_ignore_ascii_case("isac") {
                ptime = Some(30);
                bitrate = Some(32000);
            }
        }

        // Step 6: fmtp present — apply codec-specific parameter parsers.
        if let Some(ref fmtp_str) = fmtp {
            if canonical.eq_ignore_ascii_case("opus") {
                if let Some(p) = fmtp_param(fmtp_str, "ptime") {
                    if let Some(v) = parse_ptime_value(p, &mut section.warnings, "fmtp ptime") {
                        ptime = Some(v);
                    }
                }
            } else if canonical.eq_ignore_ascii_case("ilbc") {
                // mode= sets ptime; fmtp present but no mode= means 30 ms.
                ptime = Some(match fmtp_param(fmtp_str, "mode") {
                    Some(m) => {
                        parse_ptime_value(m, &mut section.warnings, "fmtp mode").unwrap_or(30)
                    }
                    None => 30,
                });
            } else if canonical.eq_ignore_ascii_case("g7221") {
                if let Some(br_str) = fmtp_param(fmtp_str, "bitrate") {
                    match br_str.parse::<u32>() {
                        Ok(br) => bitrate = Some(br),
                        Err(_) => {
                            section
                                .warnings
                                .push(SdpWarning::unparseable_numeric_attribute(
                                    "g7221 fmtp bitrate",
                                    br_str,
                                ));
                        }
                    }
                }
            }
        }

        section
            .entries
            .push(SdpCodecEntry::Rtp(SdpCodec::new(
                media_type.clone(),
                pt,
                canonical,
                clock_rate,
                channels,
                fmtp,
                ptime,
                media_maxptime,
                bitrate,
                media_direction,
                has_rtpmap,
            )));
    }

    Ok(section)
}

/// Parse an `a=rtpmap` attribute value into `(pt, name, clock_rate, channels)`.
///
/// Returns `Err(MalformedRtpmap)` for any structural violation (missing space,
/// non-numeric PT, non-numeric clock rate, non-numeric channel count).
fn parse_rtpmap(value: &str) -> Result<(u8, String, u32, Option<u8>), SdpCodecError> {
    let (pt_str, rest) = value
        .split_once(' ')
        .ok_or_else(|| SdpCodecError::MalformedRtpmap(value.to_string()))?;

    let pt = pt_str
        .trim()
        .parse::<u8>()
        .map_err(|_| SdpCodecError::MalformedRtpmap(value.to_string()))?;

    let mut parts = rest.splitn(3, '/');

    let name = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| SdpCodecError::MalformedRtpmap(value.to_string()))?
        .to_string();

    let rate_str = parts
        .next()
        .ok_or_else(|| SdpCodecError::MalformedRtpmap(value.to_string()))?;

    let clock_rate = rate_str
        .trim()
        .parse::<u32>()
        .map_err(|_| SdpCodecError::MalformedRtpmap(value.to_string()))?;

    let channels = match parts.next() {
        Some(s) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                Some(
                    s.parse::<u8>()
                        .map_err(|_| SdpCodecError::MalformedRtpmap(value.to_string()))?,
                )
            }
        }
        None => None,
    };

    Ok((pt, name, clock_rate, channels))
}

/// Extract `(pt, params)` from an `a=fmtp` attribute value.
///
/// A non-numeric payload type is a hard error — same structural breakage as in `a=rtpmap`.
fn parse_fmtp_pt(value: &str) -> Result<(u8, String), SdpCodecError> {
    let (pt_str, params) = match value.split_once(' ') {
        Some((p, rest)) => (p, rest.to_string()),
        None => (value, String::new()),
    };
    let pt = pt_str
        .trim()
        .parse::<u8>()
        .map_err(|_| SdpCodecError::MalformedFmtp(value.to_string()))?;
    Ok((pt, params))
}

/// Return ptime from the first matching attribute, recording a warning if unparseable.
fn ptime_from_attrs(
    attrs: &[sdp_types::Attribute],
    name: &str,
    warnings: &mut Vec<SdpWarning>,
) -> Option<u32> {
    attrs
        .iter()
        .find(|a| a.attribute == name)
        .and_then(|a| {
            a.value
                .as_deref()
        })
        .and_then(|v| parse_ptime_value(v, warnings, name))
}

/// Parse a ptime/mode/interval string with FreeSWITCH-compatible tolerance.
///
/// Accepts an integer or a decimal with trailing fraction (`20.0` → 20).
/// Zero and junk values are treated as unset and a warning is recorded.
fn parse_ptime_value(raw: &str, warnings: &mut Vec<SdpWarning>, attr: &str) -> Option<u32> {
    // Strip trailing fractional part — upstream uses atoi, which does the same.
    let int_str = if let Some((prefix, _)) = raw.split_once('.') {
        prefix
    } else {
        raw
    };
    match int_str.parse::<u32>() {
        Ok(0) | Err(_) => {
            warnings.push(SdpWarning::unparseable_numeric_attribute(attr, raw));
            None
        }
        Ok(n) => Some(n),
    }
}

/// Return the first direction attribute from the list, if any.
fn direction_from_attrs(attrs: &[sdp_types::Attribute]) -> Option<SdpDirection> {
    attrs
        .iter()
        .find_map(|a| {
            // Direction attributes have no value (e.g. `a=sendrecv`, not `a=sendrecv:...`).
            if a.value
                .is_none()
            {
                a.attribute
                    .parse::<SdpDirection>()
                    .ok()
            } else {
                None
            }
        })
}

/// Per-codec default ptime in milliseconds when no `a=ptime` is present.
///
/// Delegates to `static_payload::default_ptime` — one table, two call sites.
/// Applied conditionally here (only when no `a=ptime` is present in the SDP);
/// the public `default_ptime` function is unconditional.
fn default_ptime_ms(canonical_name: &str) -> u32 {
    super::static_payload::default_ptime(canonical_name)
}

/// Extract a named parameter value from a semicolon-delimited fmtp string.
fn fmtp_param<'a>(fmtp: &'a str, key: &str) -> Option<&'a str> {
    fmtp.split(';')
        .find_map(|part| {
            let part = part.trim();
            let (k, v) = part.split_once('=')?;
            if k.trim()
                .eq_ignore_ascii_case(key)
            {
                Some(v.trim())
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- helpers ---

    fn sdp_header() -> String {
        "v=0\r\no=- 0 0 IN IP4 192.0.2.1\r\ns=-\r\nt=0 0\r\n".to_string()
    }

    fn rtp_codec(entries: &[SdpCodecEntry]) -> Vec<&SdpCodec> {
        entries
            .iter()
            .filter_map(|e| {
                if let SdpCodecEntry::Rtp(c) = e {
                    Some(c)
                } else {
                    None
                }
            })
            .collect()
    }

    fn codec_named<'a>(entries: &[&'a SdpCodec], name: &str) -> Option<&'a SdpCodec> {
        entries
            .iter()
            .find(|c| {
                c.name()
                    .eq_ignore_ascii_case(name)
            })
            .copied()
    }

    // --- static payload table ---

    #[test]
    fn static_only_pcmu_pcma_g729_no_rtpmap() {
        let sdp = format!("{}m=audio 5004 RTP/AVP 0 8 18\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let entries = codecs.entries();
        let rtp = rtp_codec(entries);
        assert_eq!(rtp.len(), 3);
        assert!(codec_named(&rtp, "PCMU").is_some());
        assert!(codec_named(&rtp, "PCMA").is_some());
        assert!(codec_named(&rtp, "G729").is_some());
        for c in &rtp {
            assert!(!c.has_rtpmap(), "static types must have has_rtpmap=false");
        }
    }

    // --- RFC 3551 quirk ---

    #[test]
    fn g722_rtpmap_clock_rate_is_8000() {
        // RFC 3551 quirk: G.722 is advertised at 8000 Hz in SDP even though it
        // runs at 16 kHz. The clock rate from a=rtpmap must be preserved as-is.
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 9\r\na=rtpmap:9 G722/8000\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let rtp = rtp_codec(codecs.entries());
        let g722 = codec_named(&rtp, "G722").expect("G722 must be present");
        assert_eq!(g722.clock_rate(), 8000);
        assert!(g722.has_rtpmap());
    }

    // --- opus stereo ---

    #[test]
    fn opus_stereo_channels_2() {
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 111\r\na=rtpmap:111 opus/48000/2\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let rtp = rtp_codec(codecs.entries());
        let opus = codec_named(&rtp, "opus").expect("opus must be present");
        assert_eq!(opus.channels(), Some(2));
        assert_eq!(opus.clock_rate(), 48000);
    }

    // --- AMR-WB channel count normalization ---

    #[test]
    fn amrwb_no_channel_field_equals_explicit_one() {
        // a=rtpmap without a channel field and a=rtpmap with /1 must yield the same
        // channel count — both normalize to Some(1) for audio sections.
        let sdp_no_ch = format!(
            "{}m=audio 5004 RTP/AVP 100\r\na=rtpmap:100 AMR-WB/16000\r\n",
            sdp_header()
        );
        let sdp_ch1 = format!(
            "{}m=audio 5004 RTP/AVP 100\r\na=rtpmap:100 AMR-WB/16000/1\r\n",
            sdp_header()
        );
        let c_no_ch = SdpCodecs::parse(&sdp_no_ch).unwrap();
        let c_ch1 = SdpCodecs::parse(&sdp_ch1).unwrap();
        let rtp_no_ch = rtp_codec(c_no_ch.entries());
        let rtp_ch1 = rtp_codec(c_ch1.entries());
        let amrwb_no_ch = codec_named(&rtp_no_ch, "AMR-WB").expect("AMR-WB must be present");
        let amrwb_ch1 = codec_named(&rtp_ch1, "AMR-WB").expect("AMR-WB must be present");
        assert_eq!(
            amrwb_no_ch.channels(),
            amrwb_ch1.channels(),
            "missing /1 and explicit /1 must normalize to the same channel count"
        );
        assert_eq!(amrwb_ch1.channels(), Some(1));
    }

    // --- ptime precedence ---

    #[test]
    fn ptime_opus_fmtp_overrides_session_ptime() {
        // Sequential overwrite: a=ptime:20 sets ptime=20, then opus fmtp ptime=40
        // overrides it. The final result must be 40.
        let sdp = format!(
            "{}a=ptime:20\r\nm=audio 5004 RTP/AVP 111\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 ptime=40\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let rtp = rtp_codec(codecs.entries());
        let opus = codec_named(&rtp, "opus").expect("opus must be present");
        assert_eq!(opus.ptime(), Some(40), "fmtp ptime= must override a=ptime");
    }

    #[test]
    fn ptime_ilbc_no_fmtp_overrides_explicit_ptime() {
        // iLBC without fmtp overrides even an explicit a=ptime (step 5 sequential overwrite).
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 97\r\na=rtpmap:97 iLBC/8000\r\na=ptime:20\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let rtp = rtp_codec(codecs.entries());
        let ilbc = codec_named(&rtp, "iLBC").expect("iLBC must be present");
        assert_eq!(
            ilbc.ptime(),
            Some(30),
            "iLBC without fmtp must always yield ptime=30"
        );
    }

    #[test]
    fn ptime_ilbc_fmtp_mode20_yields_20_no_bitrate() {
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 97\r\na=rtpmap:97 iLBC/8000\r\na=fmtp:97 mode=20\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let rtp = rtp_codec(codecs.entries());
        let ilbc = codec_named(&rtp, "iLBC").expect("iLBC must be present");
        assert_eq!(ilbc.ptime(), Some(20));
        assert_eq!(
            ilbc.bitrate(),
            None,
            "fmtp present — bitrate not set by static table"
        );
    }

    // --- media-level vs session-level ptime ---

    #[test]
    fn media_ptime_overrides_session_ptime() {
        let sdp = format!(
            "{}a=ptime:10\r\nm=audio 5004 RTP/AVP 0\r\na=ptime:30\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let rtp = rtp_codec(codecs.entries());
        let pcmu = codec_named(&rtp, "PCMU").expect("PCMU must be present");
        assert_eq!(
            pcmu.ptime(),
            Some(30),
            "media-level a=ptime must override session-level"
        );
    }

    #[test]
    fn session_ptime_applies_when_media_has_none() {
        let sdp = format!("{}a=ptime:10\r\nm=audio 5004 RTP/AVP 8\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let rtp = rtp_codec(codecs.entries());
        let pcma = codec_named(&rtp, "PCMA").expect("PCMA must be present");
        assert_eq!(
            pcma.ptime(),
            Some(10),
            "session-level a=ptime must apply when media has none"
        );
    }

    // --- ptime numeric tolerance ---

    #[test]
    fn ptime_fractional_value_is_truncated() {
        let sdp = format!("{}m=audio 5004 RTP/AVP 0\r\na=ptime:20.0\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(codecs
            .warnings()
            .is_empty());
        let rtp = rtp_codec(codecs.entries());
        let pcmu = codec_named(&rtp, "PCMU").expect("PCMU must be present");
        assert_eq!(pcmu.ptime(), Some(20));
    }

    #[test]
    fn ptime_junk_is_unset_with_warning() {
        let sdp = format!("{}m=audio 5004 RTP/AVP 0\r\na=ptime:abc\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(
            codecs
                .warnings()
                .len(),
            1,
            "junk ptime must produce exactly one warning"
        );
        let rtp = rtp_codec(codecs.entries());
        let pcmu = codec_named(&rtp, "PCMU").expect("PCMU must be present");
        // Falls back to the codec default (20 ms for PCMU)
        assert_eq!(pcmu.ptime(), Some(20));
    }

    #[test]
    fn ptime_zero_is_unset_with_warning() {
        let sdp = format!("{}m=audio 5004 RTP/AVP 0\r\na=ptime:0\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(
            codecs
                .warnings()
                .len(),
            1,
            "ptime=0 must produce exactly one warning"
        );
        let rtp = rtp_codec(codecs.entries());
        let pcmu = codec_named(&rtp, "PCMU").expect("PCMU must be present");
        assert_eq!(pcmu.ptime(), Some(20));
    }

    // --- port 0 skip ---

    #[test]
    fn port_zero_m_line_is_skipped() {
        let sdp = format!("{}m=audio 0 RTP/AVP 0\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(
            codecs.is_empty(),
            "m-line with port 0 must be skipped entirely"
        );
        assert!(codecs
            .unmapped()
            .is_empty());
    }

    // --- IPv6 connection ---

    #[test]
    fn ipv6_connection_parses_without_error() {
        let sdp = format!(
            "{}c=IN IP6 2001:db8::1\r\nm=audio 5004 RTP/AVP 0\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(codecs.len(), 1);
    }

    // --- T38 ---

    #[test]
    fn image_m_line_yields_t38_entry() {
        let sdp = format!("{}m=image 5008 udptl t38\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(codecs.len(), 1);
        assert!(matches!(codecs.entries()[0], SdpCodecEntry::T38));
    }

    #[test]
    fn image_m_line_with_other_proto_still_yields_t38() {
        // The proto and fmt are not inspected; FreeSWITCH does not check them.
        let sdp = format!(
            "{}m=image 5008 RTP/AVP 98\r\na=rtpmap:98 t38/8000\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(codecs.len(), 1);
        assert!(matches!(codecs.entries()[0], SdpCodecEntry::T38));
    }

    #[test]
    fn t38_before_image_precedes_audio_in_codec_string() {
        // C writes ",t38" inside the m-line loop (switch_core_media.c:13651), so
        // position in the offer's m-line order determines position in the codec
        // string, same as any other entry.
        let sdp = format!(
            concat!(
                "{}",
                "m=image 5008 udptl t38\r\n",
                "m=audio 5004 RTP/AVP 0\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let cs = codecs
            .audio_codec_string(&CodecStringOptions::default(), None)
            .unwrap();
        assert_eq!(cs.entries()[0].name(), "t38");
        assert_eq!(cs.entries()[1].name(), "PCMU");
    }

    // --- unknown media type ---

    #[test]
    fn application_m_line_does_not_fail() {
        let sdp = format!(
            "{}m=application 9 UDP/BFCP *\r\nm=audio 5004 RTP/AVP 0\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        // application section contributes no codecs
        assert_eq!(codecs.len(), 1);
        let rtp = rtp_codec(codecs.entries());
        assert!(codec_named(&rtp, "PCMU").is_some());
    }

    // --- telephone-event and CN ---

    #[test]
    fn telephone_event_in_rates_not_in_entries() {
        // telephone-event at two clock rates; neither appears as an entry or unmapped.
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 0 101 102\r\n",
                "a=rtpmap:101 telephone-event/8000\r\n",
                "a=rtpmap:102 telephone-event/16000\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let rates = codecs.telephone_event_rates();
        assert!(
            rates.contains(&8000),
            "8000 Hz DTMF must be in telephone_event_rates"
        );
        assert!(
            rates.contains(&16000),
            "16000 Hz DTMF must be in telephone_event_rates"
        );
        // Must not appear as codec entries
        let rtp = rtp_codec(codecs.entries());
        assert!(codec_named(&rtp, "telephone-event").is_none());
        // Must not be in unmapped
        assert!(codecs
            .unmapped()
            .is_empty());
    }

    #[test]
    fn cn_sets_has_comfort_noise() {
        let sdp = format!("{}m=audio 5004 RTP/AVP 0 13\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(
            codecs.has_comfort_noise(),
            "static PT 13 (CN) must set has_comfort_noise"
        );
        // CN must not appear as a codec entry
        let rtp = rtp_codec(codecs.entries());
        assert!(codec_named(&rtp, "CN").is_none());
        assert!(codecs
            .unmapped()
            .is_empty());
    }

    // --- unmapped payload types ---

    #[test]
    fn dynamic_pt_without_rtpmap_is_unmapped_not_error() {
        let sdp = format!("{}m=audio 5004 RTP/AVP 0 99\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(
            codecs
                .unmapped()
                .len(),
            1
        );
        assert_eq!(codecs.unmapped()[0].payload_type, 99);
        // PT 0 (PCMU) is still there; PT 99 is unmapped, not silently dropped
        let rtp = rtp_codec(codecs.entries());
        assert_eq!(rtp.len(), 1);
        assert!(codec_named(&rtp, "PCMU").is_some());
    }

    // --- malformed rtpmap skips only its own section ---

    #[test]
    fn malformed_rtpmap_non_numeric_pt_skips_section_with_warning() {
        // The section-level breakage (bad a=rtpmap) drops the whole section,
        // including the otherwise-fine static PCMU payload type it shares a
        // section with — there is no per-attribute recovery, only per-section.
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 0\r\na=rtpmap:x PCMU/8000\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(
            codecs
                .entries()
                .is_empty(),
            "the whole malformed section must be skipped, not just the bad attribute"
        );
        assert_eq!(
            codecs
                .warnings()
                .len(),
            1
        );
        assert!(matches!(
            codecs.warnings()[0],
            SdpWarning::MalformedMediaSection { .. }
        ));
    }

    // --- rtpmap whitespace is a tokenizer concern, not a per-field trim ---

    #[test]
    fn rtpmap_double_space_before_name_is_parsed() {
        // Two spaces between the payload type and the encoding name must not leak
        // into the name (FreeSWITCH/sofia's parse_ul skips any run of SP/HTAB).
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 111\r\na=rtpmap:111  opus/48000/2\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let rtp = rtp_codec(codecs.entries());
        let opus = codec_named(&rtp, "opus").expect("opus must be present, not \" opus\"");
        assert_eq!(opus.clock_rate(), 48000);
        assert_eq!(opus.channels(), Some(2));
    }

    #[test]
    fn rtpmap_tab_separator_is_parsed() {
        // A HTAB between payload type and encoding name is legal SDP whitespace;
        // it must not be treated as "no separator" (which used to discard the
        // whole m=audio section).
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 0\r\na=rtpmap:0\tPCMU/8000\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let rtp = rtp_codec(codecs.entries());
        let pcmu = codec_named(&rtp, "PCMU").expect("PCMU must be present");
        assert_eq!(pcmu.clock_rate(), 8000);
        assert_eq!(
            pcmu.channels(),
            Some(1),
            "no channel field normalizes to mono"
        );
    }

    #[test]
    fn rtpmap_interior_whitespace_in_name_is_malformed() {
        // Whitespace inside what would be the encoding name is not a separator
        // sofia recognises either — it still rejects this shape, via a failed
        // clock-rate parse once the name is truncated at the first space.
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 0\r\na=rtpmap:0 foo bar/8000\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(
            codecs
                .entries()
                .is_empty(),
            "malformed rtpmap must still drop the whole section"
        );
        assert!(matches!(
            codecs.warnings()[0],
            SdpWarning::MalformedMediaSection { .. }
        ));
    }

    // --- malformed fmtp skips only its own section ---

    #[test]
    fn malformed_fmtp_non_numeric_pt_skips_section_with_warning() {
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 0\r\na=fmtp:x minptime=10\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(codecs
            .entries()
            .is_empty());
        assert_eq!(
            codecs
                .warnings()
                .len(),
            1
        );
        assert!(matches!(
            codecs.warnings()[0],
            SdpWarning::MalformedMediaSection { .. }
        ));
    }

    // --- G7221 malformed bitrate records a warning ---

    #[test]
    fn g7221_malformed_bitrate_records_warning() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 100\r\n",
                "a=rtpmap:100 G7221/16000\r\n",
                "a=fmtp:100 bitrate=abc\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(
            codecs
                .warnings()
                .len(),
            1,
            "malformed g7221 bitrate must produce exactly one warning"
        );
        let rtp = rtp_codec(codecs.entries());
        let g7221 = codec_named(&rtp, "G7221").expect("G7221 must be present");
        assert_eq!(
            g7221.bitrate(),
            None,
            "bitrate must be None after parse error"
        );
    }

    #[test]
    fn g7221_valid_bitrate_is_set() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 100\r\n",
                "a=rtpmap:100 G7221/16000\r\n",
                "a=fmtp:100 bitrate=24000\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(codecs
            .warnings()
            .is_empty());
        let rtp = rtp_codec(codecs.entries());
        let g7221 = codec_named(&rtp, "G7221").expect("G7221 must be present");
        assert_eq!(g7221.bitrate(), Some(24000));
    }

    // --- Step 7: audio_codec_string / video_codec_string / fmtp_for ---

    #[test]
    fn amr_oa_be_pair_collapses_after_dedup() {
        // Both AMR/8000 registrations differ only in fmtp; default options don't emit
        // fmtp, so the two entries are identical and only dedup() tells them apart.
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 96 97\r\n",
                "a=rtpmap:96 AMR/8000\r\n",
                "a=fmtp:96 octet-align=1\r\n",
                "a=rtpmap:97 AMR/8000\r\n",
                "a=fmtp:97 octet-align=0\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let cs = codecs
            .audio_codec_string(&CodecStringOptions::default(), None)
            .unwrap();
        assert_eq!(cs.len(), 2, "both AMR entries must be emitted");
        assert_eq!(
            cs.entries()[0],
            cs.entries()[1],
            "entries must be identical under default options (no fmtp)"
        );
        let mut deduped = cs.clone();
        deduped.dedup();
        assert_eq!(
            deduped.len(),
            1,
            "duplicate AMR entries must collapse under dedup()"
        );
    }

    #[test]
    fn fmtp_for_falls_through_to_later_codec_with_fmtp() {
        // Matches the AMR octet-aligned / bandwidth-efficient pair's shape: the
        // first payload matching name+rate has no fmtp, the second does. The
        // second's fmtp must not be shadowed by the first's absence.
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 97 96\r\n",
                "a=rtpmap:97 AMR/8000\r\n",
                "a=rtpmap:96 AMR/8000\r\n",
                "a=fmtp:96 octet-align=1\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(
            codecs.fmtp_for("AMR", 8000),
            Some("octet-align=1"),
            "fmtp_for must fall through a fmtp-less match to a later one that has fmtp"
        );
    }

    #[test]
    fn malformed_media_section_is_skipped_not_fatal() {
        // A non-numeric payload type in the m= format list is structurally broken
        // (same class as a malformed a=rtpmap/a=fmtp): parse_entry can't recover a
        // pt to key rtpmap/fmtp lookups on. One bad section must not discard every
        // other section's codecs.
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 9 RTP/AVP *\r\n",
                "m=audio 5004 RTP/AVP 0 8\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let rtp = rtp_codec(codecs.entries());
        assert!(codec_named(&rtp, "PCMU").is_some());
        assert!(codec_named(&rtp, "PCMA").is_some());
        assert_eq!(
            codecs
                .warnings()
                .len(),
            1,
            "the malformed section must record exactly one warning"
        );
        assert!(matches!(
            codecs.warnings()[0],
            SdpWarning::MalformedMediaSection { .. }
        ));
    }

    #[test]
    fn amrwb_absent_and_explicit_channel_collapse_after_dedup() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 100 101\r\n",
                "a=rtpmap:100 AMR-WB/16000\r\n",
                "a=rtpmap:101 AMR-WB/16000/1\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let cs = codecs
            .audio_codec_string(&CodecStringOptions::default(), None)
            .unwrap();
        assert_eq!(cs.len(), 2);
        let mut deduped = cs.clone();
        deduped.dedup();
        assert_eq!(
            deduped.len(),
            1,
            "absent and explicit /1 channel count must normalize identically"
        );
    }

    #[test]
    fn evs_dotted_fmtp_unreachable_under_default_options() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 110\r\n",
                "a=rtpmap:110 EVS/16000\r\n",
                "a=fmtp:110 br=13.2-24.4;bw=nb-swb\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let cs = codecs
            .audio_codec_string(&CodecStringOptions::default(), None)
            .unwrap();
        assert_eq!(cs.len(), 1);
        assert!(
            cs.entries()[0]
                .fmtp()
                .is_none(),
            "fmtp emission is off by default; the dotted fmtp path is unreachable"
        );
    }

    #[test]
    fn evs_dotted_fmtp_lenient_clears_and_warns() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 110\r\n",
                "a=rtpmap:110 EVS/16000\r\n",
                "a=fmtp:110 br=13.2-24.4;bw=nb-swb\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let opts = CodecStringOptions::default().with_fmtp(true);
        let mut warnings = Vec::new();
        let cs = codecs
            .audio_codec_string(&opts, Some(&mut warnings))
            .unwrap();
        assert_eq!(cs.len(), 1, "the codec must still be emitted");
        assert!(
            cs.entries()[0]
                .fmtp()
                .is_none(),
            "unrepresentable fmtp must be cleared, not left dangling"
        );
        assert_eq!(warnings.len(), 1);
        assert!(matches!(
            warnings[0],
            SdpWarning::FmtpUnrepresentable { .. }
        ));
    }

    #[test]
    fn evs_dotted_fmtp_strict_is_err() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 110\r\n",
                "a=rtpmap:110 EVS/16000\r\n",
                "a=fmtp:110 br=13.2-24.4;bw=nb-swb\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let opts = CodecStringOptions::default().with_fmtp(true);
        let result = codecs.audio_codec_string(&opts, None);
        assert!(
            result.is_err(),
            "strict mode must fail on an unrepresentable fmtp"
        );
    }

    #[test]
    fn g722_rate_is_8000_never_16000() {
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 9\r\na=rtpmap:9 G722/8000\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let cs = codecs
            .audio_codec_string(&CodecStringOptions::default(), None)
            .unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs.entries()[0].name(), "G722");
        assert_eq!(cs.entries()[0].rate(), Some(8000));
        assert_eq!(cs.entries()[0].ptime(), Some(20));
    }

    #[test]
    fn video_codec_string_carries_no_rate_qualifier() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 0\r\n",
                "m=video 5006 RTP/AVP 96\r\n",
                "a=rtpmap:96 H264/90000\r\n",
                "m=image 5008 udptl t38\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();

        let video_cs = codecs
            .video_codec_string(&CodecStringOptions::video(), None)
            .unwrap();
        assert_eq!(video_cs.len(), 1);
        assert_eq!(video_cs.entries()[0].name(), "H264");
        assert!(
            video_cs.entries()[0]
                .rate()
                .is_none(),
            "CodecStringOptions::video() must not emit a rate qualifier"
        );

        let audio_cs = codecs
            .audio_codec_string(&CodecStringOptions::default(), None)
            .unwrap();
        assert!(
            audio_cs.contains_name("t38"),
            "an offered m=image section must append a literal t38 entry to the audio string"
        );
    }

    #[test]
    fn codec_name_with_delimiter_is_skipped_lenient_and_erred_strict() {
        // A comma in the a=rtpmap encoding name cannot be represented as a codec-string
        // entry at all — distinct from UnmappedPayload (no rtpmap name whatsoever).
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 100\r\na=rtpmap:100 bad,name/8000\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();

        let mut warnings = Vec::new();
        let cs = codecs
            .audio_codec_string(&CodecStringOptions::default(), Some(&mut warnings))
            .unwrap();
        assert!(cs.is_empty(), "the unrepresentable codec must be skipped");
        assert_eq!(warnings.len(), 1);
        assert!(matches!(
            warnings[0],
            SdpWarning::CodecNameUnrepresentable { .. }
        ));

        let result = codecs.audio_codec_string(&CodecStringOptions::default(), None);
        assert!(result.is_err(), "strict mode must fail");
    }

    // --- fmtp_for ---

    #[test]
    fn fmtp_for_looks_up_by_name_and_rate_case_insensitively() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 100\r\n",
                "a=rtpmap:100 AMR-WB/16000\r\n",
                "a=fmtp:100 octet-align=1\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(
            codecs.fmtp_for("amr-wb", 16000),
            Some("octet-align=1"),
            "lookup must be case-insensitive on the codec name"
        );
        assert_eq!(
            codecs.fmtp_for("AMR-WB", 8000),
            None,
            "clock rate must also match"
        );
        assert_eq!(
            codecs.fmtp_for("PCMU", 8000),
            None,
            "codec not in the offer"
        );
    }
}
