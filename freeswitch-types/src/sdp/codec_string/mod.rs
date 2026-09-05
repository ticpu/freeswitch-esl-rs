//! FreeSWITCH codec-string grammar: `[modname.]name[~fmtp][@rate h][@ptime i][@bitrate b][@channels c]`.
//!
//! See `docs/codec-string-format.md` for the full grammar, parse-order hazards,
//! and fmtp delimiter collisions. This module provides typed construction and
//! round-trip-safe serialisation.

mod entry;
mod parse;

pub use entry::CodecStringEntry;

use std::fmt;
use std::str::FromStr;

use crate::sdp::error::{CodecStringError, SdpWarning};
use crate::sdp::static_payload::{default_ptime, default_rate};

use parse::parse_codec_string_inner;

/// A FreeSWITCH codec string — a comma-separated list of [`CodecStringEntry`] values.
///
/// Parses the grammar from `docs/codec-string-format.md` and emits it back via
/// [`Display`](fmt::Display). Both directions are infallible given a valid list;
/// validation happens at entry construction time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodecString(Vec<CodecStringEntry>);

impl CodecString {
    /// Create an empty codec string.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Construct directly from a parsed entry list — the `parse` submodule's shared
    /// entry point across [`FromStr`] and [`parse_lenient`](Self::parse_lenient).
    pub(super) fn from_entries(entries: Vec<CodecStringEntry>) -> Self {
        Self(entries)
    }

    /// Append an entry to the list.
    pub fn push(&mut self, entry: CodecStringEntry) {
        self.0
            .push(entry);
    }

    /// All entries in order.
    pub fn entries(&self) -> &[CodecStringEntry] {
        &self.0
    }

    /// Iterator over entries.
    pub fn iter(&self) -> std::slice::Iter<'_, CodecStringEntry> {
        self.0
            .iter()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.0
            .len()
    }

    /// `true` when the list is empty.
    pub fn is_empty(&self) -> bool {
        self.0
            .is_empty()
    }

    /// Mutable iterator over entries.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, CodecStringEntry> {
        self.0
            .iter_mut()
    }

    /// Append all entries from `other` (non-draining; `other` is unchanged).
    pub fn extend_from(&mut self, other: &CodecString) {
        self.0
            .extend_from_slice(&other.0);
    }

    /// Move all entries from `other` into `self`, leaving `other` empty.
    pub fn append(&mut self, other: &mut CodecString) {
        self.0
            .append(&mut other.0);
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.0
            .clear();
    }

    /// Retain only entries for which `f` returns `true`.
    pub fn retain(&mut self, f: impl FnMut(&CodecStringEntry) -> bool) {
        self.0
            .retain(f);
    }

    /// Insert an entry at position `index`, shifting later entries right.
    ///
    /// Non-panicking: returns `Err(entry)` when `index > len` instead of panicking
    /// like [`Vec::insert`].
    pub fn insert(
        &mut self,
        index: usize,
        entry: CodecStringEntry,
    ) -> Result<(), CodecStringEntry> {
        if index
            > self
                .0
                .len()
        {
            return Err(entry);
        }
        self.0
            .insert(index, entry);
        Ok(())
    }

    /// Remove and return the entry at position `index`.
    ///
    /// Non-panicking: returns `None` when `index >= len` instead of panicking
    /// like [`Vec::remove`].
    pub fn remove(&mut self, index: usize) -> Option<CodecStringEntry> {
        if index
            < self
                .0
                .len()
        {
            Some(
                self.0
                    .remove(index),
            )
        } else {
            None
        }
    }

    /// Iterator over entries that carry at least one explicit qualifier or fmtp.
    ///
    /// Two unrelated reasons make these worth surfacing to the caller. A numeric
    /// qualifier (rate/ptime/bitrate/channels) risks a silent, unlogged drop at
    /// match time: FreeSWITCH's implementation matching compares these against
    /// what's loaded, and neither of its two passes falls back to an unqualified
    /// match — `mod_amr.c:690-716` registers AMR at 20 ms only, so `AMR@8000h@40i`
    /// matches nothing and vanishes unlogged. An fmtp never affects that matching
    /// (`switch_loadable_module.c:2876-2877` just copies it through once a match is
    /// already found by name/qualifiers), so it can't cause this drop — but the
    /// switch hands it to the codec implementation, where it changes negotiated
    /// behaviour (AMR octet-aligned vs bandwidth-efficient is the case that
    /// matters), and an SDP `a=fmtp` doesn't translate cleanly into a codec-string
    /// `~fmtp` in general.
    pub fn qualified(&self) -> impl Iterator<Item = &CodecStringEntry> {
        self.0
            .iter()
            .filter(|e| {
                e.rate()
                    .is_some()
                    || e.ptime()
                        .is_some()
                    || e.bitrate()
                        .is_some()
                    || e.channels()
                        .is_some()
                    || e.fmtp()
                        .is_some()
            })
    }

    /// Maximum entries FreeSWITCH processes per codec string (`SWITCH_MAX_CODECS`,
    /// `switch_types.h:595`).
    ///
    /// The cap applies to raw token slots — including the empty ones consecutive commas
    /// produce — not to the number of parsed entries.
    pub const MAX_SWITCH_ENTRIES: usize = 50;
}

