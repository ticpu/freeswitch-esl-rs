//! Channel variable types: format parsers (`ARRAY::`, SIP multipart) and typed
//! variable name enums.

mod conference;
mod core;
mod core_media;
mod esl_array;
#[cfg(feature = "esl")]
mod esl_headers;
mod loopback;
mod sip_multipart;
mod sip_passthrough;
mod sofia;

pub use self::core::{ChannelVariable, ParseChannelVariableError};
pub use conference::{ConferenceVariable, ParseConferenceVariableError};
pub use core_media::{CoreMediaVariable, ParseCoreMediaVariableError, RtpStatUnit};
pub use esl_array::{EslArray, EslArrayError, MAX_ARRAY_ITEMS};
#[cfg(feature = "esl")]
pub use esl_headers::EslHeaders;
pub use loopback::{
    LoopbackHangupCause, LoopbackResignation, LoopbackVariable, ParseLoopbackHangupCauseError,
    ParseLoopbackVariableError,
};
pub use sip_multipart::{MultipartBody, MultipartItem};
pub use sip_passthrough::{
    InvalidHeaderName, ParseSipPassthroughError, SipHeaderPrefix, SipPassthroughHeader,
};
pub use sofia::{CarriedHeader, ParseSofiaVariableError, SofiaVariable};

/// Trait for typed channel variable name enums.
///
/// Implement this on variable name enums to use them with
/// [`HeaderLookup::variable()`](crate::HeaderLookup::variable) and
/// [`variable_str()`](crate::HeaderLookup::variable_str).
/// For variables not covered by any typed enum, use `variable_str()`.
pub trait VariableName {
    /// Wire-format variable name (e.g. `"sip_call_id"`).
    fn as_str(&self) -> &str;
}

impl VariableName for ChannelVariable {
    fn as_str(&self) -> &str {
        ChannelVariable::as_str(self)
    }
}

impl VariableName for SofiaVariable {
    fn as_str(&self) -> &str {
        SofiaVariable::as_str(self)
    }
}

impl VariableName for CoreMediaVariable {
    fn as_str(&self) -> &str {
        CoreMediaVariable::as_str(self)
    }
}

impl VariableName for LoopbackVariable {
    fn as_str(&self) -> &str {
        LoopbackVariable::as_str(self)
    }
}

impl VariableName for ConferenceVariable {
    fn as_str(&self) -> &str {
        ConferenceVariable::as_str(self)
    }
}
