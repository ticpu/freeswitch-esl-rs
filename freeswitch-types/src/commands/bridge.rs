//! Bridge dial string builder for multi-endpoint bridge commands.
//!
//! Supports simultaneous ring (`,`) and sequential failover (`|`)
//! with per-endpoint channel variables and global default variables.

use std::fmt;
use std::str::FromStr;

use super::endpoint::Endpoint;
use super::find_matching_bracket;
use super::originate::OriginateError;
use super::variables::Variables;

/// Typed bridge dial string.
///
/// Format: `{global_vars}[ep1_vars]ep1,[ep2_vars]ep2|[ep3_vars]ep3`
///
/// - `,` separates endpoints rung simultaneously (within a group)
/// - `|` separates groups tried sequentially (failover)
/// - Each endpoint may have channel-scope `[variables]`
/// - Global `{variables}` apply to all endpoints
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BridgeDialString {
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    variables: Option<Variables>,
    groups: Vec<Vec<Endpoint>>,
}

impl BridgeDialString {
    /// Create a new bridge dial string with the given failover groups.
    pub fn new(groups: Vec<Vec<Endpoint>>) -> Self {
        Self {
            variables: None,
            groups,
        }
    }

    /// Set global default-scope variables.
    pub fn with_variables(mut self, variables: Variables) -> Self {
        self.variables = Some(variables);
        self
    }

    /// Default-scope variables applied to all endpoints.
    pub fn variables(&self) -> Option<&Variables> {
        self.variables
            .as_ref()
    }

    /// Sequential failover groups (`|`-separated). Within each group,
    /// endpoints ring simultaneously (`,`-separated).
    pub fn groups(&self) -> &[Vec<Endpoint>] {
        &self.groups
    }

    /// Mutable reference to the default-scope variables.
    pub fn variables_mut(&mut self) -> &mut Option<Variables> {
        &mut self.variables
    }

    /// Mutable reference to the failover groups.
    pub fn groups_mut(&mut self) -> &mut Vec<Vec<Endpoint>> {
        &mut self.groups
    }
}

impl fmt::Display for BridgeDialString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(vars) = &self.variables {
            if !vars.is_empty() {
                write!(f, "{}", vars)?;
            }
        }
        for (gi, group) in self
            .groups
            .iter()
            .enumerate()
        {
            if gi > 0 {
                f.write_str("|")?;
            }
            for (ei, ep) in group
                .iter()
                .enumerate()
            {
                if ei > 0 {
                    f.write_str(",")?;
                }
                write!(f, "{}", ep)?;
            }
        }
        Ok(())
    }
}

impl FromStr for BridgeDialString {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(OriginateError::ParseError(
                "empty bridge dial string".into(),
            ));
        }

        // Extract leading {global_vars} if present
        let (variables, rest) = if s.starts_with('{') {
            let close = find_matching_bracket(s, '{', '}').ok_or_else(|| {
                OriginateError::ParseError("unclosed { in bridge dial string".into())
            })?;
            let var_str = &s[..=close];
            let vars: Variables = var_str.parse()?;
            let vars = if vars.is_empty() { None } else { Some(vars) };
            (vars, &s[close + 1..])
        } else {
            (None, s)
        };

        // Split on | for sequential groups, respecting brackets
        let group_strs = split_respecting_brackets(rest, '|');
        let mut groups = Vec::new();
        for group_str in &group_strs {
            let group_str = group_str.trim();
            if group_str.is_empty() {
                continue;
            }
            // Split on , for simultaneous endpoints, respecting brackets
            let ep_strs = split_respecting_brackets(group_str, ',');
            let mut endpoints = Vec::new();
            for ep_str in &ep_strs {
                let ep_str = ep_str.trim();
                if ep_str.is_empty() {
                    continue;
                }
                let ep: Endpoint = ep_str.parse()?;
                endpoints.push(ep);
            }
            if !endpoints.is_empty() {
                groups.push(endpoints);
            }
        }

        Ok(Self { variables, groups })
    }
}