impl fmt::Display for CodecString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, entry) in self
            .0
            .iter()
            .enumerate()
        {
            if i > 0 {
                f.write_str(",")?;
            }
            write!(f, "{entry}")?;
        }
        Ok(())
    }
}

/// Normalize ptime for dedup: `None` or `0` → `default_ptime(name)`, else as-is.
///
/// Mirrors `if (ointerval == 0) { ointerval = switch_default_ptime(name, 0); }` in
/// `switch_loadable_module.c:2816-2818`. The C initializes `interval = 0`; absence of
/// an `@xi` qualifier leaves it at 0, which then triggers the default lookup.
fn norm_ptime(e: &CodecStringEntry) -> u32 {
    match e.ptime() {
        None | Some(0) => default_ptime(e.name()),
        Some(n) => n,
    }
}

/// Normalize rate for dedup: `None` or `0` → `default_rate(name)`, else as-is.
///
/// Mirrors `if (orate == 0) { orate = switch_default_rate(name, 0); }` in
/// `switch_loadable_module.c:2820-2822`.
fn norm_rate(e: &CodecStringEntry) -> u32 {
    match e.rate() {
        None | Some(0) => default_rate(e.name()),
        Some(n) => n,
    }
}

/// Normalize channels for dedup: `None` or `0` → `1`, else as-is.
///
/// The C initializes `jchannels = 1` and `channels = 1`; absence of `@xc` leaves both
/// at 1. An explicit `@0c` sets the field to 0, then `if (ochannels == 0) { ochannels = 1; }`
/// normalizes it. `switch_loadable_module.c:2824-2826`.
///
/// This is the dedup key's rule only: `None` and `Some(0)` are the same duplicate key.
/// `CodecString::retain_available`'s implementation-matching channel check needs a
/// different rule (`Some(0)` alone is unconstrained) and does not call this.
fn norm_channels(e: &CodecStringEntry) -> u32 {
    match e.channels() {
        None | Some(0) => 1,
        Some(n) => n,
    }
}

impl CodecString {
    /// Parse a codec string in lenient mode.
    ///
    /// Behaves like the [`FromStr`] impl except that qualifier values that overflow
    /// `u32` or carry no recognised letter are recorded as
    /// [`SdpWarning::CodecStringQualifier`] in `warnings` rather than returning
    /// an error. The affected qualifier is omitted from the parsed entry.
    ///
    /// Use this when the input comes from a FreeSWITCH channel variable or any
    /// trusted internal source — FreeSWITCH itself logs these anomalies and
    /// continues. Use the [`FromStr`] impl (or `.parse()`) for policy strings you
    /// control; those must be strictly correct.
    pub fn parse_lenient(
        s: &str,
        warnings: &mut Vec<SdpWarning>,
    ) -> Result<Self, CodecStringError> {
        parse_codec_string_inner(s, Some(warnings))
    }

