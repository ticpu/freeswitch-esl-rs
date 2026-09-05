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
    ///
    /// Returns [`CodecStringError::TooManyEntries`] when the list already holds
    /// [`MAX_SWITCH_ENTRIES`](Self::MAX_SWITCH_ENTRIES); the entry is not appended.
    /// The bulk operations ([`Extend`], [`FromIterator`], [`extend_from`](Self::extend_from),
    /// [`append`](Self::append), [`insert`](Self::insert)) do not check the cap.
    pub fn push(&mut self, entry: CodecStringEntry) -> Result<(), CodecStringError> {
        if self
            .0
            .len()
            >= Self::MAX_SWITCH_ENTRIES
        {
            return Err(CodecStringError::too_many_entries(
                self.0
                    .len()
                    + 1,
                Self::MAX_SWITCH_ENTRIES,
            ));
        }
        self.0
            .push(entry);
        Ok(())
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
    /// produce — not to the number of parsed entries. Enforced by
    /// [`push`](Self::push) and by both parse modes; see `docs/codec-string-format.md`
    /// for what the switch does with the 50th slot and everything after it.
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

/// Normalize ptime for the dedup key: `None` or `0` → `default_ptime(name)`.
///
/// `switch_loadable_module.c:2816-2818`; this is the dedup rule only, one of the
/// three in `docs/codec-string-format.md`.
fn norm_ptime(e: &CodecStringEntry) -> u32 {
    match e.ptime() {
        None | Some(0) => default_ptime(e.name()),
        Some(n) => n,
    }
}

/// Normalize rate for the dedup key: `None` or `0` → `default_rate(name)`.
///
/// `switch_loadable_module.c:2820-2822`.
fn norm_rate(e: &CodecStringEntry) -> u32 {
    match e.rate() {
        None | Some(0) => default_rate(e.name()),
        Some(n) => n,
    }
}

/// Normalize channels for the dedup key: `None` or `0` → `1`.
///
/// `switch_loadable_module.c:2824-2826`. Implementation matching needs the other
/// rule — absent constrains, explicit `0` does not — and does not call this.
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
    /// an error. The affected qualifier is omitted from the parsed entry. A slot
    /// count over [`MAX_SWITCH_ENTRIES`](Self::MAX_SWITCH_ENTRIES) records
    /// [`SdpWarning::CodecStringTruncated`] and every entry is still returned.
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
    /// The key is the name, the normalized ptime, rate and channels, and the fmtp, all
    /// case-insensitive; bitrate and module name are read by the C parser and then not
    /// compared. The first occurrence keeps its position. The loop shape, the
    /// normalization rules and what a deployment's `<default-ptimes>` does to them are
    /// in `docs/codec-string-format.md`.
    ///
    /// **Invariant:** the last entry bearing a name is never removed, so a bare name
    /// survives to take the deployment's own default at match time.
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

            // Earlier *originals*, dropped ones included, as the C loop does.
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

    // --- dedup() ---

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
        )
        .unwrap();
        cs.push(
            CodecStringEntry::new("AMR")
                .unwrap()
                .with_fmtp("octet-align=1 ")
                .unwrap(),
        )
        .unwrap();
        cs.dedup();
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn a_bare_name_and_its_own_defaults_are_one_entry() {
        // The qualifier the default table already implies adds nothing to the key.
        for input in &[
            "iLBC,iLBC@30i",
            "isac,isac@30i",
            "G723,G723@30i",
            "opus,opus@48000h",
            "H264,H264@90000h",
            "h263,h263@90000h",
            "VP8,VP8@90000h",
            // switch_default_rate answers 8000 for G.722, not its 16 kHz clock.
            "G722,G722@8000h",
            "t38,t38@8000h@20i",
            "PCMU,pcmu",
            "PCMU,PCMU@1c",
            // Channels 0 normalizes to 1 for the key, as the C does.
            "PCMU,PCMU@0c",
        ] {
            assert_eq!(dedup(input).len(), 1, "{input:?} must collapse");
        }
    }

    #[test]
    fn a_qualifier_that_is_not_the_default_stays_a_second_entry() {
        for input in &[
            "iLBC,iLBC@20i",
            "isac,isac@20i",
            "G723,G723@20i",
            "opus,opus@8000h",
            "opus@2c,opus",
        ] {
            assert_eq!(dedup(input).len(), 2, "{input:?} must stay two entries");
        }
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
    fn a_duplicate_of_a_dropped_entry_is_dropped_too() {
        // The C compares against earlier originals, dropped ones included. Bare A
        // normalizes to A@20i, so all three collapse onto the first.
        let cs = dedup("A@20i,A,A@20i");
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
