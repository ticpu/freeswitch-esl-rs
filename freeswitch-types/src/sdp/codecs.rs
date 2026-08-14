//! Collection of codecs parsed from a complete SDP session description.

use std::collections::HashMap;

use crate::sdp::{
    codec::{NonCodecKind, NonCodecPayload, SdpCodec, SdpDirection, SdpMediaType},
    codec_string::{CodecString, CodecStringEntry},
    error::{CodecStringError, SdpCodecError, SdpWarning, UnmappedPayload},
    options::CodecStringOptions,
    static_payload,
};

/// A single entry in a parsed SDP offer.
///
/// Payloads negotiated outside the codec string are excluded from entries and
/// retained whole by [`SdpMediaSection::non_codec_payloads`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SdpCodecEntry {
    /// An RTP codec negotiated via `a=rtpmap` or the RFC 3551 static table.
    Rtp(SdpCodec),
    /// T.38 fax relay, derived from any `m=image` section.
    ///
    /// The proto and fmt fields are not inspected — FreeSWITCH negotiates T.38
    /// parameters independently of the codec string. Whether the section reaches
    /// a codec string is [`SdpMediaSection::is_negotiable`].
    T38,
}

/// One `m=` section of an SDP offer, whatever its port or media type.
///
/// Sections that contribute nothing to a codec string are retained all the same:
/// a port-0 section is the offer's held or declined stream and is exactly what a
/// reader needs when a call has no audio. [`is_negotiable`](Self::is_negotiable)
/// separates the two.
#[derive(Debug, Clone)]
pub struct SdpMediaSection {
    media_type: SdpMediaType,
    port: u16,
    proto: String,
    formats: String,
    direction: SdpDirection,
    entries: Vec<SdpCodecEntry>,
    unmapped: Vec<UnmappedPayload>,
    non_codec: Vec<NonCodecPayload>,
}

impl SdpMediaSection {
    /// The media type from the `m=` line.
    pub fn media_type(&self) -> &SdpMediaType {
        &self.media_type
    }

    /// The transport port; `0` means the peer declined or held this stream.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The transport protocol from the `m=` line, verbatim.
    pub fn proto(&self) -> &str {
        &self.proto
    }

    /// The `m=` format list, verbatim — the only content a non-RTP section carries.
    pub fn formats(&self) -> &str {
        &self.formats
    }

    /// The direction the peer wrote, defaulting to the session's.
    ///
    /// Sofia forces a port-0 section to inactive regardless, so on a held section
    /// this is what was offered, not what the switch acted on.
    pub fn direction(&self) -> SdpDirection {
        self.direction
    }

    /// Codec entries in `m=` format-list order.
    pub fn entries(&self) -> &[SdpCodecEntry] {
        &self.entries
    }

    /// Payload types this section named that could not be resolved to a codec.
    pub fn unmapped(&self) -> &[UnmappedPayload] {
        &self.unmapped
    }

    /// Payloads the switch negotiates outside the codec string, in format-list order.
    ///
    /// Not deduplicated. See [`NonCodecPayload`] for what the switch itself retains.
    pub fn non_codec_payloads(&self) -> &[NonCodecPayload] {
        &self.non_codec
    }

    /// Whether this section feeds the codec string and the top-level views.
    pub fn is_negotiable(&self) -> bool {
        let derives_entries = matches!(self.media_type, SdpMediaType::Audio | SdpMediaType::Video)
            || is_image(&self.media_type);
        self.port != 0 && derives_entries
    }
}