    /// Remove duplicate entries, porting `switch_loadable_module_get_codecs_sorted`'s
    /// dedup pass (`switch_loadable_module.c:2811-2847`).
    ///
    /// Two entries are duplicates when their name (case-insensitive), normalized ptime,
    /// normalized rate, normalized channels, and fmtp (case-insensitive, unset ≡ `""`)
    /// all match. Normalization: unset or `0` ptime → `default_ptime(name)`; unset or `0`
    /// rate → `default_rate(name)`; unset or `0` channels → `1`. Bitrate and module name
    /// are **not** compared — the C parser reads them but the dedup comparison ignores them.
    ///
    /// The first occurrence is kept at its original position; later duplicates are dropped.
    ///
    /// The inner loop compares entry `x` against all earlier *original* entries `0..x`,
    /// including ones already dropped. This coincides with comparing against survivors only
    /// because the dedup key is a pure function of each entry — if `x` duplicates `j` and
    /// `j` was dropped as a dup of `i`, then `x` also matches `i` directly. The C's loop
    /// shape is preserved verbatim to avoid divergence on any future edge case.
    ///
    /// **Invariant:** `dedup()` never removes the last entry bearing a given name. A codec
    /// specified only by name (no qualifiers) always survives, which makes the unknowable
    /// `switch.conf` `<default-ptimes>` extension safe: the bare-name entry gets the
    /// deployment-specific ptime at match time.
    ///
    /// **Limitation:** a deployment can override the per-codec default ptime via
    /// `switch.conf`'s `<default-ptimes>` (`switch_core.c:2061-2085`), and this crate has
    /// no way to know that override. A bare `G729` and an explicit `G729@20i` normalize to
    /// the same key here — they are only actually distinct on a switch configured for a
    /// different default (e.g. 40 ms), where the bare entry means 40 ms and the qualified
    /// one still means 20. `dedup()` collapses them regardless. The invariant above still
    /// holds: what disappears is always a qualified variant of a name, never the name
    /// itself.
    pub fn dedup(&mut self) {
        let input: Vec<CodecStringEntry> = std::mem::take(&mut self.0);
        let n = input.len();
        let mut keep = vec![true; n];

        for x in 1..n {
            let entry = &input[x];
            let o_ptime = norm_ptime(entry);
            let o_rate = norm_rate(entry);
            let o_channels = norm_channels(entry);
            let o_fmtp = entry
                .fmtp()
                .unwrap_or("");

            // Compare against ALL earlier original entries (including ones already
            // dropped as duplicates). See the doc comment for why this coincides with
            // comparing against survivors only.
            for earlier in &input[..x] {
                if entry
                    .name()
                    .eq_ignore_ascii_case(earlier.name())
                    && o_ptime == norm_ptime(earlier)
                    && o_rate == norm_rate(earlier)
                    && o_channels == norm_channels(earlier)
                    && o_fmtp.eq_ignore_ascii_case(
                        earlier
                            .fmtp()
                            .unwrap_or(""),
                    )
                {
                    keep[x] = false;
                    break;
                }
            }
        }

        self.0 = input
            .into_iter()
            .zip(keep)
            .filter_map(|(e, k)| k.then_some(e))
            .collect();
    }
}

impl FromStr for CodecString {
    type Err = CodecStringError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_codec_string_inner(s, None)
    }
}

impl IntoIterator for CodecString {
    type Item = CodecStringEntry;
    type IntoIter = std::vec::IntoIter<CodecStringEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.0
            .into_iter()
    }
}

impl<'a> IntoIterator for &'a CodecString {
    type Item = &'a CodecStringEntry;
    type IntoIter = std::slice::Iter<'a, CodecStringEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.0
            .iter()
    }
}

