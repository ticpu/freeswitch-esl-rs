//! Gateway registration state and ping status enums.

wire_enum! {
    /// Gateway registration state from `sofia::gateway_state` events.
    ///
    /// The `State` header value, mapping to `reg_state_t` / `sofia_state_names[]`
    /// in mod_sofia.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
    pub enum GatewayRegState {
        #[cfg_attr(feature = "serde", serde(alias = "Unreged"))]
        Unreged => "UNREGED",
        #[cfg_attr(feature = "serde", serde(alias = "Trying"))]
        Trying => "TRYING",
        #[cfg_attr(feature = "serde", serde(alias = "Register"))]
        Register => "REGISTER",
        #[cfg_attr(feature = "serde", serde(alias = "Reged"))]
        Reged => "REGED",
        #[cfg_attr(feature = "serde", serde(alias = "Unregister"))]
        Unregister => "UNREGISTER",
        #[cfg_attr(feature = "serde", serde(alias = "Failed"))]
        Failed => "FAILED",
        #[cfg_attr(feature = "serde", serde(alias = "FailWait"))]
        FailWait => "FAIL_WAIT",
        #[cfg_attr(feature = "serde", serde(alias = "Expired"))]
        Expired => "EXPIRED",
        #[cfg_attr(feature = "serde", serde(alias = "Noreg"))]
        Noreg => "NOREG",
        #[cfg_attr(feature = "serde", serde(alias = "Down"))]
        Down => "DOWN",
        #[cfg_attr(feature = "serde", serde(alias = "Timeout"))]
        Timeout => "TIMEOUT",
    }
    error ParseGatewayRegStateError("gateway reg state");
    tests: gateway_reg_state_wire_tests;
}

wire_enum! {
    /// Gateway ping status from `sofia::gateway_state` events.
    ///
    /// The `Ping-Status` header value, mapping to `sofia_gateway_status_name()`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
    pub enum GatewayPingStatus {
        #[cfg_attr(feature = "serde", serde(alias = "Down"))]
        Down => "DOWN",
        #[cfg_attr(feature = "serde", serde(alias = "Up"))]
        Up => "UP",
        #[cfg_attr(feature = "serde", serde(alias = "Invalid"))]
        Invalid => "INVALID",
    }
    error ParseGatewayPingStatusError("gateway ping status");
    tests: gateway_ping_status_wire_tests;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "serde")]
    mod serde_tests {
        use super::*;

        #[test]
        fn reg_state_serde_uses_wire_format() {
            for &state in GatewayRegState::ALL {
                let json = serde_json::to_string(&state).unwrap();
                let expected = format!("\"{}\"", state.as_str());
                assert_eq!(json, expected, "serde must serialize as wire format");
            }
        }

        #[test]
        fn reg_state_serde_round_trip() {
            for &state in GatewayRegState::ALL {
                let json = serde_json::to_string(&state).unwrap();
                let parsed: GatewayRegState = serde_json::from_str(&json).unwrap();
                assert_eq!(parsed, state);
            }
        }

        #[test]
        fn reg_state_serde_accepts_pascal_case_alias() {
            let cases = [
                ("\"Noreg\"", GatewayRegState::Noreg),
                ("\"Reged\"", GatewayRegState::Reged),
                ("\"FailWait\"", GatewayRegState::FailWait),
                ("\"Unreged\"", GatewayRegState::Unreged),
            ];
            for (json, expected) in cases {
                let parsed: GatewayRegState = serde_json::from_str(json).unwrap();
                assert_eq!(parsed, expected, "PascalCase alias failed for {json}");
            }
        }

        #[test]
        fn ping_status_serde_uses_wire_format() {
            for &status in GatewayPingStatus::ALL {
                let json = serde_json::to_string(&status).unwrap();
                let expected = format!("\"{}\"", status.as_str());
                assert_eq!(json, expected, "serde must serialize as wire format");
            }
        }

        #[test]
        fn ping_status_serde_round_trip() {
            for &status in GatewayPingStatus::ALL {
                let json = serde_json::to_string(&status).unwrap();
                let parsed: GatewayPingStatus = serde_json::from_str(&json).unwrap();
                assert_eq!(parsed, status);
            }
        }

        #[test]
        fn ping_status_serde_accepts_pascal_case_alias() {
            let cases = [
                ("\"Down\"", GatewayPingStatus::Down),
                ("\"Up\"", GatewayPingStatus::Up),
                ("\"Invalid\"", GatewayPingStatus::Invalid),
            ];
            for (json, expected) in cases {
                let parsed: GatewayPingStatus = serde_json::from_str(json).unwrap();
                assert_eq!(parsed, expected, "PascalCase alias failed for {json}");
            }
        }
    }
}
