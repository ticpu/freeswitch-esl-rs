//! Which codec-string entries a loaded FreeSWITCH survives implementation matching.
//!
//! `switch_loadable_module_get_codecs_sorted` (`switch_loadable_module.c:2796-2929`) drops
//! a codec-string entry in two unlogged places: no codec interface registered under that
//! name/modname at all (`:2547-2572`, `:2849`), or an interface exists but no implementation
//! matches an explicit qualifier (`:2851-2909`). Neither is discoverable from an ESL
//! connection — no API exposes the loaded implementation table — so the caller supplies
//! what it knows via [`CodecImplementation`] and [`CodecString::retain_available`] reports
//! what would be silently dropped.

use crate::sdp::codec::SdpMediaType;
use crate::sdp::codec_string::{CodecString, CodecStringEntry};

/// One loaded codec implementation, mirroring `switch_codec_implementation_t`.
///
/// Every qualifier is `Option`; `None` means "unknown, do not constrain" rather than
/// "codec has no such value". A caller who only knows codec names (not the loaded
/// implementation table) can construct one of these per name with all qualifiers
/// `None` — this still catches an entry naming a codec that is not loaded at all
/// (the module-lookup failure), it just cannot catch a qualifier mismatch against an
/// implementation that *is* loaded (the second, ptime/rate-specific failure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecImplementation {
    name: String,
    modname: Option<String>,
    media_type: Option<SdpMediaType>,
    rate: Option<u32>,
    ptime: Option<u32>,
    bitrate: Option<u32>,
    channels: Option<u32>,
}

impl CodecImplementation {
    /// Create a new implementation descriptor with only the codec name known.
    ///
    /// `None` media type means "unknown, treat as audio" — the qualifier checks
    /// apply. Set it with [`with_media_type`](Self::with_media_type) for a video
    /// codec, whose implementations bypass every qualifier check in FreeSWITCH.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            modname: None,
            media_type: None,
            rate: None,
            ptime: None,
            bitrate: None,
            channels: None,
        }
    }

    /// Set the media type (`switch_codec_implementation_t::codec_type`).
    ///
    /// A [`SdpMediaType::Video`] implementation matches on name (and modname)
    /// alone — `switch_loadable_module_get_codecs_sorted` wraps every qualifier
    /// comparison in `if (imp->codec_type != SWITCH_CODEC_TYPE_VIDEO)`
    /// (`switch_loadable_module.c:2855, 2886`).
    pub fn with_media_type(mut self, media_type: SdpMediaType) -> Self {
        self.media_type = Some(media_type);
        self
    }

    /// Set the module name (`switch_codec_implementation_t::modname`).
    pub fn with_modname(mut self, modname: impl Into<String>) -> Self {
        self.modname = Some(modname.into());
        self
    }

    /// Set the clock rate this implementation matches against (`actual_samples_per_second`,
    /// or `samples_per_second` for G.722's RFC 3551 quirk).
    pub fn with_rate(mut self, rate: u32) -> Self {
        self.rate = Some(rate);
        self
    }

    /// Set the packetization interval in milliseconds (`microseconds_per_packet / 1000`).
    pub fn with_ptime(mut self, ptime: u32) -> Self {
        self.ptime = Some(ptime);
        self
    }

    /// Set the bitrate in bits/s (`bits_per_second`).
    pub fn with_bitrate(mut self, bitrate: u32) -> Self {
        self.bitrate = Some(bitrate);
        self
    }

    /// Set the channel count (`number_of_channels`).
    pub fn with_channels(mut self, channels: u32) -> Self {
        self.channels = Some(channels);
        self
    }

    /// The codec name (`iananame`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The module name, if known.
    pub fn modname(&self) -> Option<&str> {
        self.modname
            .as_deref()
    }

    /// The media type, if known.
    pub fn media_type(&self) -> Option<&SdpMediaType> {
        self.media_type
            .as_ref()
    }

    /// The clock rate, if known.
    pub fn rate(&self) -> Option<u32> {
        self.rate
    }

    /// The packetization interval in milliseconds, if known.
    pub fn ptime(&self) -> Option<u32> {
        self.ptime
    }

    /// The bitrate in bits/s, if known.
    pub fn bitrate(&self) -> Option<u32> {
        self.bitrate
    }

    /// The channel count, if known.
    pub fn channels(&self) -> Option<u32> {
        self.channels
    }
}

