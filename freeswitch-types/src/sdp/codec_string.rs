//! FreeSWITCH codec-string grammar: `[modname.]name[~fmtp][@rate h][@ptime i][@bitrate b][@channels c]`.
//!
//! See `docs/codec-string-format.md` for the full grammar, parse-order hazards,
//! and fmtp delimiter collisions. This module provides typed construction and
//! round-trip-safe serialisation.

use std::fmt;
use std::str::FromStr;

use crate::sdp::error::{CodecStringError, SdpWarning};
use crate::sdp::static_payload::{default_ptime, default_rate};

/// Validate a codec name or module name for grammar-delimiter characters.
///
/// Names are emitted unescaped (unlike fmtp which goes through `escape_fmtp`), so
/// grammar delimiters in a name corrupt the re-parsed form. `\n`/`\r` are handled
/// separately as `WireInjection`; this checks the remaining forbidden set.
///
/// Returns the first forbidden character found, or `None` if clean.
fn check_name_grammar(value: &str) -> Option<char> {
    value
        .chars()
        .find(|&c| matches!(c, ',' | '@' | '~' | '.' | '\'' | '\\') || c.is_ascii_whitespace())
}

/// A single entry in a FreeSWITCH codec string.
///
/// All fields are private. Use [`CodecStringEntry::new`] plus the `with_*` builder
/// methods to construct, and the read accessors plus [`Display`](fmt::Display) to
/// inspect and emit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecStringEntry {
    modname: Option<String>,
    name: String,
    fmtp: Option<String>,
    rate: Option<u32>,
    ptime: Option<u32>,
    bitrate: Option<u32>,
    channels: Option<u32>,
}

impl CodecStringEntry {
    /// Create a new entry with the given codec name.
    ///
    /// Returns [`CodecStringError::InvalidCodecName`] if the name is empty,
    /// [`CodecStringError::WireInjection`] if it contains `\n` or `\r`, or
    /// [`CodecStringError::InvalidCharInName`] if it contains any codec-string
    /// grammar delimiter (`,` `@` `~` `.` `'` `\` or ASCII whitespace).
    pub fn new(name: impl Into<String>) -> Result<Self, CodecStringError> {
        let name = name.into();
        if name.is_empty() {
            return Err(CodecStringError::invalid_codec_name(name));
        }
        if name.contains('\n') || name.contains('\r') {
            return Err(CodecStringError::wire_injection("name", name));
        }
        if let Some(ch) = check_name_grammar(&name) {
            return Err(CodecStringError::invalid_char_in_name("name", ch, name));
        }
        Ok(Self {
            modname: None,
            name,
            fmtp: None,
            rate: None,
            ptime: None,
            bitrate: None,
            channels: None,
        })
    }

    /// Set the module prefix (e.g. `"mod_opus"`).
    ///
    /// Returns [`CodecStringError::WireInjection`] if the value contains `\n` or `\r`,
    /// or [`CodecStringError::InvalidCharInName`] if it contains a grammar delimiter.
    pub fn with_module(mut self, modname: impl Into<String>) -> Result<Self, CodecStringError> {
        let modname = modname.into();
        if modname.contains('\n') || modname.contains('\r') {
            return Err(CodecStringError::wire_injection("modname", modname));
        }
        if let Some(ch) = check_name_grammar(&modname) {
            return Err(CodecStringError::invalid_char_in_name(
                "modname", ch, modname,
            ));
        }
        self.modname = Some(modname);
        Ok(self)
    }

    /// Set format parameters (`~fmtp` in the grammar).
    ///
    /// # Errors
    ///
    /// - [`CodecStringError::AtInFmtp`] — `@` is unrepresentable in fmtp (the
    ///   `@` split precedes the `~` split in FreeSWITCH's parser).
    /// - [`CodecStringError::DotInFmtpWithoutModule`] — a `.` in fmtp without a
    ///   module prefix becomes the module-name boundary; the codec is silently
    ///   dropped. Set the module prefix first with [`with_module`](Self::with_module).
    /// - [`CodecStringError::WireInjection`] — `\n` or `\r` can inject ESL commands.
    pub fn with_fmtp(mut self, fmtp: impl Into<String>) -> Result<Self, CodecStringError> {
        let fmtp = fmtp.into();
        if fmtp.contains('\n') || fmtp.contains('\r') {
            return Err(CodecStringError::wire_injection("fmtp", &fmtp));
        }
        if fmtp.contains('@') {
            return Err(CodecStringError::at_in_fmtp(fmtp));
        }
        if fmtp.contains('.')
            && self
                .modname
                .is_none()
        {
            return Err(CodecStringError::dot_in_fmtp_without_module(fmtp));
        }
        self.fmtp = Some(fmtp);
        Ok(self)
    }

    /// Set the sample rate qualifier (`@<n>h`).
    pub fn with_rate(mut self, rate: u32) -> Self {
        self.rate = Some(rate);
        self
    }

    /// Set the ptime qualifier (`@<n>i`).
    pub fn with_ptime(mut self, ptime: u32) -> Self {
        self.ptime = Some(ptime);
        self
    }

    /// Set the bitrate qualifier (`@<n>b`).
    pub fn with_bitrate(mut self, bitrate: u32) -> Self {
        self.bitrate = Some(bitrate);
        self
    }

    /// Set the channel count qualifier (`@<n>c`).
    ///
    /// FreeSWITCH reads this as `atoi` into a `uint32_t`; the field is `u32` here
    /// to match. [`SdpCodec::channels`](super::codec::SdpCodec::channels) stays `Option<u8>` (from `a=rtpmap`, realistically tiny)
    /// and widens losslessly at the merge boundary.
    pub fn with_channels(mut self, channels: u32) -> Self {
        self.channels = Some(channels);
        self
    }

    // --- read accessors ---

    /// The module prefix, if any (e.g. `"mod_opus"`).
    pub fn modname(&self) -> Option<&str> {
        self.modname
            .as_deref()
    }

    /// The codec encoding name (e.g. `"PCMU"`, `"opus"`, `"AMR-WB"`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The format parameters, if any.
    pub fn fmtp(&self) -> Option<&str> {
        self.fmtp
            .as_deref()
    }

    /// The sample rate qualifier, if any.
    pub fn rate(&self) -> Option<u32> {
        self.rate
    }

    /// Mutable access to the rate qualifier.
    pub fn rate_mut(&mut self) -> &mut Option<u32> {
        &mut self.rate
    }

    /// The ptime qualifier in milliseconds, if any.
    pub fn ptime(&self) -> Option<u32> {
        self.ptime
    }

    /// Mutable access to the ptime qualifier.
    pub fn ptime_mut(&mut self) -> &mut Option<u32> {
        &mut self.ptime
    }

    /// The bitrate qualifier in bits/s, if any.
    pub fn bitrate(&self) -> Option<u32> {
        self.bitrate
    }