/// Codecs and ancillary data extracted from an SDP session description.
///
/// Produced by [`SdpCodecs::parse`]. The accessors that feed a codec string reflect
/// the same extraction logic as FreeSWITCH's
/// `switch_core_media_set_r_sdp_codec_string`; [`sections`](Self::sections) is the
/// wider view, every `m=` line the offer carried. See `docs/codec-string-format.md`
/// for the mapping rules.
#[derive(Debug)]
pub struct SdpCodecs {
    sections: Vec<SdpMediaSection>,
    warnings: Vec<SdpWarning>,
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
            sections: Vec::new(),
            warnings: Vec::new(),
        };

        // Session-level defaults; media-level attributes override these per section.
        let session_ptime = ptime_from_attrs(&session.attributes, "ptime", &mut result.warnings);
        let session_maxptime =
            ptime_from_attrs(&session.attributes, "maxptime", &mut result.warnings);
        let session_direction =
            direction_from_attrs(&session.attributes).unwrap_or(SdpDirection::SendRecv);

        for media in &session.medias {
            // SdpMediaType::from_str is infallible (Err = std::convert::Infallible).
            let media_type: SdpMediaType = match media
                .media
                .parse()
            {
                Ok(t) => t,
                Err(e) => match e {},
            };

            let mut section = SdpMediaSection {
                media_type: media_type.clone(),
                port: media.port,
                proto: media
                    .proto
                    .clone(),
                formats: media
                    .fmt
                    .clone(),
                direction: direction_from_attrs(&media.attributes).unwrap_or(session_direction),
                entries: Vec::new(),
                unmapped: Vec::new(),
                non_codec: Vec::new(),
            };

            if is_image(&media_type) {
                // The proto and fmt are not inspected; the section is the T.38 stream.
                section
                    .entries
                    .push(SdpCodecEntry::T38);
            } else if proto_has_rtp(&media.proto) {
                // A single structurally broken section (bad rtpmap/fmtp/payload type)
                // must not discard every other section already parsed. Stage this
                // section's output locally and only merge it in on success.
                match parse_media_section(
                    media,
                    media_type.clone(),
                    session_ptime,
                    session_maxptime,
                    section.direction,
                ) {
                    Ok(parsed) => {
                        section.entries = parsed.entries;
                        section.unmapped = parsed.unmapped;
                        section.non_codec = parsed.non_codec;
                        result
                            .warnings
                            .extend(parsed.warnings);
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

            result
                .sections
                .push(section);
        }

        Ok(result)
    }

    /// Every `m=` section the offer carried, in offer order.
    ///
    /// This is the inventory, including the sections no codec string can reach — a
    /// held or declined stream, or a media type the switch derives no codec from.
    pub fn sections(&self) -> &[SdpMediaSection] {
        &self.sections
    }

    fn negotiable(&self) -> impl Iterator<Item = &SdpMediaSection> {
        self.sections
            .iter()
            .filter(|s| s.is_negotiable())
    }

    /// Codec entries from the negotiable sections, in SDP offer order.
    pub fn entries(&self) -> impl Iterator<Item = &SdpCodecEntry> {
        self.negotiable()
            .flat_map(|s| {
                s.entries
                    .iter()
            })
    }

    /// Consume this collection, returning the negotiable sections' entries.
    pub fn into_entries(self) -> Vec<SdpCodecEntry> {
        self.sections
            .into_iter()
            .filter(|s| s.is_negotiable())
            .flat_map(|s| s.entries)
            .collect()
    }

    /// Number of codec entries; payloads from [`non_codec_payloads`](Self::non_codec_payloads)
    /// are not counted.
    pub fn len(&self) -> usize {
        self.entries()
            .count()
    }

    /// `true` when the offer negotiates no codecs, ignoring any non-codec payloads.
    pub fn is_empty(&self) -> bool {
        self.entries()
            .next()
            .is_none()
    }

    /// Iterator over audio RTP codecs only.
    pub fn audio(&self) -> impl Iterator<Item = &SdpCodec> {
        self.rtp_of(SdpMediaType::Audio)
    }

    /// Iterator over video RTP codecs only.
    pub fn video(&self) -> impl Iterator<Item = &SdpCodec> {
        self.rtp_of(SdpMediaType::Video)
    }

    fn rtp_of(&self, media: SdpMediaType) -> impl Iterator<Item = &SdpCodec> {
        self.entries()
            .filter_map(move |e| match e {
                SdpCodecEntry::Rtp(c) if c.media() == &media => Some(c),
                _ => None,
            })
    }

    /// Payload types that could not be named (no `a=rtpmap`, no static-table entry).
    ///
    /// These are surfaced as data rather than an error so the caller can distinguish
    /// "never offered" from "offered but unresolvable". A non-negotiable section's
    /// unresolvable payloads stay on [`SdpMediaSection::unmapped`].
    pub fn unmapped(&self) -> impl Iterator<Item = &UnmappedPayload> {
        self.negotiable()
            .flat_map(|s| {
                s.unmapped
                    .iter()
            })
    }

    /// Recoverable parse warnings (e.g. unparseable numeric attributes).
    ///
    /// Covers every section, negotiable or not — the excluded ones are parsed rather
    /// than skipped, so they warn where the switch has no occasion to.
    pub fn warnings(&self) -> &[SdpWarning] {
        &self.warnings
    }

    /// Payloads the switch negotiates outside the codec string, in `m=` order.
    ///
    /// Never included in [`entries`](Self::entries) or [`unmapped`](Self::unmapped),
    /// and drawn from the negotiable sections only; a held section's are on
    /// [`SdpMediaSection::non_codec_payloads`]. Not deduplicated: two sections offering
    /// the same kind at one clock rate under different payload types yield two entries.
    /// See [`NonCodecPayload`] for what the switch itself retains from these.
    pub fn non_codec_payloads(&self) -> impl Iterator<Item = &NonCodecPayload> {
        self.negotiable()
            .flat_map(|s| {
                s.non_codec
                    .iter()
            })
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
        for entry in self.entries() {
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

/// One `m=` section's payload walk, staged locally so a mid-section parse
/// failure can be discarded wholesale instead of reaching [`SdpMediaSection`].
#[derive(Default)]
struct MediaSection {
    entries: Vec<SdpCodecEntry>,
    unmapped: Vec<UnmappedPayload>,
    warnings: Vec<SdpWarning>,
    non_codec: Vec<NonCodecPayload>,
}

/// `m=image`, matched case-insensitively as FreeSWITCH's own comparison is.
fn is_image(media_type: &SdpMediaType) -> bool {
    matches!(media_type, SdpMediaType::Other(s) if s.eq_ignore_ascii_case("image"))
}

/// Whether sofia would read this section's format list as RTP payload types.
///
/// `sdp_media_transport` matches the proto against an exact set, so a name merely
/// containing `RTP` is not one; `sdp_media_has_rtp` then decides whether the
/// format list, `a=rtpmap` and `a=fmtp` are read into the section's rtpmap list at
/// all. The bare `RTP` is sofia's non-strict spelling of `RTP/AVP`.
fn proto_has_rtp(proto: &str) -> bool {
    const RTP_PROTOS: [&str; 8] = [
        "RTP",
        "RTP/AVP",
        "RTP/AVPF",
        "RTP/SAVP",
        "RTP/SAVPF",
        "UDP/RTP/AVPF",
        "UDP/TLS/RTP/SAVP",
        "UDP/TLS/RTP/SAVPF",
    ];
    RTP_PROTOS
        .iter()
        .any(|p| p.eq_ignore_ascii_case(proto))
}

/// Classify an encoding name the switch negotiates outside the codec string.
///
/// Matched on the canonical name the same way FreeSWITCH matches it, by
/// case-insensitive comparison against `rm_encoding` (`switch_core_media.c:5805`,
/// `:5816`). No loadable codec carries either name.
fn non_codec_kind(canonical_name: &str) -> Option<NonCodecKind> {
    if canonical_name.eq_ignore_ascii_case("telephone-event") {
        Some(NonCodecKind::TelephoneEvent)
    } else if canonical_name.eq_ignore_ascii_case("CN") {
        Some(NonCodecKind::ComfortNoise)
    } else {
        None
    }
}

/// Walk one RTP `m=` section's format list into codec entries.
///
/// Returns `Err` only for structural breakage within this section (an
/// unparseable `a=rtpmap`, `a=fmtp`, or `m=` format-list payload type) — the
/// caller records this as [`SdpWarning::MalformedMediaSection`] and keeps the
/// section empty rather than aborting the whole parse.
fn parse_media_section(
    media: &sdp_types::Media,
    media_type: SdpMediaType,
    session_ptime: Option<u32>,
    session_maxptime: Option<u32>,
    media_direction: SdpDirection,
) -> Result<MediaSection, SdpCodecError> {
    let mut section = MediaSection::default();

    // Media-level attributes override session-level.
    let media_ptime =
        ptime_from_attrs(&media.attributes, "ptime", &mut section.warnings).or(session_ptime);
    let media_maxptime =
        ptime_from_attrs(&media.attributes, "maxptime", &mut section.warnings).or(session_maxptime);

    let tables = attribute_tables(media)?;

    for pt_str in media
        .fmt
        .split_whitespace()
    {
        let pt = pt_str
            .parse::<u8>()
            .map_err(|_| SdpCodecError::NonNumericPayloadType(pt_str.to_string()))?;

        // Name, clock rate, and channel count from rtpmap or RFC 3551 static table.
        let (name, clock_rate, rtpmap_channels, has_rtpmap) = if let Some((n, r, c)) = tables
            .rtpmap
            .get(&pt)
        {
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

        let fmtp = tables
            .fmtp
            .get(&pt)
            .cloned();

        // Leaves the walk here, where FreeSWITCH does (switch_core_media.c:5447/:5456),
        // ahead of the per-codec ptime and bitrate resolution below.
        if let Some(kind) = non_codec_kind(canonical) {
            section
                .non_codec
                .push(NonCodecPayload {
                    kind,
                    payload_type: pt,
                    media_type: media_type.clone(),
                    clock_rate,
                    fmtp,
                    has_rtpmap,
                });
            continue;
        }

        // Audio rtpmap with no channel field normalizes to mono.
        let channels = match (&media_type, rtpmap_channels) {
            (SdpMediaType::Audio, None) => Some(1),
            _ => rtpmap_channels,
        };

        let (ptime, bitrate) = resolve_ptime_bitrate(
            canonical,
            pt,
            fmtp.as_deref(),
            media_ptime,
            &mut section.warnings,
        );

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

/// One section's `a=rtpmap` and `a=fmtp` values, keyed by payload type.
struct AttrTables {
    rtpmap: HashMap<u8, (String, u32, Option<u8>)>,
    fmtp: HashMap<u8, String>,
}

/// Collect a section's rtpmap and fmtp attributes into payload-type lookup tables.
fn attribute_tables(media: &sdp_types::Media) -> Result<AttrTables, SdpCodecError> {
    let mut tables = AttrTables {
        rtpmap: HashMap::new(),
        fmtp: HashMap::new(),
    };

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
                    tables
                        .rtpmap
                        .insert(pt, (name, rate, channels));
                }
            }
            "fmtp" => {
                if let Some(val) = &attr.value {
                    let (pt, params) = parse_fmtp_pt(val)?;
                    tables
                        .fmtp
                        .insert(pt, params);
                }
            }
            _ => {}
        }
    }

    Ok(tables)
}

/// Resolve one payload's ptime and bitrate as `add_audio_codec` does: a sequential
/// overwrite where a later step beats an earlier one, not a first-match-wins chain.
fn resolve_ptime_bitrate(
    canonical: &str,
    payload_type: u8,
    fmtp: Option<&str>,
    media_ptime: Option<u32>,
    warnings: &mut Vec<SdpWarning>,
) -> (Option<u32>, Option<u32>) {
    // Step 1: resolved a=ptime (media-level overrides session-level).
    let mut ptime = media_ptime;

    // Step 2: per-codec default when no a=ptime is present at all.
    if ptime.is_none() {
        ptime = Some(default_ptime_ms(canonical));
    }

    // Step 4: bitrate from the static payload type table.
    let mut bitrate = static_payload::known_bitrate(payload_type);

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
    if let Some(fmtp_str) = fmtp {
        if canonical.eq_ignore_ascii_case("opus") {
            if let Some(p) = fmtp_param(fmtp_str, "ptime") {
                if let Some(v) = parse_ptime_value(p, warnings, "fmtp ptime") {
                    ptime = Some(v);
                }
            }
        } else if canonical.eq_ignore_ascii_case("ilbc") {
            // mode= sets ptime; fmtp present but no mode= means 30 ms.
            ptime = Some(match fmtp_param(fmtp_str, "mode") {
                Some(m) => parse_ptime_value(m, warnings, "fmtp mode").unwrap_or(30),
                None => 30,
            });
        } else if canonical.eq_ignore_ascii_case("g7221") {
            if let Some(br_str) = fmtp_param(fmtp_str, "bitrate") {
                match br_str.parse::<u32>() {
                    Ok(br) => bitrate = Some(br),
                    Err(_) => {
                        warnings.push(SdpWarning::unparseable_numeric_attribute(
                            "g7221 fmtp bitrate",
                            br_str,
                        ));
                    }
                }
            }
        }
    }

    (ptime, bitrate)
}

/// Cursor over an `a=rtpmap`/`a=fmtp` attribute value.
///
/// Whitespace is decided once, here, mirroring `parse_ul` and `token` in
/// sofia-sip's `sdp_parse.c` — not re-trimmed at
/// each field after a naive split, which is how a field with no trim call
/// (the encoding name) used to slip through with leading whitespace attached.
struct AttrCursor<'a> {
    rest: &'a str,
}

impl<'a> AttrCursor<'a> {
    fn new(value: &'a str) -> Self {
        Self { rest: value }
    }

    /// Skip a run of SP/HTAB.
    fn skip_ws(&mut self) {
        self.rest = self
            .rest
            .trim_start_matches([' ', '\t']);
    }

    /// Skip whitespace, take the run of ASCII digits, parse it, skip trailing
    /// whitespace — mirrors `parse_ul`'s `strspn` on both sides of the number.
    fn number<T: std::str::FromStr>(&mut self) -> Option<T> {
        self.skip_ws();
        let end = self
            .rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(
                self.rest
                    .len(),
            );
        if end == 0 {
            return None;
        }
        let (digits, rest) = self
            .rest
            .split_at(end);
        self.rest = rest;
        self.skip_ws();
        digits
            .parse()
            .ok()
    }

    /// Skip whitespace, then take the maximal run of chars that are neither
    /// whitespace nor one of `stop`.
    fn field(&mut self, stop: &[char]) -> &'a str {
        self.skip_ws();
        let end = self
            .rest
            .find(|c: char| c == ' ' || c == '\t' || stop.contains(&c))
            .unwrap_or(
                self.rest
                    .len(),
            );
        let (field, rest) = self
            .rest
            .split_at(end);
        self.rest = rest;
        field
    }

    /// Consume an expected delimiter char if present.
    fn eat(&mut self, ch: char) -> bool {
        match self
            .rest
            .strip_prefix(ch)
        {
            Some(rest) => {
                self.rest = rest;
                true
            }
            None => false,
        }
    }

    /// The unconsumed remainder, verbatim (no whitespace stripped).
    fn remaining(&self) -> &'a str {
        self.rest
    }
}

