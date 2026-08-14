//! [`CodecStringEntry`]: a single entry in a FreeSWITCH codec string.

use std::fmt;
use std::str::FromStr;

use crate::sdp::error::CodecStringError;
use crate::sdp::static_payload::{default_ptime, default_rate};

use super::parse::{escape_fmtp, normalize_fmtp_trailing_space, parse_entry, split_codec_string};

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
        let fmtp = normalize_fmtp_trailing_space(fmtp.into());
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
    /// to match. [`SdpCodec::channels`](super::super::codec::SdpCodec::channels) stays `Option<u8>` (from `a=rtpmap`, realistically tiny)
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
        let fmtp = normalize_fmtp_trailing_space(fmtp.into());
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
    /// loaded implementation (see [`CodecString::qualified`](super::CodecString::qualified)).
    pub fn clear_rate(&mut self) {
        self.rate = None;
    }

    /// Clear the ptime qualifier.
    ///
    /// Infallible analog to `clear_fmtp`; use to strip a qualifier that matched no
    /// loaded implementation (see [`CodecString::qualified`](super::CodecString::qualified)).
    pub fn clear_ptime(&mut self) {
        self.ptime = None;
    }

    /// Clear the bitrate qualifier.
    ///
    /// Infallible analog to `clear_fmtp`; use to strip a qualifier that matched no
    /// loaded implementation (see [`CodecString::qualified`](super::CodecString::qualified)).
    pub fn clear_bitrate(&mut self) {
        self.bitrate = None;
    }

    /// Clear the channel count qualifier.
    ///
    /// Infallible analog to `clear_fmtp`; use to strip a qualifier that matched no
    /// loaded implementation (see [`CodecString::qualified`](super::CodecString::qualified)).
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
    /// [`CodecString::retain_available`](super::CodecString::retain_available). `simplify()` compares against this crate's
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
    /// Reuses the same escape/quote-aware split as [`CodecString`](super::CodecString)'s parser, then
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

#[cfg(test)]
mod tests {
    use super::super::CodecString;
    use super::*;

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

    // --- fmtp trailing whitespace normalization ---

    #[test]
    fn with_fmtp_strips_trailing_space() {
        let entry = CodecStringEntry::new("AMR")
            .unwrap()
            .with_fmtp("octet-align=1 ")
            .unwrap();
        assert_eq!(entry.fmtp(), Some("octet-align=1"));
    }

    #[test]
    fn set_fmtp_strips_trailing_space() {
        let mut entry = CodecStringEntry::new("AMR").unwrap();
        entry
            .set_fmtp("octet-align=1 ")
            .unwrap();
        assert_eq!(entry.fmtp(), Some("octet-align=1"));
    }

    #[test]
    fn fmtp_trailing_space_round_trips_through_display_and_parse() {
        let entry = CodecStringEntry::new("AMR")
            .unwrap()
            .with_fmtp("octet-align=1 ")
            .unwrap();
        let s = entry.to_string();
        let reparsed: CodecString = s
            .parse()
            .unwrap();
        assert_eq!(reparsed.entries()[0].fmtp(), Some("octet-align=1"));
    }

    #[test]
    fn fmtp_interior_whitespace_is_preserved() {
        // Only the trailing run is normalized away; interior whitespace has no
        // grammar meaning of its own but remains opaque byte-string content.
        let entry = CodecStringEntry::new("PCMU")
            .unwrap()
            .with_fmtp("a=1  b=2")
            .unwrap();
        assert_eq!(entry.fmtp(), Some("a=1  b=2"));
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

    // --- round-trip table ---

    #[test]
    #[allow(clippy::type_complexity)]
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