/// `true` if `entry` matches `imp` under the same rules as the second matching pass
/// in `switch_loadable_module_get_codecs_sorted` (`switch_loadable_module.c:2885-2909`).
fn matches_implementation(entry: &CodecStringEntry, imp: &CodecImplementation) -> bool {
    if !entry
        .name()
        .eq_ignore_ascii_case(imp.name())
    {
        return false;
    }

    if let Some(entry_mod) = entry.modname() {
        match imp.modname() {
            Some(imp_mod) if entry_mod.eq_ignore_ascii_case(imp_mod) => {}
            Some(_) => return false,
            None => {}
        }
    }

    // Video implementations bypass every qualifier comparison
    // (`switch_loadable_module.c:2855, 2886`); name/modname above is the whole check.
    if matches!(imp.media_type(), Some(SdpMediaType::Video)) {
        return true;
    }

    // G.722 registers samples_per_second = 8000 and actual_samples_per_second = 16000
    // (`mod_spandsp_codecs.c`, opposite of the usual convention). The first pass compares
    // an explicit rate against actual_samples_per_second, the second against
    // samples_per_second, so both @8000h and @16000h resolve, via different passes. A
    // single `rate` field on `CodecImplementation` can't express which pass would fire,
    // so G.722 never constrains on rate here (mirrors `CodecStringEntry::simplify`'s
    // G.722 carve-out, `codec_string.rs`).
    let is_g722 = entry
        .name()
        .eq_ignore_ascii_case("g722");

    // An explicit `0` qualifier means "unconstrained" — switch_loadable_module.c guards
    // every comparison with `if (rate && …)` / `if (bit && …)` / `if (channels && …)`, so
    // a preference of `@0h`/`@0b`/`@0c` disables that check rather than requiring a zero
    // implementation value. `nonzero` collapses `Some(0)` to `None` so the existing
    // `Option` match below already treats it as unconstrained.
    if !is_g722 {
        if let (Some(er), Some(ir)) = (nonzero(entry.rate()), imp.rate()) {
            if er != ir {
                return false;
            }
        }
    }

    if let (Some(ep), Some(ip)) = (nonzero(entry.ptime()), imp.ptime()) {
        if ep != ip {
            return false;
        }
    }

    if let (Some(eb), Some(ib)) = (nonzero(entry.bitrate()), imp.bitrate()) {
        if eb != ib {
            return false;
        }
    }

    // Channels is the odd one out: switch_loadable_module_get_codecs_sorted seeds its
    // preference-parsing variable at `channels = 1`, not 0 like rate/interval/bit
    // (`switch_loadable_module.c:2806`), so an *absent* `@Nc` still constrains to mono
    // — only an *explicit* `@0c` is the falsy value that disables the check. Do not
    // reuse `norm_channels` (the dedup-key normalization): that helper collapses
    // `None` and `Some(0)` to the same value 1, which is right for dedup but conflates
    // "absent" with "explicitly unconstrained" here.
    if let Some(ic) = imp.channels() {
        let required = entry
            .channels()
            .unwrap_or(1);
        if required != 0 && required != ic {
            return false;
        }
    }

    true
}

/// Collapse an explicit `0` qualifier to `None` ("unconstrained"); see
/// [`matches_implementation`].
fn nonzero(v: Option<u32>) -> Option<u32> {
    v.filter(|&n| n != 0)
}

