//! ESL client-side debug logging functionality
//!
//! Implements debug levels similar to the original fs_cli -d option (0-7)
//! for controlling ESL protocol message logging on the client side.

use std::fmt;
use std::str::FromStr;

/// ESL client-side debug levels (0-7)
/// Matches the original fs_cli esl_global_set_default_logger levels
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum EslDebugLevel {
    None = 0,    // No debug output
    Error = 1,   // Error messages only
    Warning = 2, // Error and warning messages
    Info = 3,    // Error, warning, and info messages
    Debug = 4,   // Basic debug output
    Debug5 = 5,  // More verbose debug
    Debug6 = 6,  // Very verbose debug
    Debug7 = 7,  // Maximum debug (all ESL protocol messages)
}

impl FromStr for EslDebugLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "0" => Ok(EslDebugLevel::None),
            "1" => Ok(EslDebugLevel::Error),
            "2" => Ok(EslDebugLevel::Warning),
            "3" => Ok(EslDebugLevel::Info),
            "4" => Ok(EslDebugLevel::Debug),
            "5" => Ok(EslDebugLevel::Debug5),
            "6" => Ok(EslDebugLevel::Debug6),
            "7" => Ok(EslDebugLevel::Debug7),
            _ => Err(format!("Invalid ESL debug level: {} (must be 0-7)", s)),
        }
    }
}

impl EslDebugLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            EslDebugLevel::None => "0",
            EslDebugLevel::Error => "1",
            EslDebugLevel::Warning => "2",
            EslDebugLevel::Info => "3",
            EslDebugLevel::Debug => "4",
            EslDebugLevel::Debug5 => "5",
            EslDebugLevel::Debug6 => "6",
            EslDebugLevel::Debug7 => "7",
        }
    }

    /// Get tracing filter level for this debug level
    pub fn tracing_filter(&self) -> &'static str {
        match self {
            EslDebugLevel::None => "error",
            EslDebugLevel::Error => "error",
            EslDebugLevel::Warning => "warn",
            EslDebugLevel::Info => "info",
            EslDebugLevel::Debug
            | EslDebugLevel::Debug5
            | EslDebugLevel::Debug6
            | EslDebugLevel::Debug7 => "debug",
        }
    }

    /// Debug print if level is high enough
    pub fn debug_print(&self, level: EslDebugLevel, message: &str) {
        if *self >= level {
            eprintln!("[ESL_DEBUG:{}] {}", level.as_str(), message);
        }
    }
}

impl Default for EslDebugLevel {
    fn default() -> Self {
        EslDebugLevel::None
    }
}

impl fmt::Display for EslDebugLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