    /// Mutable access to the bitrate qualifier.
    pub fn bitrate_mut(&mut self) -> &mut Option<u32> {
        &mut self.bitrate
    }

    /// The channel count qualifier, if any.
    pub fn channels(&self) -> Option<u32> {
        self.channels
    }

    /// Mutable access to the channel count qualifier.
    pub fn channels_mut(&mut self) -> &mut Option<u32> {
        &mut self.channels
    }

    // --- fallible setters ---
    //
    // `CodecStringEntry` is not serde-deserializable, so fallible setters are
    // the right shape for validated fields — the `_mut()` convention applies
    // only to unvalidated numeric qualifiers where no invariant can be violated.

    /// Set or replace the codec encoding name.
    ///
    /// Returns [`CodecStringError::InvalidCodecName`] if empty,
    /// [`CodecStringError::WireInjection`] if the value contains `\n` or `\r`, or
    /// [`CodecStringError::InvalidCharInName`] if it contains a grammar delimiter.
    pub fn set_name(&mut self, name: impl Into<String>) -> Result<(), CodecStringError> {
        let name = name.into();
        if name.is_empty() {
            return Err(CodecStringError::invalid_codec_name(name));
        }
        if name.contains('\n') || name.contains('\r') {
            return Err(CodecStringError::wire_injection("name", name));
        }
        if let Some(ch) = check_name_grammar(&name) {
            return Err(CodecStringError::invalid_char_in_name("name", ch, name));
        }
        self.name = name;
        Ok(())
    }

    /// Set or replace the module prefix.
    ///
    /// Returns [`CodecStringError::WireInjection`] if the value contains `\n` or `\r`,
    /// or [`CodecStringError::InvalidCharInName`] if it contains a grammar delimiter.
    pub fn set_module(&mut self, modname: impl Into<String>) -> Result<(), CodecStringError> {
        let modname = modname.into();
        if modname.contains('\n') || modname.contains('\r') {
            return Err(CodecStringError::wire_injection("modname", modname));
        }
        if let Some(ch) = check_name_grammar(&modname) {
            return Err(CodecStringError::invalid_char_in_name(
                "modname", ch, modname,
            ));
        }
        self.modname = Some(modname);
        Ok(())
    }

    /// Set or replace the format parameters (`~fmtp`).
    ///
    /// Returns the same errors as [`with_fmtp`](Self::with_fmtp).
    pub fn set_fmtp(&mut self, fmtp: impl Into<String>) -> Result<(), CodecStringError> {
        let fmtp = fmtp.into();
        if fmtp.contains('\n') || fmtp.contains('\r') {
            return Err(CodecStringError::wire_injection("fmtp", &fmtp));
        }
        if fmtp.contains('@') {
            return Err(CodecStringError::at_in_fmtp(fmtp));
        }
        if fmtp.contains('.')
            && self
                .modname
                .is_none()
        {
            return Err(CodecStringError::dot_in_fmtp_without_module(fmtp));
        }
        self.fmtp = Some(fmtp);
        Ok(())
    }

    /// Clear the format parameters.
    pub fn clear_fmtp(&mut self) {
        self.fmtp = None;
    }

    /// Clear the module prefix.
    ///
    /// Returns [`CodecStringError::DotInFmtpWithoutModule`] if the current fmtp
    /// contains a `.` — removing the prefix would make the entry re-parse with the
    /// dot as the module boundary, silently dropping the codec. The entry is left
    /// unchanged on error.
    pub fn clear_module(&mut self) -> Result<(), CodecStringError> {
        if let Some(ref f) = self.fmtp {
            if f.contains('.') {
                return Err(CodecStringError::dot_in_fmtp_without_module(f.clone()));
            }
        }
        self.modname = None;
        Ok(())
    }

    /// Clear the rate qualifier.
    ///
    /// Infallible analog to `clear_fmtp`; use to strip a qualifier that matched no
    /// loaded implementation (see [`CodecString::qualified`]).
    pub fn clear_rate(&mut self) {
        self.rate = None;
    }

    /// Clear the ptime qualifier.
    ///
    /// Infallible analog to `clear_fmtp`; use to strip a qualifier that matched no
    /// loaded implementation (see [`CodecString::qualified`]).
    pub fn clear_ptime(&mut self) {
        self.ptime = None;
    }

    /// Clear the bitrate qualifier.
    ///
    /// Infallible analog to `clear_fmtp`; use to strip a qualifier that matched no
    /// loaded implementation (see [`CodecString::qualified`]).
    pub fn clear_bitrate(&mut self) {
        self.bitrate = None;
    }

    /// Clear the channel count qualifier.
    ///
    /// Infallible analog to `clear_fmtp`; use to strip a qualifier that matched no
    /// loaded implementation (see [`CodecString::qualified`]).
    pub fn clear_channels(&mut self) {
        self.channels = None;
    }

    /// Drop qualifiers whose value already equals the FreeSWITCH default for this codec.
    ///
    /// Applies `default_rate`/`default_ptime` (ported from `switch_core.c:2033-2055`).
    /// Channels `1` is the C init value and is always the default. Bitrate has no default
    /// function and is left unchanged.
    ///
    /// **Precondition: only safe on a qualifier that already matched a loaded
    /// implementation** — e.g. an entry that survived
    /// [`CodecString::retain_available`]. `simplify()` compares against this crate's
    /// per-name `default_rate`/`default_ptime` table, not against what a real
    /// implementation registered, so an unmatched qualifier can *start* matching after
    /// stripping. `AMR-WB@8000h` matches no implementation (both of
    /// `switch_loadable_module_get_codecs_sorted`'s passes compare the explicit `8000`
    /// against `AMR-WB`'s actual rate of 16000) and the switch silently drops it — but
    /// `default_rate("AMR-WB")` is 8000, so `simplify()` strips the rate, and bare
    /// `AMR-WB` then matches (an unconstrained rate falls through to whichever
    /// implementation is first). Any codec whose real rate differs from what
    /// `default_rate` returns for its name has this shape.
    ///
    /// **G.722 is exempt.** FreeSWITCH registers G.722 with `samples_per_second = 8000`
    /// and `actual_samples_per_second = 16000` (`mod_spandsp_codecs.c`); an explicit
    /// `@8000h` therefore only ever matches via the second pass
    /// (`switch_loadable_module.c:2885-2909`), which applies no default-ptime
    /// preference. Stripping `@8000h@20i` from `G722@8000h@20i` leaves no rate and no
    /// ptime, so the second pass takes whichever implementation is first in
    /// registration order rather than the 20 ms one — silently changing which
    /// implementation is selected.
    pub fn simplify(&mut self) {
        if self
            .name
            .eq_ignore_ascii_case("g722")
        {
            return;
        }
        if self.rate == Some(default_rate(&self.name)) {
            self.rate = None;
        }
        if self.ptime == Some(default_ptime(&self.name)) {
            self.ptime = None;
        }
        if self.channels == Some(1) {
            self.channels = None;
        }
    }
}