impl CodecString {
    /// Remove entries that match no implementation in `implementations`, returning the
    /// removed entries in their original order.
    ///
    /// Ports the drop behaviour of `switch_loadable_module_get_codecs_sorted`
    /// (`switch_loadable_module.c:2849-2909`): a name/modname miss
    /// (`switch_loadable_module_get_codec_interface` returning NULL) or a qualifier miss
    /// against every implementation registered under that name both silently drop the
    /// entry in FreeSWITCH. A name-only implementation list (all qualifiers `None`) models
    /// only the first failure; [`CodecString::qualified`] identifies the entries still
    /// exposed to the second, qualifier-matching failure.
    pub fn retain_available<'a, I>(&mut self, implementations: I) -> Vec<CodecStringEntry>
    where
        I: IntoIterator<Item = &'a CodecImplementation>,
    {
        let impls: Vec<&CodecImplementation> = implementations
            .into_iter()
            .collect();

        let mut removed = Vec::new();
        for entry in std::mem::take(self) {
            if impls
                .iter()
                .any(|imp| matches_implementation(&entry, imp))
            {
                self.push(entry);
            } else {
                removed.push(entry);
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> CodecString {
        s.parse()
            .unwrap()
    }

    #[test]
    fn unavailable_codec_removed_case_insensitive_both_directions() {
        let mut cs = parse("pcmu@20i,EVS~mode=1,AMR");
        let impls = vec![
            CodecImplementation::new("PCMU"),
            CodecImplementation::new("amr"),
        ];
        let removed = cs.retain_available(&impls);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].name(), "EVS");
        assert_eq!(removed[0].fmtp(), Some("mode=1"));
        assert_eq!(cs.len(), 2);
        assert!(cs.contains_name("pcmu"));
        assert!(cs.contains_name("AMR"));
    }

    #[test]
    fn amr_ptime_mismatch_is_removed() {
        // mod_amr.c:690-716 registers both AMR implementations at 20000us (20ms) only.
        // A peer offering ptime=40 must match nothing and be dropped.
        let mut cs = parse("AMR@8000h@40i");
        let impls = vec![CodecImplementation::new("AMR")
            .with_rate(8000)
            .with_ptime(20)];
        let removed = cs.retain_available(&impls);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].name(), "AMR");
        assert!(cs.is_empty());
    }

    #[test]
    fn amr_ptime_mismatch_survives_name_only_implementation_list() {
        // Pins the documented degradation: a name-only list (site (a) coverage only)
        // cannot catch the qualifier-mismatch failure (site (b)).
        let mut cs = parse("AMR@8000h@40i");
        let impls = vec![CodecImplementation::new("AMR")];
        let removed = cs.retain_available(&impls);
        assert!(removed.is_empty());
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn modname_mismatch_is_removed() {
        let mut cs = CodecString::new();
        cs.push(
            CodecStringEntry::new("EVS")
                .unwrap()
                .with_module("mod_evs")
                .unwrap(),
        );
        let impls = vec![CodecImplementation::new("EVS").with_modname("mod_other")];
        let removed = cs.retain_available(&impls);
        assert_eq!(removed.len(), 1);
        assert!(cs.is_empty());
    }

    #[test]
    fn modname_unknown_on_implementation_does_not_constrain() {
        let mut cs = CodecString::new();
        cs.push(
            CodecStringEntry::new("EVS")
                .unwrap()
                .with_module("mod_evs")
                .unwrap(),
        );
        let impls = vec![CodecImplementation::new("EVS")];
        let removed = cs.retain_available(&impls);
        assert!(removed.is_empty());
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn removed_entries_preserve_original_order() {
        let mut cs = parse("EVS,H264,AMR@8000h@40i");
        let impls = vec![CodecImplementation::new("PCMU")];
        let removed = cs.retain_available(&impls);
        assert_eq!(removed.len(), 3);
        assert_eq!(removed[0].name(), "EVS");
        assert_eq!(removed[1].name(), "H264");
        assert_eq!(removed[2].name(), "AMR");
        assert!(cs.is_empty());
    }

    // --- video implementations bypass qualifier checks ---

    #[test]
    fn video_implementation_matches_regardless_of_rate() {
        // switch_loadable_module.c:2855/:2886 wrap every qualifier comparison in
        // `if (imp->codec_type != SWITCH_CODEC_TYPE_VIDEO)`, so a video implementation
        // matches on name alone. Before this fix, a declared rate of 90000 would have
        // been compared against the entry's 8000 and dropped VP8@8000h.
        let mut cs = parse("VP8@8000h");
        let impls = vec![CodecImplementation::new("VP8")
            .with_media_type(SdpMediaType::Video)
            .with_rate(90000)];
        let removed = cs.retain_available(&impls);
        assert!(removed.is_empty());
        assert_eq!(cs.len(), 1);
    }

    // --- G.722's dual rate ---

    #[test]
    fn g722_matches_at_either_advertised_rate() {
        // mod_spandsp_codecs.c registers G.722 with samples_per_second = 8000 and
        // actual_samples_per_second = 16000 -- the opposite of the usual convention.
        // The first pass compares an explicit rate against actual_samples_per_second
        // (16000); the second compares against samples_per_second (8000). Both
        // G722@8000h and G722@16000h therefore resolve, via different passes, and a
        // single `rate` field on CodecImplementation cannot express that -- so the
        // rate comparison is skipped entirely for G.722.
        let mut cs = parse("G722@16000h@20i,G722@8000h@20i");
        let impls = vec![CodecImplementation::new("G722")
            .with_rate(8000)
            .with_ptime(20)];
        let removed = cs.retain_available(&impls);
        assert!(removed.is_empty());
        assert_eq!(cs.len(), 2);
    }

    // --- explicit zero qualifiers are unconstrained ---

    #[test]
    fn pcmu_zero_rate_does_not_constrain() {
        let mut cs = parse("PCMU@0h");
        let impls = vec![CodecImplementation::new("PCMU").with_rate(8000)];
        let removed = cs.retain_available(&impls);
        assert!(removed.is_empty());
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn pcmu_zero_ptime_does_not_constrain() {
        let mut cs = parse("PCMU@0i");
        let impls = vec![CodecImplementation::new("PCMU").with_ptime(20)];
        let removed = cs.retain_available(&impls);
        assert!(removed.is_empty());
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn pcmu_zero_bitrate_does_not_constrain() {
        let mut cs = parse("PCMU@0b");
        let impls = vec![CodecImplementation::new("PCMU").with_bitrate(64000)];
        let removed = cs.retain_available(&impls);
        assert!(removed.is_empty());
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn pcmu_zero_channels_does_not_constrain() {
        let mut cs = parse("PCMU@0c");
        let impls = vec![CodecImplementation::new("PCMU").with_channels(2)];
        let removed = cs.retain_available(&impls);
        assert!(removed.is_empty());
        assert_eq!(cs.len(), 1);
    }
}
