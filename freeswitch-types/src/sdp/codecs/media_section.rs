//! One `m=` section's payload walk: RTP-transport detection and per-payload resolution.
//!
//! Line numbers in this module index FreeSWITCH `v1.11.1`
//! (`c2c59645f6911a76589e5008c4d73349ded44b65`).

use crate::sdp::error::{SdpCodecError, SdpWarning};
use crate::sdp::{
    NonCodecKind, NonCodecPayload, SdpCodec, SdpCodecEntry, SdpDirection, SdpMediaType,
    UnmappedPayload,
};

use super::attrs::{
    attribute_tables, default_ptime_ms, fmtp_param, parse_ptime_value, ptime_from_attrs,
};

/// One `m=` section's payload walk, staged locally so a mid-section parse
/// failure can be discarded wholesale instead of reaching
/// [`SdpMediaSection`](super::SdpMediaSection).
#[derive(Default)]
pub(super) struct MediaSection {
    pub(super) entries: Vec<SdpCodecEntry>,
    pub(super) unmapped: Vec<UnmappedPayload>,
    pub(super) warnings: Vec<SdpWarning>,
    pub(super) non_codec: Vec<NonCodecPayload>,
}

/// `m=image`, matched case-insensitively as FreeSWITCH's own comparison is.
pub(super) fn is_image(media_type: &SdpMediaType) -> bool {
    matches!(media_type, SdpMediaType::Other(s) if s.eq_ignore_ascii_case("image"))
}

/// Whether sofia would read this section's format list as RTP payload types.
///
/// `sdp_media_transport` matches the proto against an exact set, so a name merely
/// containing `RTP` is not one; `sdp_media_has_rtp` then decides whether the
/// format list, `a=rtpmap` and `a=fmtp` are read into the section's rtpmap list at
/// all. The bare `RTP` is sofia's non-strict spelling of `RTP/AVP`.
pub(super) fn proto_has_rtp(proto: &str) -> bool {
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
pub(super) fn parse_media_section(
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
        } else if let Some(st) = crate::sdp::static_payload::rfc3551_payload_type(pt) {
            (st.encoding_name, st.clock_rate, st.channels, false)
        } else {
            section
                .unmapped
                .push(UnmappedPayload::new(pt, media_type.clone()));
            continue;
        };

        let canonical = crate::sdp::static_payload::canonical_iananame(name);

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
    let mut bitrate = crate::sdp::static_payload::known_bitrate(payload_type);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdp::{CodecStringOptions, SdpCodecs, SdpMediaType, SdpWarning};

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

    // --- non-RTP proto ---

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

    // --- text section ---

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

    // --- image / T38 ---

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

    // --- malformed section handling ---

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
}