impl fmt::Display for CodecStringEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref m) = self.modname {
            write!(f, "{m}.")?;
        }
        write!(f, "{}", self.name)?;
        if let Some(ref fmtp) = self.fmtp {
            write!(f, "~{}", escape_fmtp(fmtp))?;
        }
        if let Some(r) = self.rate {
            write!(f, "@{r}h")?;
        }
        if let Some(p) = self.ptime {
            write!(f, "@{p}i")?;
        }
        if let Some(b) = self.bitrate {
            write!(f, "@{b}b")?;
        }
        if let Some(c) = self.channels {
            write!(f, "@{c}c")?;
        }
        Ok(())
    }
}

impl FromStr for CodecStringEntry {
    type Err = CodecStringError;

    /// Parse a single codec-string entry from a string.
    ///
    /// Reuses the same escape/quote-aware split as [`CodecString`]'s parser, then
    /// delegates to the same single-entry parser. A top-level (unescaped) comma is
    /// [`CodecStringError::MultipleEntries`], not a silent take-first.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let tokens: Vec<String> = split_codec_string(s)
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect();
        match tokens.len() {
            0 => Err(CodecStringError::invalid_codec_name(s)),
            1 => parse_entry(&tokens[0], None),
            _ => Err(CodecStringError::multiple_entries(s)),
        }
    }
}

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

    /// Consume the list, returning the inner vector.
    pub fn into_entries(self) -> Vec<CodecStringEntry> {
        self.0
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

    /// `true` if any entry's name equals `name` (case-insensitive).
    ///
    /// FreeSWITCH's codec hash is case-insensitive (`switch_core_hash_init_nocase`,
    /// `switch_loadable_module.c:2554`).
    pub fn contains_name(&self, name: &str) -> bool {
        self.0
            .iter()
            .any(|e| {
                e.name()
                    .eq_ignore_ascii_case(name)
            })
    }

    /// Iterator over codec names in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .map(|e| e.name())
    }

    /// Iterator over entries that carry at least one explicit qualifier.
    ///
    /// FreeSWITCH silently drops entries whose qualifiers match no loaded
    /// implementation — `mod_amr.c:690-712` registers AMR at 20 ms only, so
    /// `AMR@8000h@40i` matches nothing and vanishes unlogged. Qualified entries
    /// are the ones at risk of this silent drop.
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
    /// The cap applies to raw token slots — including empty ones from consecutive
    /// commas — not to the number of parsed entries. Use [`raw_token_count`](Self::raw_token_count)
    /// on the original string to compare against this limit accurately.
    pub const MAX_SWITCH_ENTRIES: usize = 50;

    /// How many entries this list has beyond [`MAX_SWITCH_ENTRIES`](Self::MAX_SWITCH_ENTRIES).
    ///
    /// FreeSWITCH's codec array (`switch_types.h:595`) silently ignores entries beyond
    /// position 50. Returns 0 when within the limit.
    pub fn excess_count(&self) -> usize {
        self.0
            .len()
            .saturating_sub(Self::MAX_SWITCH_ENTRIES)
    }

    /// Count the token slots a raw codec string would consume in FreeSWITCH's
    /// `separate_string_char_delim` (`switch_utils.c:2765-2771`).
    ///
    /// Unlike `len()` on a parsed [`CodecString`], this counts empty slots from
    /// consecutive commas: `"PCMU,,PCMA"` → 3, not 2. Compare against
    /// [`MAX_SWITCH_ENTRIES`](Self::MAX_SWITCH_ENTRIES) to detect silent truncation.
    pub fn raw_token_count(s: &str) -> usize {
        // Port of the START/FIND_DELIM state machine, not a delimiter tally: C
        // allocates a slot only on *entering* START with a non-null char, so a
        // trailing or doubled comma doesn't necessarily mean an extra slot.
        enum State {
            Start,
            FindDelim,
        }

        let chars: Vec<char> = s
            .chars()
            .collect();
        let mut state = State::Start;
        let mut count = 0usize;
        let mut inside_quotes = false;
        let mut i = 0usize;
        while i < chars.len() {
            match state {
                State::Start => {
                    count += 1;
                    state = State::FindDelim;
                    // No advance: C re-enters the switch on the same char in FIND_DELIM.
                }
                State::FindDelim => {
                    if chars[i] == '\\' {
                        // Escaped char: the C ESCAPE_META branch advances once here,
                        // then the shared trailing advance below skips the escapee too.
                        i += 1;
                    } else if chars[i] == '\'' && (inside_quotes || chars[i + 1..].contains(&'\''))
                    {
                        inside_quotes = !inside_quotes;
                    } else if chars[i] == ',' && !inside_quotes {
                        state = State::Start;
                    }
                    i += 1;
                }
            }
        }
        count
    }
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

/// Inner parser shared by [`FromStr`] (strict, `warnings = None`) and
/// [`CodecString::parse_lenient`] (lenient, `warnings = Some`).
fn parse_codec_string_inner(
    s: &str,
    mut warnings: Option<&mut Vec<SdpWarning>>,
) -> Result<CodecString, CodecStringError> {
    let mut entries = Vec::new();
    for token in split_codec_string(s) {
        if token.is_empty() {
            continue;
        }
        entries.push(parse_entry(&token, warnings.as_deref_mut())?);
    }
    Ok(CodecString(entries))
}

/// Escape `,` `\` `'` in an fmtp value for safe embedding in a codec string.
///
/// A raw comma splits entries; a lone `'` or `\` has grammar significance in the
/// surrounding separator layer (`cleanup_separated_string`, `switch_utils.c:2702`).
fn escape_fmtp(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            ',' => out.push_str("\\,"),
            c => out.push(c),
        }
    }
    out
}

/// Split a codec string on `,` and apply `cleanup_separated_string` to each token.
///
/// Faithfully ports `separate_string_char_delim` + `cleanup_separated_string` from
/// `switch_utils.c`. For the split step, `\` before `,` prevents splitting and `'`
/// quote-toggling keeps the current token intact through a comma inside quotes.
/// Then for each token, leading spaces are stripped, trailing spaces (outside quotes)
/// are dropped, `'` is toggled (and stripped from output), and escape sequences are
/// expanded: `\'`→`'`, `\"`→`"`, `\,`→`,`, `\\`→`\`, `\n`→LF, `\r`→CR,
/// `\t`→TAB, `\s`→space; any other `\X` passes through as `\X`.
fn split_codec_string(s: &str) -> Vec<String> {
    // Step 1: split on ',' honouring escape and quote, mirroring separate_string_char_delim.
    let mut raw_tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut inside_quotes = false;
    let mut chars = s
        .chars()
        .peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // Escaped char: copy backslash + next char verbatim into the raw token
            // so cleanup_token can process the escape. Only skip the next char for
            // split-prevention purposes (we don't split inside \X).
            if let Some(&next) = chars.peek() {
                chars.next();
                current.push('\\');
                current.push(next);
            } else {
                current.push('\\');
            }
        } else if ch == '\'' {
            // Quote toggle — affects split point but is stripped by cleanup_token.
            // Only toggle when there's a matching closing quote ahead (mirrors C
            // `strchr(ptr+1, '\'')` check), OR we're already inside quotes.
            if inside_quotes
                || chars
                    .clone()
                    .any(|c| c == '\'')
            {
                inside_quotes = !inside_quotes;
            }
            current.push('\'');
        } else if ch == ',' && !inside_quotes {
            raw_tokens.push(current.clone());
            current.clear();
        } else {
            current.push(ch);
        }
    }
    raw_tokens.push(current);

    // Step 2: apply cleanup_token to each raw token.
    raw_tokens
        .into_iter()
        .map(|t| cleanup_token(&t))
        .collect()
}

