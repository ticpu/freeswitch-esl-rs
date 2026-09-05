//! Tokenizer and single-entry parser shared by [`CodecString`] and [`CodecStringEntry`].

use std::iter::Peekable;
use std::str::Chars;

use crate::sdp::error::{CodecStringError, SdpWarning};
use crate::sdp::num::atoi_prefix;

use super::entry::CodecStringEntry;
use super::CodecString;

/// Whether a matching close quote lies ahead, mirroring the C `strchr(ptr + 1, '\'')`.
fn has_closing_quote(rest: &Peekable<Chars<'_>>) -> bool {
    rest.clone()
        .any(|c| c == '\'')
}

/// Inner parser shared by [`FromStr`] (strict, `warnings = None`) and
/// [`CodecString::parse_lenient`] (lenient, `warnings = Some`).
pub(super) fn parse_codec_string_inner(
    s: &str,
    mut warnings: Option<&mut Vec<SdpWarning>>,
) -> Result<CodecString, CodecStringError> {
    let tokens = split_codec_string(s);
    if tokens.len() > CodecString::MAX_SWITCH_ENTRIES {
        match warnings.as_deref_mut() {
            None => {
                return Err(CodecStringError::too_many_entries(
                    tokens.len(),
                    CodecString::MAX_SWITCH_ENTRIES,
                ))
            }
            Some(acc) => acc.push(SdpWarning::codec_string_truncated(
                tokens.len(),
                CodecString::MAX_SWITCH_ENTRIES,
            )),
        }
    }

    let mut entries = Vec::new();
    for token in tokens {
        if token.is_empty() {
            continue;
        }
        entries.push(parse_entry(&token, warnings.as_deref_mut())?);
    }
    Ok(CodecString::from_entries(entries))
}

/// Strip trailing spaces from an fmtp value at the point it's set.
///
/// `cleanup_separated_string` (`switch_utils.c:2702`) strips a trailing SP run
/// from the codec-string token regardless of what this layer does — normalizing
/// here means a directly-constructed entry and a round-tripped one compare equal
/// in [`CodecString::dedup`]. Only SP is stripped, matching the C (which never
/// special-cases HTAB).
pub(super) fn normalize_fmtp_trailing_space(fmtp: String) -> String {
    let trimmed = fmtp.trim_end_matches(' ');
    if trimmed.len() == fmtp.len() {
        fmtp
    } else {
        trimmed.to_string()
    }
}