/// Parse an `a=rtpmap` attribute value into `(pt, name, clock_rate, channels)`.
///
/// Grammar: number (pt) / field stopping at `/` (name) / required `/` / number
/// (clock rate) / optionally `/` then number (channels). The name has no
/// charset restriction beyond "not whitespace, not `/`" — sofia's stricter
/// `TOKEN` charset is enforced later, in `audio_codec_string`, on a path that
/// warns and drops one codec rather than the whole media section.
///
/// Returns `Err(MalformedRtpmap)` for any structural violation (missing
/// separator, non-numeric PT, non-numeric clock rate, non-numeric channel count).
fn parse_rtpmap(value: &str) -> Result<(u8, String, u32, Option<u8>), SdpCodecError> {
    let mut cursor = AttrCursor::new(value);

    let pt = cursor
        .number()
        .ok_or_else(|| SdpCodecError::MalformedRtpmap(value.to_string()))?;

    let name = cursor.field(&['/']);
    if name.is_empty() {
        return Err(SdpCodecError::MalformedRtpmap(value.to_string()));
    }
    let name = name.to_string();

    if !cursor.eat('/') {
        return Err(SdpCodecError::MalformedRtpmap(value.to_string()));
    }

    let clock_rate = cursor
        .number()
        .ok_or_else(|| SdpCodecError::MalformedRtpmap(value.to_string()))?;

    let channels = if cursor.eat('/') {
        cursor.skip_ws();
        if cursor
            .remaining()
            .is_empty()
        {
            None
        } else {
            Some(
                cursor
                    .number()
                    .ok_or_else(|| SdpCodecError::MalformedRtpmap(value.to_string()))?,
            )
        }
    } else {
        None
    };

    if !cursor
        .remaining()
        .is_empty()
    {
        return Err(SdpCodecError::MalformedRtpmap(value.to_string()));
    }

    Ok((pt, name, clock_rate, channels))
}