/// Apply `cleanup_separated_string` logic to a single raw token.
///
/// - Strips leading spaces (only space, not other whitespace — mirrors the C `' '` check).
/// - Strips trailing spaces outside quotes (via `end` pointer tracking).
/// - Strips `'` quote characters (they are not included in output).
/// - Expands escape sequences.
fn cleanup_token(raw: &str) -> String {
    let mut out = String::new();
    // `end_len` tracks the length of `out` at the last non-trailing-space position.
    let mut end_len: usize = 0;
    let mut inside_quotes = false;

    // Skip leading spaces (C: `for (ptr = str; *ptr == ' '; ++ptr)`).
    let s = raw.trim_start_matches(' ');

    let mut chars = s
        .chars()
        .peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // Expand escape: '\'', '"', ',', '\\' are direct; unescape_char handles n/r/t/s.
            let expanded = chars
                .peek()
                .and_then(|&next| match next {
                    '\'' | '"' | ',' | '\\' => Some(next),
                    'n' => Some('\n'),
                    'r' => Some('\r'),
                    't' => Some('\t'),
                    's' => Some(' '),
                    _ => None,
                });
            match expanded {
                Some(e) => {
                    chars.next();
                    out.push(e);
                    end_len = out.len();
                }
                // Unrecognized escape (and a trailing lone backslash): upstream leaves the
                // next char unconsumed and reprocesses it, so a following space still trims.
                None => {
                    out.push('\\');
                    end_len = out.len();
                }
            }
        } else if ch == '\'' {
            // Toggle quote state. C only toggles when inside_quotes is already set
            // OR there is a matching closing ' ahead. We track it the same way we
            // did in the split step.
            let has_closing = chars
                .clone()
                .any(|c| c == '\'');
            if inside_quotes || has_closing {
                inside_quotes = !inside_quotes;
                // Quote char is NOT output; only update end_len when entering quotes.
                if inside_quotes {
                    end_len = out.len();
                }
            } else {
                // No matching close quote: output the quote literally.
                out.push('\'');
                end_len = out.len();
            }
        } else {
            out.push(ch);
            // Update end tracker when the char is not a trailing space.
            if ch != ' ' || inside_quotes {
                end_len = out.len();
            }
        }
    }

    // Truncate to end_len to strip trailing spaces.
    out.truncate(end_len);
    out
}

/// Parse one codec-string entry token (after comma-splitting and unescaping).
///
/// `warnings = None` → strict: qualifier errors are returned as `Err`.
/// `warnings = Some(acc)` → lenient: qualifier errors are pushed to `acc` and the
/// qualifier is omitted from the entry.
fn parse_entry(
    token: &str,
    mut warnings: Option<&mut Vec<SdpWarning>>,
) -> Result<CodecStringEntry, CodecStringError> {
    // Step 1: split on `@` — name segment is everything before the first `@`.
    let (name_seg, qualifier_str) = match token.split_once('@') {
        Some((n, q)) => (n, q),
        None => (token, ""),
    };

    // Step 2: classify each `@`-delimited qualifier part.
    let qualifiers: Vec<&str> = if qualifier_str.is_empty() {
        Vec::new()
    } else {
        qualifier_str
            .split('@')
            .collect()
    };

    // Steps 3+4: split name_seg on first `.` for modname, then first `~` for fmtp.
    let (modname, name_and_fmtp) = match name_seg.split_once('.') {
        Some((m, rest)) => (Some(m.to_string()), rest),
        None => (None, name_seg),
    };

    let (name, fmtp_raw) = match name_and_fmtp.split_once('~') {
        Some((n, f)) => (n.to_string(), Some(f.to_string())),
        None => (name_and_fmtp.to_string(), None),
    };

    // Route through the validated builders so every construction path enforces
    // the same invariants (newline rejection, fmtp delimiter checks).
    let mut entry = CodecStringEntry::new(name)?;
    if let Some(m) = modname {
        entry = entry.with_module(m)?;
    }
    if let Some(f) = fmtp_raw {
        entry = entry.with_fmtp(f)?;
    }

    // Classify each qualifier part by substring scan — order: i, k/h, b, c.
    // Unknown letter or overflow: strict (None) = Err, lenient (Some) = warn + skip.
    for part in &qualifiers {
        let letter_found = part.contains('i')
            || part.contains('h')
            || part.contains('k')
            || part.contains('b')
            || part.contains('c');

        if !letter_found {
            let reason = format!("no recognised qualifier letter in {part:?}");
            match warnings.as_deref_mut() {
                None => {
                    return Err(CodecStringError::qualifier_parse_error(
                        part.to_string(),
                        reason,
                    ));
                }
                Some(acc) => {
                    acc.push(SdpWarning::codec_string_qualifier(part.to_string(), reason));
                    continue;
                }
            }
        }

        // Helper that returns None on overflow (no leading digits is also None).
        let parsed = atoi_prefix(part);

        if part.contains('i') {
            if let Some(v) = apply_qualifier(part, parsed, warnings.as_deref_mut())? {
                entry = entry.with_ptime(v)
            }
        } else if part.contains('h') || part.contains('k') {
            if let Some(v) = apply_qualifier(part, parsed, warnings.as_deref_mut())? {
                entry = entry.with_rate(v)
            }
        } else if part.contains('b') {
            if let Some(v) = apply_qualifier(part, parsed, warnings.as_deref_mut())? {
                entry = entry.with_bitrate(v)
            }
        } else if part.contains('c') {
            if let Some(v) = apply_qualifier(part, parsed, warnings.as_deref_mut())? {
                entry = entry.with_channels(v)
            }
        }
    }

    Ok(entry)
}