impl<'a> IntoIterator for &'a mut CodecString {
    type Item = &'a mut CodecStringEntry;
    type IntoIter = std::slice::IterMut<'a, CodecStringEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.0
            .iter_mut()
    }
}

impl FromIterator<CodecStringEntry> for CodecString {
    fn from_iter<I: IntoIterator<Item = CodecStringEntry>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .collect(),
        )
    }
}

impl Extend<CodecStringEntry> for CodecString {
    fn extend<I: IntoIterator<Item = CodecStringEntry>>(&mut self, iter: I) {
        self.0
            .extend(iter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extend_from_appends_entries() {
        let a: CodecString = "PCMU,PCMA"
            .parse()
            .unwrap();
        let b: CodecString = "G722"
            .parse()
            .unwrap();
        let mut c = a.clone();
        c.extend_from(&b);
        assert_eq!(c.len(), 3);
        assert_eq!(c.entries()[2].name(), "G722");
    }

    #[test]
    fn append_drains_source() {
        let mut a: CodecString = "PCMU"
            .parse()
            .unwrap();
        let mut b: CodecString = "PCMA"
            .parse()
            .unwrap();
        a.append(&mut b);
        assert_eq!(a.len(), 2);
        assert!(b.is_empty(), "source must be empty after append");
    }

    #[test]
    fn clear_empties_codec_string() {
        let mut cs: CodecString = "PCMU,PCMA"
            .parse()
            .unwrap();
        cs.clear();
        assert!(cs.is_empty());
    }

    #[test]
    fn retain_filters_entries() {
        let mut cs: CodecString = "PCMU,PCMA,G722"
            .parse()
            .unwrap();
        cs.retain(|e| e.name() != "PCMA");
        assert_eq!(cs.len(), 2);
        assert_eq!(cs.entries()[0].name(), "PCMU");
        assert_eq!(cs.entries()[1].name(), "G722");
    }

    #[test]
    fn iter_mut_modifies_entries() {
        let mut cs: CodecString = "PCMU@8000h"
            .parse()
            .unwrap();
        for e in cs.iter_mut() {
            *e.rate_mut() = Some(16000);
        }
        assert_eq!(cs.entries()[0].rate(), Some(16000));
    }

    #[test]
    fn insert_within_bounds() {
        let mut cs: CodecString = "PCMU,PCMA"
            .parse()
            .unwrap();
        let g722 = CodecStringEntry::new("G722").unwrap();
        let result = cs.insert(1, g722);
        assert!(result.is_ok());
        assert_eq!(cs.entries()[1].name(), "G722");
        assert_eq!(cs.len(), 3);
    }

    #[test]
    fn insert_out_of_bounds_returns_entry() {
        let mut cs: CodecString = "PCMU"
            .parse()
            .unwrap();
        let g722 = CodecStringEntry::new("G722").unwrap();
        let result = cs.insert(5, g722);
        assert!(result.is_err());
        assert_eq!(
            result
                .unwrap_err()
                .name(),
            "G722"
        );
    }

    #[test]
    fn remove_within_bounds() {
        let mut cs: CodecString = "PCMU,PCMA"
            .parse()
            .unwrap();
        let removed = cs.remove(0);
        assert_eq!(
            removed
                .as_ref()
                .map(|e| e.name()),
            Some("PCMU")
        );
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn remove_out_of_bounds_is_none() {
        let mut cs: CodecString = "PCMU"
            .parse()
            .unwrap();
        assert!(cs
            .remove(5)
            .is_none());
    }

    #[test]
    fn into_iterator_consuming() {
        let cs: CodecString = "PCMU,PCMA"
            .parse()
            .unwrap();
        let names: Vec<String> = cs
            .into_iter()
            .map(|e| {
                e.name()
                    .to_owned()
            })
            .collect();
        assert_eq!(names, vec!["PCMU", "PCMA"]);
    }

    #[test]
    fn into_iterator_ref() {
        let cs: CodecString = "PCMU,PCMA"
            .parse()
            .unwrap();
        let names: Vec<&str> = (&cs)
            .into_iter()
            .map(|e| e.name())
            .collect();
        assert_eq!(names, vec!["PCMU", "PCMA"]);
    }

    #[test]
    fn from_iterator_collects() {
        let entries = vec![
            CodecStringEntry::new("PCMU").unwrap(),
            CodecStringEntry::new("PCMA").unwrap(),
        ];
        let cs: CodecString = entries
            .into_iter()
            .collect();
        assert_eq!(cs.len(), 2);
        assert_eq!(cs.entries()[0].name(), "PCMU");
    }

    #[test]
    fn extend_trait_appends() {
        let mut cs = CodecString::new();
        cs.extend(vec![
            CodecStringEntry::new("PCMU").unwrap(),
            CodecStringEntry::new("PCMA").unwrap(),
        ]);
        assert_eq!(cs.len(), 2);
    }

    #[test]
    fn qualified_yields_entries_with_qualifiers() {
        let cs: CodecString = "PCMU,PCMU@8000h,AMR@8000h@40i"
            .parse()
            .unwrap();
        let qualified: Vec<&CodecStringEntry> = cs
            .qualified()
            .collect();
        assert_eq!(qualified.len(), 2);
        assert_eq!(qualified[0].name(), "PCMU");
        assert_eq!(qualified[1].name(), "AMR");
    }

    fn filled_to_cap() -> CodecString {
        (0..CodecString::MAX_SWITCH_ENTRIES)
            .map(|_| CodecStringEntry::new("PCMU").unwrap())
            .collect()
    }

    #[test]
    fn push_at_the_switch_cap_is_refused() {
        let mut cs = filled_to_cap();
        let err = cs
            .push(CodecStringEntry::new("PCMA").unwrap())
            .unwrap_err();
        assert!(matches!(err, CodecStringError::TooManyEntries { .. }));
        assert_eq!(cs.len(), CodecString::MAX_SWITCH_ENTRIES);
    }

    #[test]
    fn push_below_the_switch_cap_is_accepted() {
        let mut cs = CodecString::new();
        assert!(cs
            .push(CodecStringEntry::new("PCMU").unwrap())
            .is_ok());
    }

    #[test]
    fn strict_parse_past_the_switch_cap_is_an_error() {
        let s = vec!["PCMU"; CodecString::MAX_SWITCH_ENTRIES + 1].join(",");
        let err = s
            .parse::<CodecString>()
            .unwrap_err();
        assert!(matches!(err, CodecStringError::TooManyEntries { .. }));
    }

    #[test]
    fn lenient_parse_past_the_switch_cap_warns() {
        let s = vec!["PCMU"; CodecString::MAX_SWITCH_ENTRIES + 1].join(",");
        let mut warnings = Vec::new();
        let cs = CodecString::parse_lenient(&s, &mut warnings).unwrap();
        assert_eq!(cs.len(), CodecString::MAX_SWITCH_ENTRIES + 1);
        assert!(matches!(
            warnings[0],
            SdpWarning::CodecStringTruncated { .. }
        ));
    }

    #[test]
    fn empty_tokens_count_against_the_switch_cap() {
        // separate_string_char_delim assigns a slot per token, empty ones included.
        let s = format!("{},PCMU", ",".repeat(CodecString::MAX_SWITCH_ENTRIES - 1));
        let mut warnings = Vec::new();
        let cs = CodecString::parse_lenient(&s, &mut warnings).unwrap();
        assert_eq!(cs.len(), 1);
        assert!(matches!(
            warnings[0],
            SdpWarning::CodecStringTruncated { .. }
        ));
    }

    // --- Step 5: dedup() ---

    fn dedup(s: &str) -> CodecString {
        let mut cs: CodecString = s
            .parse()
            .unwrap();
        cs.dedup();
        cs
    }

    #[test]
    fn dedup_qualified_first_kept() {
        // PCMU@8000h@20i@64000b@1c normalizes to rate=8000,ptime=20,ch=1; same as bare PCMU.
        // Bitrate is not compared, so the qualified entry counts as a dup of the bare one.
        // First occurrence (the qualified entry) is kept.
        let cs = dedup("PCMU@8000h@20i@64000b@1c,PCMU");
        assert_eq!(cs.len(), 1);
        assert_eq!(cs.entries()[0].rate(), Some(8000));
    }

    #[test]
    fn dedup_bitrate_not_compared() {
        // switch_loadable_module.c:2843 compares name/ptime/rate/channels/fmtp only.
        let cs = dedup("PCMU@8000h@20i,PCMU@8000h@20i@64000b");
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn dedup_modname_not_compared() {
        let cs = dedup("mod_a.PCMU,mod_b.PCMU");
        assert_eq!(cs.len(), 1);
        assert_eq!(cs.entries()[0].modname(), Some("mod_a"));
    }

    #[test]
    fn dedup_fmtp_case_insensitive_dup() {
        let cs = dedup("AMR~octet-align=1,AMR~OCTET-ALIGN=1");
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn dedup_fmtp_set_vs_unset_is_not_dup() {
        let cs = dedup("AMR~octet-align=1,AMR");
        assert_eq!(cs.len(), 2);
    }

    #[test]
    fn dedup_amr_fmtp_trailing_space_is_dup() {
        // Built directly (not round-tripped through Display/FromStr), so this
        // exercises with_fmtp's own normalization, not split_codec_string's.
        let mut cs = CodecString::new();
        cs.push(
            CodecStringEntry::new("AMR")
                .unwrap()
                .with_fmtp("octet-align=1")
                .unwrap(),
        );
        cs.push(
            CodecStringEntry::new("AMR")
                .unwrap()
                .with_fmtp("octet-align=1 ")
                .unwrap(),
        );
        cs.dedup();
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn dedup_ilbc_ptime_default_30() {
        // iLBC defaults to 30 ms; iLBC and iLBC@30i are duplicates, iLBC@20i is distinct.
        let cs_same = dedup("iLBC,iLBC@30i");
        assert_eq!(cs_same.len(), 1);
        let cs_diff = dedup("iLBC,iLBC@20i");
        assert_eq!(cs_diff.len(), 2);
    }

    #[test]
    fn dedup_isac_ptime_default_30() {
        let cs_same = dedup("isac,isac@30i");
        assert_eq!(cs_same.len(), 1);
        let cs_diff = dedup("isac,isac@20i");
        assert_eq!(cs_diff.len(), 2);
    }

    #[test]
    fn dedup_g723_ptime_default_30() {
        let cs_same = dedup("G723,G723@30i");
        assert_eq!(cs_same.len(), 1);
        let cs_diff = dedup("G723,G723@20i");
        assert_eq!(cs_diff.len(), 2);
    }

    #[test]
    fn dedup_opus_rate_48k() {
        let cs_same = dedup("opus,opus@48000h");
        assert_eq!(cs_same.len(), 1);
        let cs_diff = dedup("opus,opus@8000h");
        assert_eq!(cs_diff.len(), 2);
    }

    #[test]
    fn dedup_h264_rate_90k() {
        let cs_same = dedup("H264,H264@90000h");
        assert_eq!(cs_same.len(), 1);
    }

    #[test]
    fn dedup_h263_rate_90k() {
        let cs_same = dedup("h263,h263@90000h");
        assert_eq!(cs_same.len(), 1);
    }

    #[test]
    fn dedup_vp8_rate_90k() {
        let cs_same = dedup("VP8,VP8@90000h");
        assert_eq!(cs_same.len(), 1);
    }

    #[test]
    fn dedup_case_insensitive_name() {
        let cs = dedup("PCMU,pcmu");
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn dedup_g722_rate_is_8000_not_16000() {
        // default_rate("G722") = 8000 per switch_default_rate; so G722 and G722@8000h are dups.
        let cs_same = dedup("G722,G722@8000h");
        assert_eq!(cs_same.len(), 1);
    }

    #[test]
    fn dedup_channels_0_normalizes_to_1() {
        let cs = dedup("PCMU,PCMU@1c");
        assert_eq!(cs.len(), 1);
        let cs2 = dedup("PCMU,PCMU@0c");
        assert_eq!(cs2.len(), 1);
    }

    #[test]
    fn dedup_nondefault_channels_is_not_dup() {
        // opus@2c has channels=2; bare opus normalizes to 1. Distinct.
        let cs = dedup("opus@2c,opus");
        assert_eq!(cs.len(), 2);
    }

    #[test]
    fn dedup_t38_default_rate_8k() {
        let cs = dedup("t38,t38@8000h@20i");
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn dedup_preserves_position_of_first_occurrence() {
        // PCMU appears at positions 0 and 2; position 2 is the dup. PCMA at 1 stays.
        let cs = dedup("PCMU,PCMA,PCMU");
        assert_eq!(cs.len(), 2);
        assert_eq!(cs.entries()[0].name(), "PCMU");
        assert_eq!(cs.entries()[1].name(), "PCMA");
    }

    #[test]
    fn dedup_idempotent() {
        let mut cs: CodecString = "PCMU,PCMA,G722"
            .parse()
            .unwrap();
        cs.dedup();
        let after_first = cs.clone();
        cs.dedup();
        assert_eq!(cs, after_first);
    }

    #[test]
    fn dedup_algorithm_shape_comment() {
        // A@20i, A, A@20i: the C inner loop compares against ALL earlier original entries,
        // not just survivors. Entry 2 (A@20i) is compared against entry 0 (A@20i) and
        // matches, so it's dropped. Comparing against survivors only would give the same
        // result here (entry 0 survived), which is why the C's shape coincides.
        let cs = dedup("A@20i,A,A@20i");
        // Entry 1 (bare A) has ptime=default_ptime("A")=20, matching entry 0 (A@20i). → 1 entry.
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn dedup_invariant_no_name_disappears() {
        // Every name present in the input must appear in the output.
        let cs = dedup("PCMU,PCMA,G722,PCMU,PCMA");
        let out_names: std::collections::HashSet<&str> = cs
            .entries()
            .iter()
            .map(|e| e.name())
            .collect();
        for name in &["PCMU", "PCMA", "G722"] {
            assert!(
                out_names
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(name)),
                "{name} missing from dedup output"
            );
        }
    }

    // --- real-world multi-entry round-trip ---

    #[test]
    fn real_world_codec_string_roundtrip() {
        let s = "opus@16000h@20i,opus@48000h@20i,G722,PCMU,PCMA,AMR-WB,AMR";
        let cs: CodecString = s
            .parse()
            .unwrap();
        assert_eq!(cs.len(), 7);
        assert_eq!(cs.entries()[0].name(), "opus");
        assert_eq!(cs.entries()[0].rate(), Some(16000));
        assert_eq!(cs.entries()[0].ptime(), Some(20));
        assert_eq!(cs.entries()[1].name(), "opus");
        assert_eq!(cs.entries()[1].rate(), Some(48000));
        assert_eq!(cs.entries()[2].name(), "G722");
        assert_eq!(cs.entries()[3].name(), "PCMU");
        assert_eq!(cs.entries()[4].name(), "PCMA");
        assert_eq!(cs.entries()[5].name(), "AMR-WB");
        assert_eq!(cs.entries()[6].name(), "AMR");
        // Round-trip: Display output re-parses to the same entries.
        let displayed = cs.to_string();
        let cs2: CodecString = displayed
            .parse()
            .unwrap();
        assert_eq!(cs, cs2);
    }
}