/// Escape `,` `\` `'` in an fmtp value for safe embedding in a codec string.
///
/// A raw comma splits entries; a lone `'` or `\` has grammar significance in the
/// surrounding separator layer (`cleanup_separated_string`, `switch_utils.c:2702`).
pub(super) fn escape_fmtp(s: &str) -> String {
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
pub(super) fn split_codec_string(s: &str) -> Vec<String> {
    // Split on ',' honouring escape and quote, mirroring separate_string_char_delim.
    let mut raw_tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut inside_quotes = false;
    let mut chars = s
        .chars()
        .peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // Backslash and the char it escapes ride into the raw token verbatim;
            // cleanup_token expands them. Skipping ahead only prevents a split.
            if let Some(&next) = chars.peek() {
                chars.next();
                current.push('\\');
                current.push(next);
            } else {
                current.push('\\');
            }
        } else if ch == '\'' {
            // Quote state affects the split point; cleanup_token strips the quote itself.
            if inside_quotes || has_closing_quote(&chars) {
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

    // Then cleanup_token on each raw token.
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
                // Unrecognized escape: upstream reprocesses the next char, so a
                // following space still trims.
                None => {
                    out.push('\\');
                    end_len = out.len();
                }
            }
        } else if ch == '\'' {
            if inside_quotes || has_closing_quote(&chars) {
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
pub(super) fn parse_entry(
    token: &str,
    mut warnings: Option<&mut Vec<SdpWarning>>,
) -> Result<CodecStringEntry, CodecStringError> {
    // `has_at` separates no `@` at all from a trailing one, which yields the same
    // empty remainder but is an empty qualifier part, like `@@`.
    let (name_seg, qualifier_str, has_at) = match token.split_once('@') {
        Some((n, q)) => (n, q, true),
        None => (token, "", false),
    };

    let qualifiers: Vec<&str> = if has_at {
        qualifier_str
            .split('@')
            .collect()
    } else {
        Vec::new()
    };

    // The `.` split precedes the `~` split, as it does in the C.
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

    for part in &qualifiers {
        apply_qualifier_part(&mut entry, part, warnings.as_deref_mut())?;
    }

    Ok(entry)
}

/// Which field a `@`-delimited part sets.
#[derive(Debug, Clone, Copy)]
enum Qualifier {
    Ptime,
    Rate,
    Bitrate,
    Channels,
}

/// Scan a part for its qualifier letter in FreeSWITCH's own order — `i`, then
/// `k`/`h`, then `b`, then `c` — as a substring search, not a suffix check.
fn classify(part: &str) -> Option<Qualifier> {
    if part.contains('i') {
        Some(Qualifier::Ptime)
    } else if part.contains('h') || part.contains('k') {
        Some(Qualifier::Rate)
    } else if part.contains('b') {
        Some(Qualifier::Bitrate)
    } else if part.contains('c') {
        Some(Qualifier::Channels)
    } else {
        None
    }
}

/// The strict/lenient decision for a qualifier this parser cannot use: strict
/// (`warnings = None`) fails, lenient records the fault and the part is skipped.
fn qualifier_fault(
    part: &str,
    reason: &str,
    warnings: Option<&mut Vec<SdpWarning>>,
) -> Result<(), CodecStringError> {
    match warnings {
        None => Err(CodecStringError::qualifier_parse_error(part, reason)),
        Some(acc) => {
            acc.push(SdpWarning::codec_string_qualifier(part, reason));
            Ok(())
        }
    }
}

/// Classify one `@`-delimited qualifier part and assign it.
fn apply_qualifier_part(
    entry: &mut CodecStringEntry,
    part: &str,
    warnings: Option<&mut Vec<SdpWarning>>,
) -> Result<(), CodecStringError> {
    let Some(qualifier) = classify(part) else {
        return qualifier_fault(part, "no recognised qualifier letter", warnings);
    };

    let Some(value) = atoi_prefix(part).0 else {
        let reason = if part.starts_with(|c: char| c.is_ascii_digit()) {
            "value overflows u32"
        } else {
            "no leading digits"
        };
        return qualifier_fault(part, reason, warnings);
    };

    match qualifier {
        Qualifier::Ptime => *entry.ptime_mut() = Some(value),
        Qualifier::Rate => *entry.rate_mut() = Some(value),
        Qualifier::Bitrate => *entry.bitrate_mut() = Some(value),
        Qualifier::Channels => *entry.channels_mut() = Some(value),
    }

    Ok(())
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

    // --- trailing `@` must behave like `@@`: an empty qualifier segment is a hard error ---

    #[test]
    fn trailing_at_is_strict_error_like_double_at() {
        let result: Result<CodecString, _> = "PCMU@".parse();
        assert!(
            result.is_err(),
            "trailing @ produces an empty qualifier segment; strict mode must fail"
        );
        assert!(matches!(
            result.unwrap_err(),
            CodecStringError::QualifierParseError { .. }
        ));
    }

    #[test]
    fn double_at_is_still_strict_error() {
        // Guard against the fix changing `@@` behaviour instead of just `@`.
        let result: Result<CodecString, _> = "PCMU@@8000h".parse();
        assert!(result.is_err(), "@@ must remain a strict error");
        assert!(matches!(
            result.unwrap_err(),
            CodecStringError::QualifierParseError { .. }
        ));
    }

    #[test]
    fn trailing_at_lenient_records_warning() {
        let mut warnings = Vec::new();
        let cs = CodecString::parse_lenient("PCMU@", &mut warnings).unwrap();
        assert_eq!(cs.entries()[0].name(), "PCMU");
        assert_eq!(cs.entries()[0].rate(), None);
        assert_eq!(cs.entries()[0].ptime(), None);
        assert!(
            !warnings.is_empty(),
            "lenient parse must record a warning for the empty qualifier from a trailing @"
        );
    }

    #[test]
    fn no_at_still_parses_clean() {
        let cs: CodecString = "PCMU"
            .parse()
            .unwrap();
        assert_eq!(cs.entries()[0].name(), "PCMU");
        assert_eq!(cs.entries()[0].rate(), None);
    }

    #[test]
    fn single_qualifier_still_parses_clean() {
        let cs: CodecString = "PCMU@8000h"
            .parse()
            .unwrap();
        assert_eq!(cs.entries()[0].name(), "PCMU");
        assert_eq!(cs.entries()[0].rate(), Some(8000));
    }

    // --- the split mirrors cleanup_separated_string ---

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
}
