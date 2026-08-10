use std::fmt;

use super::{extract_variables, write_variables};
use crate::commands::originate::OriginateError;
use crate::commands::variables::DialStringCarrier;
use crate::commands::variables::Variables;

/// Audio device endpoint for portaudio, pulseaudio, or ALSA modules.
///
/// Wire format: `{module}[/{destination}]` where destination is typically
/// empty or `auto_answer` (recognized by portaudio and pulseaudio).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct AudioEndpoint {
    /// Destination string (e.g. `auto_answer`). `None` for bare module name.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub destination: Option<String>,
    /// Per-channel variables prepended as `{key=value}`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub variables: Option<Variables>,
}

impl AudioEndpoint {
    /// Create a new audio endpoint with no destination.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the destination string.
    pub fn with_destination(mut self, destination: impl Into<String>) -> Self {
        self.destination = Some(destination.into());
        self
    }

    /// Set per-channel variables.
    pub fn with_variables(mut self, variables: Variables) -> Self {
        self.variables = Some(variables);
        self
    }

    /// Format with the given module prefix (`portaudio`, `pulseaudio`, `alsa`),
    /// which the [`Endpoint`](super::Endpoint) variant supplies because this
    /// struct does not record which of the three modules it belongs to.
    ///
    /// Renders for [`DialStringCarrier::EslApi`]; the [`Endpoint`](super::Endpoint)
    /// variants render for whichever carrier they were asked for.
    pub fn fmt_with_prefix(&self, f: &mut fmt::Formatter<'_>, prefix: &str) -> fmt::Result {
        self.write_with_prefix(f, prefix, DialStringCarrier::EslApi)
    }

    pub(super) fn write_with_prefix(
        &self,
        f: &mut fmt::Formatter<'_>,
        prefix: &str,
        carrier: DialStringCarrier,
    ) -> fmt::Result {
        write_variables(f, &self.variables, carrier)?;
        match &self.destination {
            Some(dest) => write!(f, "{}/{}", prefix, dest),
            None => f.write_str(prefix),
        }
    }

    /// Parse from a dial string with the given module prefix.
    pub fn parse_with_prefix(s: &str, prefix: &str) -> Result<Self, OriginateError> {
        let (variables, uri) = extract_variables(s, DialStringCarrier::EslApi)?;
        let rest = uri
            .strip_prefix(prefix)
            .ok_or_else(|| OriginateError::ParseError(format!("not a {} endpoint", prefix)))?;
        let destination = rest
            .strip_prefix('/')
            .and_then(|d| {
                if d.is_empty() {
                    None
                } else {
                    Some(d.to_string())
                }
            });
        Ok(Self {
            destination,
            variables,
        })
    }
}

/// **Warning:** This `Display` impl exists only to satisfy the `DialString: Display`
/// trait bound. The `"audio"` prefix is not a valid FreeSWITCH endpoint.
/// Always use `AudioEndpoint` through [`Endpoint::PortAudio`](super::Endpoint::PortAudio),
/// [`Endpoint::PulseAudio`](super::Endpoint::PulseAudio), or
/// [`Endpoint::Alsa`](super::Endpoint::Alsa) which call
/// [`fmt_with_prefix`](AudioEndpoint::fmt_with_prefix) with the correct module name.
impl fmt::Display for AudioEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_with_prefix(f, "audio")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_with_prefix_portaudio() {
        let ep = AudioEndpoint::parse_with_prefix("portaudio/auto_answer", "portaudio").unwrap();
        assert_eq!(
            ep.destination
                .as_deref(),
            Some("auto_answer")
        );
        assert!(ep
            .variables
            .is_none());
    }

    #[test]
    fn parse_with_prefix_bare() {
        let ep = AudioEndpoint::parse_with_prefix("alsa", "alsa").unwrap();
        assert!(ep
            .destination
            .is_none());
    }

    #[test]
    fn parse_with_prefix_trailing_slash() {
        let ep = AudioEndpoint::parse_with_prefix("pulseaudio/", "pulseaudio").unwrap();
        assert!(ep
            .destination
            .is_none());
    }

    #[test]
    fn parse_with_prefix_wrong_module() {
        let result = AudioEndpoint::parse_with_prefix("portaudio/x", "alsa");
        assert!(result.is_err());
    }
}
