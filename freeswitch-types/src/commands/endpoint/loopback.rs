use std::fmt;
use std::str::FromStr;

use super::{strip_endpoint_prefix, write_variables};
use crate::commands::originate::OriginateError;
use crate::commands::variables::DialStringCarrier;
use crate::commands::variables::Variables;

/// Internal loopback endpoint: `loopback/{extension}[/{context}]`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct LoopbackEndpoint {
    /// Extension number or pattern.
    pub extension: String,
    /// Dialplan context. `None` omits the context segment, letting
    /// FreeSWITCH use its default.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub context: Option<String>,
    /// Per-channel variables prepended as `{key=value}`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub variables: Option<Variables>,
}

impl LoopbackEndpoint {
    /// Create a new loopback endpoint with no explicit context.
    pub fn new(extension: impl Into<String>) -> Self {
        Self {
            extension: extension.into(),
            context: None,
            variables: None,
        }
    }

    /// Set the dialplan context.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// Set per-channel variables.
    pub fn with_variables(mut self, variables: Variables) -> Self {
        self.variables = Some(variables);
        self
    }
}

impl LoopbackEndpoint {
    pub(super) fn write_for(
        &self,
        f: &mut fmt::Formatter<'_>,
        carrier: DialStringCarrier,
    ) -> fmt::Result {
        write_variables(f, &self.variables, carrier)?;
        match &self.context {
            Some(ctx) => write!(f, "loopback/{}/{}", self.extension, ctx),
            None => write!(f, "loopback/{}", self.extension),
        }
    }
}

impl fmt::Display for LoopbackEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_for(f, DialStringCarrier::EslApi)
    }
}

impl FromStr for LoopbackEndpoint {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (variables, rest) =
            strip_endpoint_prefix(s, "loopback/", "loopback", DialStringCarrier::EslApi)?;
        let (extension, context) = match rest.split_once('/') {
            Some((ext, ctx)) => (ext, Some(ctx.to_string())),
            None => (rest, None),
        };
        Ok(Self {
            extension: extension.into(),
            context,
            variables,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::variables::VariablesType;

    #[test]
    fn loopback_display_no_context() {
        let ep = LoopbackEndpoint::new("9199");
        assert_eq!(ep.to_string(), "loopback/9199");
    }

    #[test]
    fn loopback_display_with_context() {
        let ep = LoopbackEndpoint::new("9199").with_context("default");
        assert_eq!(ep.to_string(), "loopback/9199/default");
    }

    #[test]
    fn loopback_display_with_variables() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("loopback_initial_codec", "L16@48000h");
        let ep = LoopbackEndpoint::new("100")
            .with_context("test")
            .with_variables(vars);
        assert_eq!(
            ep.to_string(),
            "{loopback_initial_codec=L16@48000h}loopback/100/test"
        );
    }

    #[test]
    fn loopback_from_str_with_context() {
        let ep: LoopbackEndpoint = "loopback/9199/test"
            .parse()
            .unwrap();
        assert_eq!(ep.extension, "9199");
        assert_eq!(
            ep.context
                .as_deref(),
            Some("test")
        );
    }

    #[test]
    fn loopback_from_str_no_context() {
        let ep: LoopbackEndpoint = "loopback/9199"
            .parse()
            .unwrap();
        assert_eq!(ep.extension, "9199");
        assert!(ep
            .context
            .is_none());
    }

    #[test]
    fn loopback_round_trip_with_context() {
        let ep = LoopbackEndpoint::new("100").with_context("myctx");
        let s = ep.to_string();
        let parsed: LoopbackEndpoint = s
            .parse()
            .unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn loopback_round_trip_no_context() {
        let ep = LoopbackEndpoint::new("9199");
        let s = ep.to_string();
        let parsed: LoopbackEndpoint = s
            .parse()
            .unwrap();
        assert_eq!(parsed, ep);
    }

    // --- T5: LoopbackEndpoint parse -> display asymmetry ---
    // Display omits context when None, but FromStr always produces
    // context=None for bare "loopback/ext". Round-trip is symmetric.

    #[test]
    fn loopback_display_parse_display_stable() {
        let inputs = [
            "loopback/9199",
            "loopback/100/default",
            "loopback/ext123/custom_ctx",
        ];
        for input in inputs {
            let parsed: LoopbackEndpoint = input
                .parse()
                .unwrap();
            let displayed = parsed.to_string();
            assert_eq!(displayed, input, "round-trip failed for: {}", input);
            let reparsed: LoopbackEndpoint = displayed
                .parse()
                .unwrap();
            assert_eq!(reparsed, parsed);
        }
    }

    #[test]
    fn serde_loopback_endpoint() {
        let ep = LoopbackEndpoint::new("9199").with_context("default");
        let json = serde_json::to_string(&ep).unwrap();
        let parsed: LoopbackEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }
}