/// Split a string on `sep` while skipping separators inside `{...}`, `[...]`,
/// `<...>`, and `${...}` blocks.
fn split_respecting_brackets(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let bytes = s.as_bytes();

    for (i, &b) in bytes
        .iter()
        .enumerate()
    {
        match b {
            b'{' | b'[' | b'<' | b'(' => depth += 1,
            b'}' | b']' | b'>' | b')' => {
                depth -= 1;
                if depth < 0 {
                    depth = 0;
                }
            }
            _ if b == sep as u8 && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::endpoint::{ErrorEndpoint, LoopbackEndpoint, SofiaEndpoint, SofiaGateway};
    use crate::commands::variables::VariablesType;

    // === Display ===

    #[test]
    fn display_single_endpoint() {
        let bridge =
            BridgeDialString::new(vec![vec![
                SofiaGateway::new("my_provider", "18005551234").into()
            ]]);
        assert_eq!(bridge.to_string(), "sofia/gateway/my_provider/18005551234");
    }

    #[test]
    fn display_simultaneous_ring() {
        let bridge = BridgeDialString::new(vec![vec![
            SofiaGateway::new("primary", "18005551234").into(),
            SofiaGateway::new("secondary", "18005551234").into(),
        ]]);
        assert_eq!(
            bridge.to_string(),
            "sofia/gateway/primary/18005551234,sofia/gateway/secondary/18005551234"
        );
    }

    #[test]
    fn display_sequential_failover() {
        let bridge = BridgeDialString::new(vec![
            vec![SofiaGateway::new("primary", "18005551234").into()],
            vec![SofiaGateway::new("backup", "18005551234").into()],
        ]);
        assert_eq!(
            bridge.to_string(),
            "sofia/gateway/primary/18005551234|sofia/gateway/backup/18005551234"
        );
    }

    #[test]
    fn display_mixed_simultaneous_and_sequential() {
        let bridge = BridgeDialString::new(vec![
            vec![
                SofiaGateway::new("primary", "1234").into(),
                SofiaGateway::new("secondary", "1234").into(),
            ],
            vec![SofiaGateway::new("backup", "1234").into()],
        ]);
        assert_eq!(
            bridge.to_string(),
            "sofia/gateway/primary/1234,sofia/gateway/secondary/1234|sofia/gateway/backup/1234"
        );
    }

    #[test]
    fn display_with_global_variables() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("hangup_after_bridge", "true");
        let bridge =
            BridgeDialString::new(vec![vec![
                SofiaEndpoint::new("internal", "1000@domain").into()
            ]])
            .with_variables(vars);
        assert_eq!(
            bridge.to_string(),
            "{hangup_after_bridge=true}sofia/internal/1000@domain"
        );
    }

    #[test]
    fn display_with_per_endpoint_variables() {
        let mut ep_vars = Variables::new(VariablesType::Channel);
        ep_vars.insert("leg_timeout", "30");
        let bridge = BridgeDialString::new(vec![vec![
            SofiaGateway::new("gw1", "1234")
                .with_variables(ep_vars)
                .into(),
            SofiaGateway::new("gw2", "1234").into(),
        ]]);
        assert_eq!(
            bridge.to_string(),
            "[leg_timeout=30]sofia/gateway/gw1/1234,sofia/gateway/gw2/1234"
        );
    }

    #[test]
    fn display_with_error_endpoint_failover() {
        let bridge = BridgeDialString::new(vec![
            vec![SofiaGateway::new("primary", "1234").into()],
            vec![ErrorEndpoint::new(crate::channel::HangupCause::UserBusy).into()],
        ]);
        assert_eq!(
            bridge.to_string(),
            "sofia/gateway/primary/1234|error/USER_BUSY"
        );
    }

    #[test]
    fn display_with_loopback() {
        let bridge = BridgeDialString::new(vec![vec![LoopbackEndpoint::new("9199")
            .with_context("default")
            .into()]]);
        assert_eq!(bridge.to_string(), "loopback/9199/default");
    }

    // === FromStr ===

    #[test]
    fn from_str_single_endpoint() {
        let bridge: BridgeDialString = "sofia/gateway/my_provider/18005551234"
            .parse()
            .unwrap();
        assert_eq!(
            bridge
                .groups()
                .len(),
            1
        );
        assert_eq!(bridge.groups()[0].len(), 1);
        assert!(bridge
            .variables()
            .is_none());
    }

    #[test]
    fn from_str_simultaneous_ring() {
        let bridge: BridgeDialString = "sofia/gateway/primary/1234,sofia/gateway/secondary/1234"
            .parse()
            .unwrap();
        assert_eq!(
            bridge
                .groups()
                .len(),
            1
        );
        assert_eq!(bridge.groups()[0].len(), 2);
    }

    #[test]
    fn from_str_sequential_failover() {
        let bridge: BridgeDialString = "sofia/gateway/primary/1234|sofia/gateway/backup/1234"
            .parse()
            .unwrap();
        assert_eq!(
            bridge
                .groups()
                .len(),
            2
        );
        assert_eq!(bridge.groups()[0].len(), 1);
        assert_eq!(bridge.groups()[1].len(), 1);
    }

    #[test]
    fn from_str_mixed() {
        let bridge: BridgeDialString =
            "sofia/gateway/primary/1234,sofia/gateway/secondary/1234|sofia/gateway/backup/1234"
                .parse()
                .unwrap();
        assert_eq!(
            bridge
                .groups()
                .len(),
            2
        );
        assert_eq!(bridge.groups()[0].len(), 2);
        assert_eq!(bridge.groups()[1].len(), 1);
    }

    #[test]
    fn from_str_with_global_variables() {
        let bridge: BridgeDialString = "{hangup_after_bridge=true}sofia/internal/1000@domain"
            .parse()
            .unwrap();
        assert!(bridge
            .variables()
            .is_some());
        assert_eq!(
            bridge
                .variables()
                .unwrap()
                .get("hangup_after_bridge"),
            Some("true")
        );
        assert_eq!(
            bridge
                .groups()
                .len(),
            1
        );
        assert_eq!(bridge.groups()[0].len(), 1);
    }

    #[test]
    fn from_str_with_per_endpoint_variables() {
        let bridge: BridgeDialString =
            "[leg_timeout=30]sofia/gateway/gw1/1234,sofia/gateway/gw2/1234"
                .parse()
                .unwrap();
        assert_eq!(
            bridge
                .groups()
                .len(),
            1
        );
        assert_eq!(bridge.groups()[0].len(), 2);
        let ep = &bridge.groups()[0][0];
        if let Endpoint::SofiaGateway(gw) = ep {
            assert!(gw
                .variables
                .is_some());
        } else {
            panic!("expected SofiaGateway");
        }
    }

    #[test]
    fn from_str_round_trip_single() {
        let input = "sofia/gateway/my_provider/18005551234";
        let bridge: BridgeDialString = input
            .parse()
            .unwrap();
        assert_eq!(bridge.to_string(), input);
    }

    #[test]
    fn from_str_round_trip_mixed() {
        let input =
            "sofia/gateway/primary/1234,sofia/gateway/secondary/1234|sofia/gateway/backup/1234";
        let bridge: BridgeDialString = input
            .parse()
            .unwrap();
        assert_eq!(bridge.to_string(), input);
    }

    #[test]
    fn from_str_round_trip_with_global_vars() {
        let input = "{hangup_after_bridge=true}sofia/internal/1000@domain";
        let bridge: BridgeDialString = input
            .parse()
            .unwrap();
        assert_eq!(bridge.to_string(), input);
    }

    // === Serde ===

    #[test]
    fn serde_round_trip_single() {
        let bridge =
            BridgeDialString::new(vec![vec![
                SofiaGateway::new("my_provider", "18005551234").into()
            ]]);
        let json = serde_json::to_string(&bridge).unwrap();
        let parsed: BridgeDialString = serde_json::from_str(&json).unwrap();
        assert_eq!(bridge, parsed);
    }

    #[test]
    fn serde_round_trip_multi_group() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("hangup_after_bridge", "true");
        let bridge = BridgeDialString::new(vec![
            vec![
                SofiaGateway::new("primary", "1234").into(),
                SofiaGateway::new("secondary", "1234").into(),
            ],
            vec![ErrorEndpoint::new(crate::channel::HangupCause::UserBusy).into()],
        ])
        .with_variables(vars);
        let json = serde_json::to_string(&bridge).unwrap();
        let parsed: BridgeDialString = serde_json::from_str(&json).unwrap();
        assert_eq!(bridge, parsed);
    }

    // --- T5: BridgeDialString edge cases ---

    #[test]
    fn from_str_empty_string_rejected() {
        let result = "".parse::<BridgeDialString>();
        assert!(result.is_err());
    }

    #[test]
    fn from_str_whitespace_only_rejected() {
        let result = "   ".parse::<BridgeDialString>();
        assert!(result.is_err());
    }

    #[test]
    fn from_str_empty_groups_from_trailing_pipe() {
        // "ep1|" should parse as one group (empty trailing group is skipped)
        let bridge: BridgeDialString = "sofia/gateway/gw1/1234|"
            .parse()
            .unwrap();
        assert_eq!(
            bridge
                .groups()
                .len(),
            1
        );
    }

    #[test]
    fn from_str_empty_variable_block() {
        let bridge: BridgeDialString = "{}sofia/gateway/gw1/1234"
            .parse()
            .unwrap();
        assert!(bridge
            .variables()
            .is_none());
        assert_eq!(
            bridge
                .groups()
                .len(),
            1
        );
    }

    #[test]
    fn from_str_mismatched_bracket_rejected() {
        let result = "{unclosed=true sofia/gateway/gw1/1234".parse::<BridgeDialString>();
        assert!(result.is_err());
    }

    #[test]
    fn serde_to_display_wire_format() {
        let json = r#"{
            "groups": [[{
                "sofia_gateway": {
                    "gateway": "my_gw",
                    "destination": "18005551234"
                }
            }]]
        }"#;
        let bridge: BridgeDialString = serde_json::from_str(json).unwrap();
        assert_eq!(bridge.to_string(), "sofia/gateway/my_gw/18005551234");
    }
}
