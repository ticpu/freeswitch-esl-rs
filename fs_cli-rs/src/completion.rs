//! Tab completion support for fs_cli-rs

use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::highlight::{CmdKind, Highlighter, MatchingBracketHighlighter};
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::validate::{self, MatchingBracketValidator, Validator};
use rustyline::{Context, Helper};
use std::borrow::Cow::{self, Borrowed, Owned};

/// FreeSWITCH CLI completer with command suggestions
pub struct FsCliCompleter {
    filename_completer: FilenameCompleter,
    history_hinter: HistoryHinter,
    bracket_highlighter: MatchingBracketHighlighter,
    bracket_validator: MatchingBracketValidator,
}

impl FsCliCompleter {
    /// Create new completer
    pub fn new() -> Self {
        Self {
            filename_completer: FilenameCompleter::new(),
            history_hinter: HistoryHinter::new(),
            bracket_highlighter: MatchingBracketHighlighter::new(),
            bracket_validator: MatchingBracketValidator::new(),
        }
    }

    /// Get FreeSWITCH command suggestions
    fn get_fs_commands() -> Vec<&'static str> {
        vec![
            // Basic commands
            "status",
            "version",
            "uptime",
            "help",
            // Show commands
            "show",
            "show channels",
            "show channels count",
            "show calls",
            "show registrations",
            "show modules",
            "show interfaces",
            "show api",
            "show application",
            "show codec",
            "show file",
            "show timer",
            "show tasks",
            "show complete",
            // Control commands
            "reload",
            "reloadxml",
            "reload mod_sofia",
            "reload mod_dialplan_xml",
            "originate",
            // Sofia commands
            "sofia",
            "sofia status",
            "sofia profile",
            "sofia profile internal",
            "sofia profile external",
            "sofia global",
            // Channel commands
            "uuid_answer",
            "uuid_hangup",
            "uuid_transfer",
            "uuid_bridge",
            "uuid_park",
            "uuid_hold",
            "uuid_break",
            "uuid_kill",
            // Conference commands
            "conference",
            "conference list",
            "conference kick",
            "conference mute",
            "conference unmute",
            // System commands
            "fsctl",
            "fsctl pause",
            "fsctl resume",
            "fsctl shutdown",
            "fsctl crash",
            "fsctl send_sighup",
            "load",
            "unload",
            "bgapi",
            // Log commands
            "console",
            "log",
            "uuid_dump",
            // Database commands
            "db",
            "group",
            "user_exists",
            // Other common commands
            "hupall",
            "pause",
            "resume",
            "shutdown",
            "expr",
            "eval",
            "expand",
            "global_getvar",
            "global_setvar",
        ]
    }


    /// Get command completions for a given input
    fn complete_command(&self, line: &str, pos: usize) -> rustyline::Result<(usize, Vec<Pair>)> {
        let commands = Self::get_fs_commands();

        // Find the current word being completed
        let line_bytes = line.as_bytes();
        let mut start = pos;
        
        // Find start of current word (go back to last space or start)
        while start > 0 && line_bytes[start - 1] != b' ' {
            start -= 1;
        }
        
        let current_word = &line[start..pos];
        
        // Find matching commands
        let matches: Vec<Pair> = commands
            .into_iter()
            .filter(|cmd| {
                // For multi-word commands, check if they start with current line
                if cmd.starts_with(&line[..start]) {
                    // Get the next word in the command after current position
                    let remaining = &cmd[start..];
                    if let Some(next_space) = remaining.find(' ') {
                        let next_word = &remaining[..next_space];
                        next_word.starts_with(current_word)
                    } else {
                        remaining.starts_with(current_word)
                    }
                } else {
                    // Single word commands
                    start == 0 && cmd.starts_with(current_word)
                }
            })
            .map(|cmd| {
                // Extract just the word we're completing
                let remaining = &cmd[start..];
                let next_word = if let Some(space_pos) = remaining.find(' ') {
                    &remaining[..space_pos]
                } else {
                    remaining
                };
                
                Pair {
                    display: next_word.to_string(),
                    replacement: next_word[current_word.len()..].to_string(),
                }
            })
            .collect();

        Ok((pos, matches))
    }
}

impl Default for FsCliCompleter {
    fn default() -> Self {
        Self::new()
    }
}

impl Helper for FsCliCompleter {}

impl Completer for FsCliCompleter {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        // Try command completion first
        let (start, candidates) = self.complete_command(line, pos)?;

        // If no command matches and we're completing a path-like string, try filename completion
        if candidates.is_empty() && (line.contains('/') || line.contains('\\')) {
            let (file_start, file_candidates) = self.filename_completer.complete(line, pos, ctx)?;
            return Ok((file_start, file_candidates));
        }

        Ok((start, candidates))
    }
}

impl Hinter for FsCliCompleter {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<Self::Hint> {
        self.history_hinter.hint(line, pos, ctx)
    }
}

impl Highlighter for FsCliCompleter {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            Borrowed(prompt)
        } else {
            Borrowed(prompt)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Owned(format!("\x1b[90m{}\x1b[0m", hint)) // Gray hint
    }

    fn highlight<'l>(&self, line: &'l str, pos: usize) -> Cow<'l, str> {
        self.bracket_highlighter.highlight(line, pos)
    }

    fn highlight_char(&self, line: &str, pos: usize, kind: CmdKind) -> bool {
        self.bracket_highlighter.highlight_char(line, pos, kind)
    }
}

impl Validator for FsCliCompleter {
    fn validate(
        &self,
        ctx: &mut validate::ValidationContext,
    ) -> rustyline::Result<validate::ValidationResult> {
        self.bracket_validator.validate(ctx)
    }

    fn validate_while_typing(&self) -> bool {
        self.bracket_validator.validate_while_typing()
    }
}
