use std::fmt;
use std::str::FromStr;

use super::{extract_variables, write_variables};
use crate::commands::originate::OriginateError;
use crate::commands::variables::Variables;

wire_enum! {
    /// Distribution order for group_call dial strings.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum GroupCallOrder {
        /// Ring all members simultaneously.
        All => "A",
        /// Enterprise-style hunt (try each in order, across domains).
        Enterprise => "E",
        /// Ring first available member only.
        First => "F",
    }
    error ParseGroupCallOrderError("group call order");
    tests: group_call_order_tests;
}

/// Runtime expression resolving directory group members:
/// `${group_call(group@domain[+order])}`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct GroupCall {
    /// Group name from the directory.
    pub group: String,
    /// Domain for the group lookup.
    pub domain: String,
    /// Distribution order.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub order: Option<GroupCallOrder>,
    /// Per-channel variables prepended as `{key=value}`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub variables: Option<Variables>,
}

impl GroupCall {
    /// Create a new group_call expression.
    pub fn new(group: impl Into<String>, domain: impl Into<String>) -> Self {
        Self {
            group: group.into(),
            domain: domain.into(),
            order: None,
            variables: None,
        }
    }

    /// Set the distribution order.
    pub fn with_order(mut self, order: GroupCallOrder) -> Self {
        self.order = Some(order);
        self
    }

    /// Set per-channel variables.
    pub fn with_variables(mut self, variables: Variables) -> Self {
        self.variables = Some(variables);
        self
    }
}

impl fmt::Display for GroupCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_variables(f, &self.variables)?;
        match &self.order {
            Some(o) => write!(f, "${{group_call({}@{}+{})}}", self.group, self.domain, o),
            None => write!(f, "${{group_call({}@{})}}", self.group, self.domain),
        }
    }
}

impl FromStr for GroupCall {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (variables, uri) = extract_variables(s)?;
        let inner = uri
            .strip_prefix("${group_call(")
            .and_then(|r| r.strip_suffix(")}"))
            .ok_or_else(|| OriginateError::ParseError("not a group_call expression".into()))?;
        let (group_at_domain, order) = if let Some((gd, o)) = inner.split_once('+') {
            let order: GroupCallOrder = o
                .parse()
                .map_err(|_| OriginateError::ParseError(format!("unknown order: {}", o)))?;
            (gd, Some(order))
        } else {
            (inner, None)
        };
        let (group, domain) = group_at_domain
            .split_once('@')
            .ok_or_else(|| OriginateError::ParseError("group_call needs group@domain".into()))?;
        Ok(Self {
            group: group.into(),
            domain: domain.into(),
            order,
            variables,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_call_display() {
        let ep = GroupCall::new("support", "example.com");
        assert_eq!(ep.to_string(), "${group_call(support@example.com)}");
    }

    #[test]
    fn group_call_display_with_order() {
        let ep = GroupCall::new("support", "example.com").with_order(GroupCallOrder::All);
        assert_eq!(ep.to_string(), "${group_call(support@example.com+A)}");
    }

    #[test]
    fn group_call_from_str() {
        let ep: GroupCall = "${group_call(support@example.com)}"
            .parse()
            .unwrap();
        assert_eq!(ep.group, "support");
        assert_eq!(ep.domain, "example.com");
        assert!(ep
            .order
            .is_none());
    }

    #[test]
    fn group_call_from_str_with_order() {
        let ep: GroupCall = "${group_call(support@example.com+A)}"
            .parse()
            .unwrap();
        assert_eq!(ep.group, "support");
        assert_eq!(ep.domain, "example.com");
        assert_eq!(ep.order, Some(GroupCallOrder::All));
    }

    #[test]
    fn group_call_round_trip() {
        let ep = GroupCall::new("calltakers", "example.com").with_order(GroupCallOrder::Enterprise);
        let s = ep.to_string();
        let parsed: GroupCall = s
            .parse()
            .unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_group_call() {
        let ep = GroupCall::new("support", "example.com").with_order(GroupCallOrder::All);
        let json = serde_json::to_string(&ep).unwrap();
        let parsed: GroupCall = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }
}