/// Extract `(pt, params)` from an `a=fmtp` attribute value.
///
/// Only the whitespace between the payload type and the params is a
/// separator (consumed by `number`'s trailing skip); everything after that,
/// including any trailing whitespace, is opaque `byte-string` content per
/// RFC 8866 and is returned verbatim.
///
/// A non-numeric payload type is a hard error — same structural breakage as in `a=rtpmap`.
fn parse_fmtp_pt(value: &str) -> Result<(u8, String), SdpCodecError> {
    let mut cursor = AttrCursor::new(value);
    let pt = cursor
        .number()
        .ok_or_else(|| SdpCodecError::MalformedFmtp(value.to_string()))?;
    Ok((
        pt,
        cursor
            .remaining()
            .to_string(),
    ))
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

    fn rtp_codec<'a>(entries: impl IntoIterator<Item = &'a SdpCodecEntry>) -> Vec<&'a SdpCodec> {
        entries
            .into_iter()
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

    // --- retained sections ---

    #[test]
    fn port_zero_section_contributes_nothing_but_is_retained_whole() {
        // A held or declined stream: the offer still names its codecs, and that is
        // what an operator reads when the complaint is "no audio".
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 0 RTP/AVP 104 96\r\n",
                "a=rtpmap:104 AMR-WB/16000\r\n",
                "a=fmtp:104 mode-set=1; max-red=0\r\n",
                "a=rtpmap:96 telephone-event/16000\r\n",
                "a=fmtp:96 0-15\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();

        assert!(codecs.is_empty(), "port 0 contributes no codec entries");
        assert!(codecs
            .audio_codec_string(&CodecStringOptions::audio(), None)
            .unwrap()
            .is_empty());

        let sections = codecs.sections();
        assert_eq!(
            sections.len(),
            1,
            "every m= line yields exactly one section"
        );
        let held = &sections[0];
        assert_eq!(held.port(), 0);
        assert!(!held.is_negotiable());
        assert_eq!(held.media_type(), &SdpMediaType::Audio);

        let rtp = rtp_codec(held.entries());
        let amr = codec_named(&rtp, "AMR-WB").expect("the held codec must survive by name");
        assert_eq!(amr.clock_rate(), 16000);
        assert_eq!(amr.fmtp(), Some("mode-set=1; max-red=0"));

        let te = held.non_codec_payloads();
        assert_eq!(te.len(), 1);
        assert_eq!(
            te[0]
                .fmtp
                .as_deref(),
            Some("0-15")
        );
    }

    #[test]
    fn a_retained_sections_payloads_stay_out_of_the_top_level_views() {
        // sections() is the inventory; the top-level accessors stay the switch's view.
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 0 RTP/AVP 0 99 101\r\n",
                "a=rtpmap:101 telephone-event/8000\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(
            codecs
                .non_codec_payloads()
                .count(),
            0
        );
        assert_eq!(
            codecs
                .unmapped()
                .count(),
            0
        );

        let held = &codecs.sections()[0];
        assert_eq!(
            held.non_codec_payloads()
                .len(),
            1
        );
        assert_eq!(
            held.unmapped()
                .len(),
            1,
            "PT 99 has no rtpmap and no static entry, on the section it appeared in"
        );
    }

    #[test]
    fn fmtp_for_ignores_a_held_sections_parameters() {
        // fmtp_for drives rtp_force_audio_fmtp. Answering from a stream that is not
        // being negotiated pins the wrong parameters on the live call.
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 0 RTP/AVP 104\r\n",
                "a=rtpmap:104 AMR-WB/16000\r\n",
                "a=fmtp:104 octet-align=1\r\n",
                "m=audio 5004 RTP/AVP 105\r\n",
                "a=rtpmap:105 AMR-WB/16000\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(codecs.fmtp_for("AMR-WB", 16000), None);
    }

    #[test]
    fn port_zero_image_section_yields_no_t38_entry() {
        let sdp = format!("{}m=image 0 udptl t38\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(codecs.is_empty());
        assert!(codecs
            .audio_codec_string(&CodecStringOptions::audio(), None)
            .unwrap()
            .is_empty());

        let declined = &codecs.sections()[0];
        assert!(!declined.is_negotiable());
        assert!(matches!(declined.entries()[0], SdpCodecEntry::T38));
    }

    #[test]
    fn non_rtp_proto_reads_no_payload_types_and_raises_no_warning() {
        // sofia fills a section's rtpmap list only for the transports it maps to an
        // RTP proto, so a datachannel format token is not a malformed payload type.
        let sdp = format!(
            "{}m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(codecs
            .warnings()
            .is_empty());

        let section = &codecs.sections()[0];
        assert!(section
            .entries()
            .is_empty());
        assert_eq!(section.proto(), "UDP/DTLS/SCTP");
        assert_eq!(section.formats(), "webrtc-datachannel");
    }

    #[test]
    fn non_rtp_proto_on_audio_gets_the_same_treatment() {
        // The gate is sofia's, not the media type's: an m=audio under udptl carries
        // no rtpmaps for the switch either.
        let sdp = format!("{}m=audio 5004 udptl t38\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(codecs
            .warnings()
            .is_empty());
        assert!(codecs.is_empty());
        assert_eq!(codecs.sections()[0].formats(), "t38");
    }

    #[test]
    fn a_proto_merely_containing_rtp_is_not_an_rtp_transport() {
        // sofia matches the proto against an exact set; TCP/RTP/AVP is not in it.
        let sdp = format!("{}m=audio 5004 TCP/RTP/AVP 0\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(codecs.is_empty());
        assert!(codecs.sections()[0]
            .entries()
            .is_empty());
    }

    #[test]
    fn text_section_codecs_are_retained_but_never_negotiable() {
        let sdp = format!(
            "{}m=text 5000 RTP/AVP 98\r\na=rtpmap:98 t140/1000\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert!(codecs.is_empty());
        assert_eq!(
            codecs
                .audio()
                .count(),
            0
        );
        assert_eq!(
            codecs
                .video()
                .count(),
            0
        );

        let section = &codecs.sections()[0];
        assert!(!section.is_negotiable());
        let rtp = rtp_codec(section.entries());
        assert_eq!(
            codec_named(&rtp, "t140")
                .expect("the text codec must survive")
                .clock_rate(),
            1000
        );
    }

    #[test]
    fn sections_keep_m_line_order_across_negotiable_and_not() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 0\r\n",
                "m=video 0 RTP/AVP 99\r\n",
                "a=rtpmap:99 H264/90000\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let sections = codecs.sections();
        assert_eq!(sections[0].media_type(), &SdpMediaType::Audio);
        assert!(sections[0].is_negotiable());
        assert_eq!(sections[1].media_type(), &SdpMediaType::Video);
        assert!(!sections[1].is_negotiable());

        let rtp = rtp_codec(codecs.entries());
        assert_eq!(rtp.len(), 1, "the declined video must not reach entries()");
        assert!(codec_named(&rtp, "PCMU").is_some());
    }

    #[test]
    fn a_malformed_section_is_retained_beside_its_warning() {
        let sdp = format!(
            "{}m=audio 5004 RTP/AVP 0\r\na=rtpmap:x PCMU/8000\r\n",
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(
            codecs
                .warnings()
                .len(),
            1
        );
        let section = &codecs.sections()[0];
        assert!(section
            .entries()
            .is_empty());
        assert_eq!(section.port(), 5004);
    }

    #[test]
    fn a_non_negotiable_section_still_reports_its_parse_warnings() {
        // These sections are parsed rather than skipped, so they warn where the
        // switch, which never looks at them, has no occasion to.
        let sdp = format!("{}m=audio 0 RTP/AVP 0\r\na=ptime:abc\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(
            codecs
                .warnings()
                .len(),
            1
        );
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
        assert!(matches!(
            codecs
                .entries()
                .next(),
            Some(SdpCodecEntry::T38)
        ));
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
        assert!(matches!(
            codecs
                .entries()
                .next(),
            Some(SdpCodecEntry::T38)
        ));
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
    fn telephone_event_retains_payload_type_rate_and_fmtp() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 0 101 102\r\n",
                "a=rtpmap:101 telephone-event/8000\r\n",
                "a=fmtp:101 0-16\r\n",
                "a=rtpmap:102 telephone-event/16000\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let te: Vec<_> = codecs
            .non_codec_payloads()
            .collect();
        assert_eq!(
            te.len(),
            2,
            "both telephone-event payloads must be retained"
        );

        assert_eq!(te[0].kind, NonCodecKind::TelephoneEvent);
        assert_eq!(te[0].payload_type, 101);
        assert_eq!(te[0].clock_rate, 8000);
        assert_eq!(
            te[0]
                .fmtp
                .as_deref(),
            Some("0-16"),
            "the offered digit range is what an operator reads on a DTMF fault"
        );
        assert!(te[0].has_rtpmap);
        assert_eq!(te[0].media_type, SdpMediaType::Audio);

        assert_eq!(te[1].payload_type, 102);
        assert_eq!(te[1].clock_rate, 16000);
        assert!(te[1]
            .fmtp
            .is_none());

        // Still excluded from entries() and distinguishable from an unresolvable payload.
        let rtp = rtp_codec(codecs.entries());
        assert!(codec_named(&rtp, "telephone-event").is_none());
        assert_eq!(
            codecs
                .unmapped()
                .count(),
            0
        );
    }

    #[test]
    fn comfort_noise_from_static_table_has_no_rtpmap() {
        let sdp = format!("{}m=audio 5004 RTP/AVP 0 13\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let cn: Vec<_> = codecs
            .non_codec_payloads()
            .collect();
        assert_eq!(cn.len(), 1);
        assert_eq!(cn[0].kind, NonCodecKind::ComfortNoise);
        assert_eq!(cn[0].payload_type, 13);
        assert_eq!(cn[0].clock_rate, 8000);
        assert!(cn[0]
            .fmtp
            .is_none());
        assert!(
            !cn[0].has_rtpmap,
            "PT 13 resolved from the RFC 3551 table, not declared by the peer"
        );

        let rtp = rtp_codec(codecs.entries());
        assert!(codec_named(&rtp, "CN").is_none());
        assert_eq!(
            codecs
                .unmapped()
                .count(),
            0
        );
    }

    #[test]
    fn comfort_noise_with_explicit_rtpmap_is_distinguishable() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 0 13\r\n",
                "a=rtpmap:13 CN/8000\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let cn: Vec<_> = codecs
            .non_codec_payloads()
            .collect();
        assert_eq!(cn.len(), 1);
        assert!(cn[0].has_rtpmap);
    }

    #[test]
    fn non_codec_payloads_are_an_inventory_not_a_set() {
        // Two sections offering DTMF at one rate under different payload types: both
        // survive, in offer order. Collapsing them would lose which side offered what.
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 0 101\r\n",
                "a=rtpmap:101 telephone-event/8000\r\n",
                "m=audio 5006 RTP/AVP 8 96\r\n",
                "a=rtpmap:96 telephone-event/8000\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let te: Vec<_> = codecs
            .non_codec_payloads()
            .collect();
        assert_eq!(te.len(), 2);
        assert_eq!(te[0].payload_type, 101);
        assert_eq!(te[1].payload_type, 96);
    }

    #[test]
    fn non_codec_payloads_never_reach_the_codec_string() {
        let sdp = format!(
            concat!(
                "{}",
                "m=audio 5004 RTP/AVP 0 13 101\r\n",
                "a=rtpmap:101 telephone-event/8000\r\n"
            ),
            sdp_header()
        );
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        assert_eq!(
            codecs
                .non_codec_payloads()
                .count(),
            2
        );
        let cs = codecs
            .audio_codec_string(&CodecStringOptions::default(), None)
            .unwrap();
        assert_eq!(cs.len(), 1, "only PCMU may reach the codec string");
        let rendered = cs.to_string();
        assert!(!rendered.contains("telephone-event"));
        assert!(!rendered.contains("CN"));
    }

    // --- unmapped payload types ---

    #[test]
    fn dynamic_pt_without_rtpmap_is_unmapped_not_error() {
        let sdp = format!("{}m=audio 5004 RTP/AVP 0 99\r\n", sdp_header());
        let codecs = SdpCodecs::parse(&sdp).unwrap();
        let unmapped: Vec<_> = codecs
            .unmapped()
            .collect();
        assert_eq!(unmapped.len(), 1);
        assert_eq!(unmapped[0].payload_type, 99);
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
        assert_eq!(
            codecs
                .entries()
                .count(),
            0,
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
        assert_eq!(
            codecs
                .entries()
                .count(),
            0,
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
        assert_eq!(
            codecs
                .entries()
                .count(),
            0
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