/// Handle the strict/lenient decision for a parsed qualifier value.
///
/// Returns `Ok(Some(v))` when the value is valid, `Ok(None)` in lenient mode when
/// the value is invalid (a warning has been pushed), or `Err` in strict mode.
fn apply_qualifier(
    part: &str,
    parsed: Option<u32>,
    warnings: Option<&mut Vec<SdpWarning>>,
) -> Result<Option<u32>, CodecStringError> {
    match parsed {
        Some(v) => Ok(Some(v)),
        None => {
            let reason = format!("value in {part:?} overflows u32 or has no leading digits");
            match warnings {
                None => Err(CodecStringError::qualifier_parse_error(part, reason)),
                Some(acc) => {
                    acc.push(SdpWarning::codec_string_qualifier(part, reason));
                    Ok(None)
                }
            }
        }
    }
}

/// Parse a number from the start of a string, stopping at the first non-digit.
///
/// Returns `None` when there are no leading digits (including overflow — the caller
/// must distinguish between "no digits" and "overflow" using the raw string if needed).
fn atoi_prefix(s: &str) -> Option<u32> {
    let end = s
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(s.len());
    if end == 0 {
        None
    } else {
        s[..end]
            .parse::<u32>()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- order-free qualifiers ---

    #[test]
    fn order_free_qualifiers_both_orders() {
        let a: CodecString = "PCMU@20i@8000h"
            .parse()
            .unwrap();
        let b: CodecString = "PCMU@8000h@20i"
            .parse()
            .unwrap();
        assert_eq!(a.entries()[0].name(), "PCMU");
        assert_eq!(b.entries()[0].name(), "PCMU");
        assert_eq!(a.entries()[0].ptime(), b.entries()[0].ptime());
        assert_eq!(a.entries()[0].rate(), b.entries()[0].rate());
        assert_eq!(
            a.entries()[0].ptime(),
            Some(20),
            "ptime must be 20 in both orders"
        );
        assert_eq!(
            a.entries()[0].rate(),
            Some(8000),
            "rate must be 8000 in both orders"
        );
    }

    // --- unknown qualifier letter: strict vs lenient ---

    #[test]
    fn qualifier_with_no_letter_is_strict_error() {
        // In strict mode (FromStr), a qualifier part with no letter (e.g. "999") is
        // a hard parse error: data is silently lost otherwise.
        let result: Result<CodecString, _> = "PCMU@999@8000h".parse();
        assert!(
            result.is_err(),
            "FromStr must be strict: qualifier with no letter must fail"
        );
    }

    #[test]
    fn qualifier_overflow_is_strict_error() {
        // u32::MAX is 4294967295; 9999999999 overflows. Strict mode must fail.
        let result: Result<CodecString, _> = "PCMU@9999999999h".parse();
        assert!(
            result.is_err(),
            "FromStr must be strict: overflow rate qualifier must fail"
        );
    }

    #[test]
    fn qualifier_with_no_letter_lenient_records_warning() {
        // In lenient mode, the qualifier is skipped with a warning.
        let mut warnings = Vec::new();
        let cs = CodecString::parse_lenient("PCMU@999@8000h", &mut warnings).unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs.entries()[0].name(), "PCMU");
        assert_eq!(cs.entries()[0].rate(), Some(8000));
        assert!(
            !warnings.is_empty(),
            "lenient parse must record a warning for the no-letter qualifier"
        );
    }

    #[test]
    fn qualifier_overflow_lenient_records_warning() {
        let mut warnings = Vec::new();
        let cs = CodecString::parse_lenient("PCMU@9999999999h", &mut warnings).unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs.entries()[0].name(), "PCMU");
        assert_eq!(
            cs.entries()[0].rate(),
            None,
            "overflowed rate must be absent"
        );
        assert!(
            !warnings.is_empty(),
            "lenient parse must record a warning for the overflow"
        );
    }

    // --- modname.name splitting ---

    #[test]
    fn modname_name_split() {
        let cs: CodecString = "mod_opus.opus@48000h@20i"
            .parse()
            .unwrap();
        let e = &cs.entries()[0];
        assert_eq!(e.modname(), Some("mod_opus"));
        assert_eq!(e.name(), "opus");
        assert_eq!(e.rate(), Some(48000));
        assert_eq!(e.ptime(), Some(20));
    }

    // --- name~fmtp splitting ---

    #[test]
    fn name_fmtp_split() {
        let cs: CodecString = "AMR-WB~octet-align=1@16000h@20i"
            .parse()
            .unwrap();
        let e = &cs.entries()[0];
        assert_eq!(e.name(), "AMR-WB");
        assert_eq!(e.fmtp(), Some("octet-align=1"));
        assert_eq!(e.rate(), Some(16000));
        assert_eq!(e.ptime(), Some(20));
    }

    // --- dotted fmtp with module prefix ---

    #[test]
    fn dotted_fmtp_with_module_roundtrips() {
        let entry = CodecStringEntry::new("EVS")
            .unwrap()
            .with_module("mod_evs")
            .unwrap()
            .with_fmtp("br=13.2-24.4")
            .unwrap()
            .with_rate(16000)
            .with_ptime(20);
        let s = entry.to_string();
        let cs: CodecString = s
            .parse()
            .unwrap();
        let e = &cs.entries()[0];
        assert_eq!(e.modname(), Some("mod_evs"));
        assert_eq!(e.name(), "EVS");
        assert_eq!(e.fmtp(), Some("br=13.2-24.4"));
        assert_eq!(e.rate(), Some(16000));
        assert_eq!(e.ptime(), Some(20));
    }

    // --- with_fmtp validation ---

    #[test]
    fn fmtp_with_at_is_rejected() {
        let result = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_fmtp("br=13.2@24.4");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CodecStringError::AtInFmtp(_)));
    }

    #[test]
    fn fmtp_with_dot_no_module_is_rejected() {
        let result = CodecStringEntry::new("EVS")
            .unwrap()
            .with_fmtp("br=13.2-24.4");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CodecStringError::DotInFmtpWithoutModule(_)
        ));
    }

    #[test]
    fn fmtp_with_dot_and_module_is_accepted() {
        let result = CodecStringEntry::new("EVS")
            .unwrap()
            .with_module("mod_evs")
            .unwrap()
            .with_fmtp("br=13.2-24.4");
        assert!(result.is_ok());
    }

    #[test]
    fn fmtp_with_newline_is_rejected() {
        let result = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_fmtp("mode=20\n");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CodecStringError::WireInjection { .. }
        ));
    }

    #[test]
    fn fmtp_with_cr_is_rejected() {
        let result = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_fmtp("mode=20\r");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CodecStringError::WireInjection { .. }
        ));
    }

    // --- Display escaping ---

    #[test]
    fn display_escapes_comma_in_fmtp() {
        let entry = CodecStringEntry::new("AMR-WB")
            .unwrap()
            .with_module("mod_amrwb")
            .unwrap()
            .with_fmtp("mode-set=0,1,2")
            .unwrap();
        let s = entry.to_string();
        assert!(
            s.contains("mode-set=0\\,1\\,2"),
            "commas in fmtp must be escaped as \\,: {s}"
        );
    }

    #[test]
    fn display_escapes_backslash_in_fmtp() {
        let entry = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_module("mod_pcmu")
            .unwrap()
            .with_fmtp("x=a\\b")
            .unwrap();
        let s = entry.to_string();
        assert!(
            s.contains("x=a\\\\b"),
            "backslash in fmtp must be escaped as \\\\: {s}"
        );
    }

    #[test]
    fn display_escapes_quote_in_fmtp() {
        let entry = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_module("mod_pcmu")
            .unwrap()
            .with_fmtp("x=a'b")
            .unwrap();
        let s = entry.to_string();
        assert!(
            s.contains("x=a\\'b"),
            "quote in fmtp must be escaped as \\': {s}"
        );
    }

    // --- escaped comma round-trips ---

    #[test]
    fn escaped_comma_in_fmtp_roundtrips() {
        let entry = CodecStringEntry::new("AMR-WB")
            .unwrap()
            .with_module("mod_amrwb")
            .unwrap()
            .with_fmtp("mode-set=0,1,2")
            .unwrap()
            .with_rate(16000)
            .with_ptime(20);
        let s = entry.to_string();
        let cs: CodecString = s
            .parse()
            .unwrap();
        let e = &cs.entries()[0];
        assert_eq!(
            e.fmtp(),
            Some("mode-set=0,1,2"),
            "fmtp must round-trip with commas unescaped: {s}"
        );
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

    // --- round-trip table ---

    #[test]
    fn round_trip_table() {
        let cases: &[(&str, &dyn Fn(&CodecStringEntry))] = &[
            ("PCMU", &|e| {
                assert_eq!(e.name(), "PCMU");
                assert!(e
                    .modname()
                    .is_none());
                assert!(e
                    .rate()
                    .is_none());
                assert!(e
                    .fmtp()
                    .is_none());
            }),
            ("PCMU@8000h", &|e| {
                assert_eq!(e.name(), "PCMU");
                assert_eq!(e.rate(), Some(8000));
            }),
            ("PCMU@8000h@20i@64000b@1c", &|e| {
                assert_eq!(e.rate(), Some(8000));
                assert_eq!(e.ptime(), Some(20));
                assert_eq!(e.bitrate(), Some(64000));
                assert_eq!(e.channels(), Some(1));
            }),
            ("mod_opus.opus@48000h@20i", &|e| {
                assert_eq!(e.modname(), Some("mod_opus"));
                assert_eq!(e.name(), "opus");
                assert_eq!(e.rate(), Some(48000));
            }),
        ];

        for (input, check) in cases {
            let cs: CodecString = input
                .parse()
                .unwrap_or_else(|e| panic!("parse failed for {input:?}: {e}"));
            assert_eq!(cs.len(), 1, "expected 1 entry for {input:?}");
            let entry = &cs.entries()[0];
            check(entry);
            // Round-trip.
            let displayed = cs.to_string();
            let cs2: CodecString = displayed
                .parse()
                .unwrap_or_else(|e| panic!("re-parse failed for displayed {displayed:?}: {e}"));
            assert_eq!(cs, cs2, "round-trip failed for {input:?}");
        }
    }

    // --- Fix 1: delimiter chars are rejected in names and modnames ---

    #[test]
    fn name_with_comma_is_rejected() {
        assert!(CodecStringEntry::new("PC,MU").is_err());
        let err = CodecStringEntry::new("PC,MU").unwrap_err();
        assert!(matches!(err, CodecStringError::InvalidCharInName { .. }));
    }

    #[test]
    fn name_with_at_is_rejected() {
        let err = CodecStringEntry::new("bad@name").unwrap_err();
        assert!(matches!(err, CodecStringError::InvalidCharInName { .. }));
    }

    #[test]
    fn name_with_tilde_is_rejected() {
        let err = CodecStringEntry::new("bad~name").unwrap_err();
        assert!(matches!(err, CodecStringError::InvalidCharInName { .. }));
    }

    #[test]
    fn name_with_dot_is_rejected() {
        let err = CodecStringEntry::new("bad.name").unwrap_err();
        assert!(matches!(err, CodecStringError::InvalidCharInName { .. }));
    }

    #[test]
    fn name_with_backslash_is_rejected() {
        let err = CodecStringEntry::new("bad\\name").unwrap_err();
        assert!(matches!(err, CodecStringError::InvalidCharInName { .. }));
    }

    #[test]
    fn name_with_quote_is_rejected() {
        let err = CodecStringEntry::new("bad'name").unwrap_err();
        assert!(matches!(err, CodecStringError::InvalidCharInName { .. }));
    }

    #[test]
    fn name_with_space_is_rejected() {
        let err = CodecStringEntry::new("bad name").unwrap_err();
        assert!(matches!(err, CodecStringError::InvalidCharInName { .. }));
    }

    #[test]
    fn modname_with_comma_is_rejected() {
        let result = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_module("bad,mod");
        assert!(matches!(
            result.unwrap_err(),
            CodecStringError::InvalidCharInName { .. }
        ));
    }

    #[test]
    fn name_injection_does_not_roundtrip() {
        // "PC,MU" must be rejected so the Display output cannot produce two entries
        // when re-parsed. If this passes through, parse("PC,MU") produces ["PC","MU"].
        assert!(CodecStringEntry::new("PC,MU").is_err());
    }

    #[test]
    fn set_name_with_delimiter_is_rejected() {
        let mut e = CodecStringEntry::new("PCMU").unwrap();
        assert!(matches!(
            e.set_name("PC,MU")
                .unwrap_err(),
            CodecStringError::InvalidCharInName { .. }
        ));
    }

    #[test]
    fn set_module_with_delimiter_is_rejected() {
        let mut e = CodecStringEntry::new("PCMU").unwrap();
        assert!(matches!(
            e.set_module("bad@mod")
                .unwrap_err(),
            CodecStringError::InvalidCharInName { .. }
        ));
    }

    // Real FreeSWITCH codec names must still pass validation.
    #[test]
    fn real_codec_names_accepted() {
        for name in &[
            "PCMU",
            "PCMA",
            "G722",
            "AMR-WB",
            "AMR",
            "opus",
            "telephone-event",
            "G7221",
            "H263-1998",
        ] {
            assert!(
                CodecStringEntry::new(*name).is_ok(),
                "real codec name {name:?} must be accepted"
            );
        }
    }

    // --- name+fmtp and name+fmtp+qualifiers ---

    #[test]
    fn name_fmtp_no_qualifiers_roundtrip() {
        let entry = CodecStringEntry::new("AMR-WB")
            .unwrap()
            .with_module("mod_amrwb")
            .unwrap()
            .with_fmtp("octet-align=1")
            .unwrap();
        let s = entry.to_string();
        let cs: CodecString = s
            .parse()
            .unwrap();
        assert_eq!(cs.entries()[0].fmtp(), Some("octet-align=1"));
        assert!(cs.entries()[0]
            .rate()
            .is_none());
    }

    #[test]
    fn name_fmtp_with_qualifiers_roundtrip() {
        let entry = CodecStringEntry::new("AMR-WB")
            .unwrap()
            .with_module("mod_amrwb")
            .unwrap()
            .with_fmtp("octet-align=1")
            .unwrap()
            .with_rate(16000)
            .with_ptime(20);
        let s = entry.to_string();
        let cs: CodecString = s
            .parse()
            .unwrap();
        let e = &cs.entries()[0];
        assert_eq!(e.fmtp(), Some("octet-align=1"));
        assert_eq!(e.rate(), Some(16000));
        assert_eq!(e.ptime(), Some(20));
    }

    // --- Defect 1: parse_entry must reject newlines in fmtp ---

    #[test]
    fn parse_entry_fmtp_with_newline_is_rejected() {
        let result: Result<CodecString, _> = "AMR-WB~mode=0\n".parse();
        assert!(
            result.is_err(),
            "parsed fmtp containing \\n must be rejected"
        );
    }

    #[test]
    fn parse_entry_fmtp_with_cr_is_rejected() {
        let result: Result<CodecString, _> = "AMR-WB~mode=0\r".parse();
        assert!(
            result.is_err(),
            "parsed fmtp containing \\r must be rejected"
        );
    }

    // --- Defect 2: fallible setters replace unsafe _mut() accessors ---

    #[test]
    fn set_fmtp_with_at_is_rejected() {
        let mut entry = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_module("mod_pcmu")
            .unwrap();
        assert!(matches!(
            entry
                .set_fmtp("br=13.2@24.4")
                .unwrap_err(),
            CodecStringError::AtInFmtp(_)
        ));
    }

    #[test]
    fn set_fmtp_with_newline_is_rejected() {
        let mut entry = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_module("mod_pcmu")
            .unwrap();
        assert!(matches!(
            entry
                .set_fmtp("mode=0\n")
                .unwrap_err(),
            CodecStringError::WireInjection { .. }
        ));
    }

    #[test]
    fn set_name_empty_is_rejected() {
        let mut entry = CodecStringEntry::new("PCMU").unwrap();
        assert!(entry
            .set_name("")
            .is_err());
    }

    #[test]
    fn clear_module_with_dotted_fmtp_is_rejected() {
        let mut entry = CodecStringEntry::new("EVS")
            .unwrap()
            .with_module("mod_evs")
            .unwrap()
            .with_fmtp("br=13.2-24.4")
            .unwrap();
        let result = entry.clear_module();
        assert!(result.is_err());
        assert_eq!(
            entry.modname(),
            Some("mod_evs"),
            "entry must be left unchanged after failed clear_module"
        );
    }

    #[test]
    fn clear_module_with_dotless_fmtp_is_ok() {
        let mut entry = CodecStringEntry::new("AMR-WB")
            .unwrap()
            .with_module("mod_amrwb")
            .unwrap()
            .with_fmtp("octet-align=1")
            .unwrap();
        assert!(entry
            .clear_module()
            .is_ok());
        assert!(entry
            .modname()
            .is_none());
    }

    // --- Fix 4: channels must not silently truncate ---

    #[test]
    fn channels_u8_overflow_is_not_truncated() {
        // 300 as u8 = 44; the bug is the "as u8" cast. After widening to u32 this roundtrips.
        let cs: CodecString = "PCMU@300c"
            .parse()
            .unwrap();
        assert_eq!(
            cs.entries()[0].channels(),
            Some(300),
            "channels=300 must not be truncated to 44"
        );
    }

    #[test]
    fn with_channels_accepts_u32() {
        let entry = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_channels(300_u32);
        assert_eq!(entry.channels(), Some(300));
    }

    #[test]
    fn channels_roundtrip() {
        let entry = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_channels(300_u32)
            .with_rate(8000);
        let s = entry.to_string();
        let cs: CodecString = s
            .parse()
            .unwrap();
        assert_eq!(cs.entries()[0].channels(), Some(300));
    }

    // --- Fix 2: split_codec_string must mirror cleanup_separated_string ---

    #[test]
    fn leading_space_after_comma_is_stripped() {
        // "PCMU, PCMA" is a normal policy string; the space must not land in the name.
        let cs: CodecString = "PCMU, PCMA"
            .parse()
            .unwrap();
        assert_eq!(cs.len(), 2);
        assert_eq!(cs.entries()[0].name(), "PCMU");
        assert_eq!(
            cs.entries()[1].name(),
            "PCMA",
            "leading space must be stripped"
        );
    }

    #[test]
    fn trailing_space_before_comma_is_stripped() {
        let cs: CodecString = "PCMU ,PCMA"
            .parse()
            .unwrap();
        assert_eq!(cs.len(), 2);
        assert_eq!(
            cs.entries()[0].name(),
            "PCMU",
            "trailing space must be stripped"
        );
        assert_eq!(cs.entries()[1].name(), "PCMA");
    }

    #[test]
    fn both_leading_and_trailing_spaces_stripped() {
        let cs: CodecString = "PCMU , PCMA"
            .parse()
            .unwrap();
        assert_eq!(cs.len(), 2);
        assert_eq!(cs.entries()[0].name(), "PCMU");
        assert_eq!(cs.entries()[1].name(), "PCMA");
    }

    #[test]
    fn quote_stripping_in_codec_string() {
        // Quotes toggle inside_quotes and are stripped from output.
        // 'PCMU' should parse as PCMU.
        let cs: CodecString = "'PCMU'"
            .parse()
            .unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs.entries()[0].name(), "PCMU");
    }

    #[test]
    fn odd_quote_count_before_comma_still_splits() {
        // Each ' toggles only when a closing ' is ahead (C: strchr(ptr+1, '\'')),
        // never by scanning what's already been consumed. Three quotes then a
        // comma must still split into two raw tokens; the first token's content
        // (an unterminated quote survives as a literal char, same as the C
        // cleanup) is a separate, orthogonal name-validation concern.
        let tokens = split_codec_string("a'b'c'd,PCMA");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[1], "PCMA");
    }

    #[test]
    fn quoted_pair_with_trailing_codec_still_splits() {
        let cs: CodecString = "'PCMU',PCMA"
            .parse()
            .unwrap();
        assert_eq!(cs.len(), 2);
        assert_eq!(cs.entries()[0].name(), "PCMU");
        assert_eq!(cs.entries()[1].name(), "PCMA");
    }

    #[test]
    fn backslash_n_escape_becomes_lf_and_then_rejected() {
        // The C cleanup_separated_string unescapes \n to a real LF.
        // Our name validator then rejects it as WireInjection.
        let result: Result<CodecString, _> = "PCMU\\nPCMA".parse();
        assert!(
            result.is_err(),
            "\\n in a token must be unescaped to LF then rejected"
        );
    }

    #[test]
    fn backslash_s_escape_becomes_space_and_then_rejected_in_name() {
        // \s → space; space is forbidden in a name.
        let result: Result<CodecString, _> = "PC\\sMU".parse();
        assert!(
            result.is_err(),
            "\\s in a name must be unescaped to space then rejected"
        );
    }

    // --- Step 3: FromStr for CodecStringEntry ---

    #[test]
    fn entry_from_str_basic_roundtrip() {
        let s = "PCMU@8000h@20i@64000b@1c";
        let entry: CodecStringEntry = s
            .parse()
            .unwrap();
        assert_eq!(entry.name(), "PCMU");
        assert_eq!(entry.rate(), Some(8000));
        assert_eq!(entry.ptime(), Some(20));
        assert_eq!(entry.bitrate(), Some(64000));
        assert_eq!(entry.channels(), Some(1));
        assert_eq!(entry.to_string(), s);
    }

    #[test]
    fn entry_from_str_escaped_comma_fmtp_roundtrip() {
        let entry = CodecStringEntry::new("AMR-WB")
            .unwrap()
            .with_module("mod_amrwb")
            .unwrap()
            .with_fmtp("mode-set=0,1,2")
            .unwrap()
            .with_rate(16000)
            .with_ptime(20);
        let s = entry.to_string();
        let parsed: CodecStringEntry = s
            .parse()
            .unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn entry_from_str_comma_is_error() {
        let result = "PCMU,PCMA".parse::<CodecStringEntry>();
        assert!(
            result.is_err(),
            "top-level comma must be an error for single-entry FromStr"
        );
        assert!(
            matches!(result.unwrap_err(), CodecStringError::MultipleEntries(_)),
            "error must be MultipleEntries"
        );
    }

    // --- Step 3: clear_* methods ---

    #[test]
    fn clear_rate_sets_none() {
        let mut entry = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_rate(8000);
        assert_eq!(entry.rate(), Some(8000));
        entry.clear_rate();
        assert!(entry
            .rate()
            .is_none());
    }

    #[test]
    fn clear_ptime_sets_none() {
        let mut entry = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_ptime(20);
        assert_eq!(entry.ptime(), Some(20));
        entry.clear_ptime();
        assert!(entry
            .ptime()
            .is_none());
    }

    #[test]
    fn clear_bitrate_sets_none() {
        let mut entry = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_bitrate(64000);
        assert_eq!(entry.bitrate(), Some(64000));
        entry.clear_bitrate();
        assert!(entry
            .bitrate()
            .is_none());
    }

    #[test]
    fn clear_channels_sets_none() {
        let mut entry = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_channels(1);
        assert_eq!(entry.channels(), Some(1));
        entry.clear_channels();
        assert!(entry
            .channels()
            .is_none());
    }

    // --- Step 4: collection methods ---

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
    fn contains_name_case_insensitive() {
        let cs: CodecString = "PCMU,pcma"
            .parse()
            .unwrap();
        assert!(cs.contains_name("pcmu"));
        assert!(cs.contains_name("PCMA"));
        assert!(!cs.contains_name("G722"));
    }

    #[test]
    fn names_yields_all_names() {
        let cs: CodecString = "PCMU,PCMA"
            .parse()
            .unwrap();
        let names: Vec<&str> = cs
            .names()
            .collect();
        assert_eq!(names, vec!["PCMU", "PCMA"]);
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

    #[test]
    fn max_switch_entries_const() {
        assert_eq!(CodecString::MAX_SWITCH_ENTRIES, 50);
    }

    #[test]
    fn excess_count_at_limit() {
        let mut cs = CodecString::new();
        for _ in 0..50 {
            cs.push(CodecStringEntry::new("PCMU").unwrap());
        }
        assert_eq!(cs.excess_count(), 0);
    }

    #[test]
    fn excess_count_over_limit() {
        let mut cs = CodecString::new();
        for _ in 0..51 {
            cs.push(CodecStringEntry::new("PCMU").unwrap());
        }
        assert_eq!(cs.excess_count(), 1);
    }

    #[test]
    fn raw_token_count_double_comma() {
        // C's separate_string_char_delim assigns a slot for every token including empties.
        assert_eq!(CodecString::raw_token_count("PCMU,,PCMA"), 3);
    }

    #[test]
    fn raw_token_count_simple() {
        assert_eq!(CodecString::raw_token_count("PCMU,PCMA,G722"), 3);
        assert_eq!(CodecString::raw_token_count("PCMU"), 1);
        assert_eq!(CodecString::raw_token_count(""), 0);
    }

    #[test]
    fn raw_token_count_trailing_comma_is_one_slot() {
        // C only allocates a slot on entering START with a non-null char; a
        // trailing comma never starts a new token, so it isn't a second slot.
        assert_eq!(CodecString::raw_token_count("PCMU,"), 1);
    }

    #[test]
    fn raw_token_count_bare_commas_two_slots() {
        assert_eq!(CodecString::raw_token_count("A,,"), 2);
    }

    #[test]
    fn raw_token_count_escaped_comma_not_split() {
        // \, in fmtp is one entry, not two tokens
        assert_eq!(CodecString::raw_token_count("AMR~mode-set=0\\,1"), 1);
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
            .names()
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

    // --- Step 5: simplify() ---

    #[test]
    fn simplify_pcmu_full_qualifiers() {
        let mut e: CodecStringEntry = "PCMU@8000h@20i@1c"
            .parse()
            .unwrap();
        e.simplify();
        assert_eq!(e.to_string(), "PCMU");
    }

    #[test]
    fn simplify_opus_default_qualifiers() {
        let mut e: CodecStringEntry = "opus@48000h@20i"
            .parse()
            .unwrap();
        e.simplify();
        assert_eq!(e.to_string(), "opus");
    }

    #[test]
    fn simplify_ilbc_default_30ms() {
        let mut e: CodecStringEntry = "iLBC@8000h@30i"
            .parse()
            .unwrap();
        e.simplify();
        assert_eq!(e.to_string(), "iLBC");
    }

    #[test]
    fn simplify_amr_nondefault_ptime_survives() {
        // AMR@8000h@40i: rate 8000 is default (cleared), ptime 40 ≠ 20 (kept).
        let mut e: CodecStringEntry = "AMR@8000h@40i"
            .parse()
            .unwrap();
        e.simplify();
        assert_eq!(e.to_string(), "AMR@40i");
    }

    #[test]
    fn simplify_g722_exempt() {
        // G.722 uses samples_per_second=16000 in the matching pass but default_rate=8000;
        // simplifying would silently change which implementation is selected.
        let mut e: CodecStringEntry = "G722@8000h@20i"
            .parse()
            .unwrap();
        e.simplify();
        assert_eq!(e.to_string(), "G722@8000h@20i");
    }

    #[test]
    fn simplify_bitrate_not_touched() {
        // Bitrate has no default function; it survives simplification.
        let mut e: CodecStringEntry = "PCMU@64000b"
            .parse()
            .unwrap();
        e.simplify();
        assert_eq!(e.to_string(), "PCMU@64000b");
    }
}
