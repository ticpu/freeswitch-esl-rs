use super::event_type::EslEventType;
use super::format::EventFormat;
use crate::headers::EventHeader;
use crate::sofia::SofiaEventSubclass;
use std::fmt;

/// Error returned when an [`EventSubscription`] builder method receives invalid input.
///
/// Custom subclasses and filter values are validated against ESL wire-safety
/// constraints: no newlines, carriage returns, or (for subclasses) spaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventSubscriptionError(pub String);

impl fmt::Display for EventSubscriptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid event subscription: {}", self.0)
    }
}

impl std::error::Error for EventSubscriptionError {}

/// Declarative description of an ESL event subscription.
///
/// Captures the event format, event types, custom subclasses, and filters
/// as a single unit. Useful for config-driven subscriptions and reconnection
/// patterns where the caller needs to rebuild subscriptions from a saved
/// description.
///
/// # Wire safety
///
/// Builder methods validate inputs against ESL wire injection risks.
/// Custom subclasses reject `\n`, `\r`, spaces, and empty strings.
/// Filter headers and values reject `\n` and `\r`.
///
/// # Example
///
/// ```rust
/// use freeswitch_types::{EventSubscription, EventFormat, EslEventType, EventHeader};
///
/// let sub = EventSubscription::new(EventFormat::Plain)
///     .events(EslEventType::CHANNEL_EVENTS)
///     .event(EslEventType::Heartbeat)
///     .custom_subclass("sofia::register").unwrap()
///     .filter(EventHeader::CallDirection, "inbound").unwrap();
///
/// assert!(!sub.is_empty());
/// assert!(!sub.is_all());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EventSubscription {
    format: EventFormat,
    events: Vec<EslEventType>,
    raw_events: Vec<String>,
    custom_subclasses: Vec<String>,
    filters: Vec<(String, String)>,
}

/// Wire-safety validator shared by raw events, custom subclasses, and filter fields.
///
/// Newlines/CRs are always rejected (would inject extra ESL commands). When
/// `reject_empty` is set the value must be non-empty. When `reject_space` is
/// set spaces are rejected (token splitting on the wire).
fn validate_wire_token(
    s: &str,
    label: &str,
    reject_empty: bool,
    reject_space: bool,
) -> Result<(), EventSubscriptionError> {
    if reject_empty && s.is_empty() {
        return Err(EventSubscriptionError(format!("{} cannot be empty", label)));
    }
    if crate::wire_safety::contains_wire_terminator(s) {
        return Err(EventSubscriptionError(format!(
            "{} contains newline: {:?}",
            label, s
        )));
    }
    if reject_space && s.contains(' ') {
        return Err(EventSubscriptionError(format!(
            "{} contains space: {:?}",
            label, s
        )));
    }
    Ok(())
}

fn validate_raw_event(s: &str) -> Result<(), EventSubscriptionError> {
    validate_wire_token(s, "raw event", true, true)
}

fn validate_custom_subclass(s: &str) -> Result<(), EventSubscriptionError> {
    validate_wire_token(s, "custom subclass", true, true)
}

fn validate_filter_field(field: &str, label: &str) -> Result<(), EventSubscriptionError> {
    validate_wire_token(field, &format!("filter {}", label), false, false)
}

impl EventSubscription {
    /// Create an empty subscription with the given format.
    pub fn new(format: EventFormat) -> Self {
        Self {
            format,
            events: Vec::new(),
            raw_events: Vec::new(),
            custom_subclasses: Vec::new(),
            filters: Vec::new(),
        }
    }

    /// Create a subscription for all events.
    pub fn all(format: EventFormat) -> Self {
        Self {
            format,
            events: vec![EslEventType::All],
            raw_events: Vec::new(),
            custom_subclasses: Vec::new(),
            filters: Vec::new(),
        }
    }

    /// Add a single event type.
    pub fn event(mut self, event: EslEventType) -> Self {
        self.events
            .push(event);
        self
    }

    /// Add multiple event types (e.g. from group constants like `EslEventType::CHANNEL_EVENTS`).
    pub fn events<T: IntoIterator<Item = impl std::borrow::Borrow<EslEventType>>>(
        mut self,
        events: T,
    ) -> Self {
        self.events
            .extend(
                events
                    .into_iter()
                    .map(|e| *e.borrow()),
            );
        self
    }

    /// Add a single event by wire name.
    ///
    /// Escape hatch for events the [`EslEventType`] enum hasn't yet been
    /// updated to cover. The argument is validated for newline injection,
    /// spaces, and emptiness.
    ///
    /// Raw events appear on the wire alongside typed events when
    /// [`to_event_string()`](Self::to_event_string) is called.
    pub fn event_raw(mut self, event: impl Into<String>) -> Result<Self, EventSubscriptionError> {
        let s = event.into();
        validate_raw_event(&s)?;
        self.raw_events
            .push(s);
        Ok(self)
    }

    /// Add multiple events by wire name.
    ///
    /// Returns `Err` on the first invalid entry.
    pub fn events_raw<I, S>(mut self, events: I) -> Result<Self, EventSubscriptionError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for e in events {
            let s = e.into();
            validate_raw_event(&s)?;
            self.raw_events
                .push(s);
        }
        Ok(self)
    }

    /// Add a custom subclass (e.g. `"sofia::register"`).
    ///
    /// Returns `Err` if the subclass contains spaces, newlines, or is empty.
    pub fn custom_subclass(
        mut self,
        subclass: impl Into<String>,
    ) -> Result<Self, EventSubscriptionError> {
        let s = subclass.into();
        validate_custom_subclass(&s)?;
        self.custom_subclasses
            .push(s);
        Ok(self)
    }

    /// Add multiple custom subclasses.
    ///
    /// Returns `Err` on the first invalid subclass.
    pub fn custom_subclasses(
        mut self,
        subclasses: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, EventSubscriptionError> {
        for s in subclasses {
            let s = s.into();
            validate_custom_subclass(&s)?;
            self.custom_subclasses
                .push(s);
        }
        Ok(self)
    }

    /// Subscribe to a single Sofia event subclass.
    ///
    /// Convenience wrapper around [`custom_subclass()`](Self::custom_subclass) that
    /// accepts a typed [`SofiaEventSubclass`] instead of a raw string.
    pub fn sofia_event(mut self, subclass: SofiaEventSubclass) -> Self {
        self.custom_subclasses
            .push(
                subclass
                    .as_str()
                    .to_string(),
            );
        self
    }

    /// Subscribe to multiple Sofia event subclasses.
    pub fn sofia_events(
        mut self,
        subclasses: impl IntoIterator<Item = impl std::borrow::Borrow<SofiaEventSubclass>>,
    ) -> Self {
        self.custom_subclasses
            .extend(
                subclasses
                    .into_iter()
                    .map(|s| {
                        s.borrow()
                            .as_str()
                            .to_string()
                    }),
            );
        self
    }

    /// Add a filter with a typed header.
    ///
    /// The header enum is always valid; only the value is validated.
    pub fn filter(
        self,
        header: EventHeader,
        value: impl Into<String>,
    ) -> Result<Self, EventSubscriptionError> {
        let v = value.into();
        validate_filter_field(&v, "value")?;
        let mut s = self;
        s.filters
            .push((
                header
                    .as_str()
                    .to_string(),
                v,
            ));
        Ok(s)
    }

    /// Add a filter with raw header and value strings.
    ///
    /// Both header and value are validated against newline injection.
    pub fn filter_raw(
        self,
        header: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, EventSubscriptionError> {
        let h = header.into();
        let v = value.into();
        validate_filter_field(&h, "header")?;
        validate_filter_field(&v, "value")?;
        let mut s = self;
        s.filters
            .push((h, v));
        Ok(s)
    }

    /// Change the event format.
    pub fn with_format(mut self, format: EventFormat) -> Self {
        self.format = format;
        self
    }

    /// The event format.
    pub fn format(&self) -> EventFormat {
        self.format
    }

    /// Mutable reference to the event format.
    pub fn format_mut(&mut self) -> &mut EventFormat {
        &mut self.format
    }

    /// The subscribed event types.
    pub fn event_types(&self) -> &[EslEventType] {
        &self.events
    }

    /// Mutable access to the event types list.
    pub fn event_types_mut(&mut self) -> &mut Vec<EslEventType> {
        &mut self.events
    }

    /// Events subscribed by raw wire name (see [`event_raw`](Self::event_raw)).
    pub fn event_types_raw(&self) -> &[String] {
        &self.raw_events
    }

    /// Mutable access to the raw event list.
    ///
    /// Direct push to this list bypasses [`event_raw`](Self::event_raw)'s
    /// validation. Callers are responsible for ensuring entries contain no
    /// newlines, spaces, or empty strings.
    pub fn event_types_raw_mut(&mut self) -> &mut Vec<String> {
        &mut self.raw_events
    }

    /// The subscribed custom subclasses.
    pub fn custom_subclass_list(&self) -> &[String] {
        &self.custom_subclasses
    }

    /// Mutable access to the custom subclasses list.
    pub fn custom_subclasses_mut(&mut self) -> &mut Vec<String> {
        &mut self.custom_subclasses
    }

    /// The event filters as (header, value) pairs.
    pub fn filters(&self) -> &[(String, String)] {
        &self.filters
    }

    /// Mutable access to the filters list.
    pub fn filters_mut(&mut self) -> &mut Vec<(String, String)> {
        &mut self.filters
    }

    /// Whether the subscription includes all events.
    pub fn is_all(&self) -> bool {
        self.events
            .contains(&EslEventType::All)
    }

    /// Whether the subscription has no events, no raw events, and no
    /// custom subclasses.
    pub fn is_empty(&self) -> bool {
        self.events
            .is_empty()
            && self
                .raw_events
                .is_empty()
            && self
                .custom_subclasses
                .is_empty()
    }

    /// Build the event string for the ESL `event` command.
    ///
    /// Returns `None` if no events, raw events, or custom subclasses are
    /// configured. Returns `Some("ALL")` if `EslEventType::All` is present.
    /// Otherwise returns space-separated typed event names, then raw event
    /// names, with custom subclasses appended after a `CUSTOM` token.
    pub fn to_event_string(&self) -> Option<String> {
        if self
            .events
            .contains(&EslEventType::All)
        {
            return Some("ALL".to_string());
        }

        let mut parts: Vec<&str> = self
            .events
            .iter()
            .map(|e| e.as_str())
            .collect();

        parts.extend(
            self.raw_events
                .iter()
                .map(|s| s.as_str()),
        );

        if !self
            .custom_subclasses
            .is_empty()
        {
            if !self
                .events
                .contains(&EslEventType::Custom)
            {
                parts.push("CUSTOM");
            }
            for sc in &self.custom_subclasses {
                parts.push(sc.as_str());
            }
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        }
    }
}

#[cfg(feature = "serde")]
mod event_subscription_serde {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct EventSubscriptionRaw {
        format: EventFormat,
        #[serde(default)]
        events: Vec<EslEventType>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        raw_events: Vec<String>,
        #[serde(default)]
        custom_subclasses: Vec<String>,
        #[serde(default)]
        filters: Vec<(String, String)>,
    }

    impl TryFrom<EventSubscriptionRaw> for EventSubscription {
        type Error = EventSubscriptionError;

        fn try_from(raw: EventSubscriptionRaw) -> Result<Self, Self::Error> {
            for re in &raw.raw_events {
                validate_raw_event(re)?;
            }
            for sc in &raw.custom_subclasses {
                validate_custom_subclass(sc)?;
            }
            for (h, v) in &raw.filters {
                validate_filter_field(h, "header")?;
                validate_filter_field(v, "value")?;
            }
            Ok(EventSubscription {
                format: raw.format,
                events: raw.events,
                raw_events: raw.raw_events,
                custom_subclasses: raw.custom_subclasses,
                filters: raw.filters,
            })
        }
    }

    impl Serialize for EventSubscription {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let raw = EventSubscriptionRaw {
                format: self.format,
                events: self
                    .events
                    .clone(),
                raw_events: self
                    .raw_events
                    .clone(),
                custom_subclasses: self
                    .custom_subclasses
                    .clone(),
                filters: self
                    .filters
                    .clone(),
            };
            raw.serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for EventSubscription {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let raw = EventSubscriptionRaw::deserialize(deserializer)?;
            EventSubscription::try_from(raw).map_err(serde::de::Error::custom)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_empty() {
        let sub = EventSubscription::new(EventFormat::Plain);
        assert!(sub.is_empty());
        assert!(!sub.is_all());
        assert_eq!(sub.format(), EventFormat::Plain);
        assert!(sub
            .event_types()
            .is_empty());
        assert!(sub
            .custom_subclass_list()
            .is_empty());
        assert!(sub
            .filters()
            .is_empty());
    }

    #[test]
    fn all_creates_all() {
        let sub = EventSubscription::all(EventFormat::Json);
        assert!(sub.is_all());
        assert!(!sub.is_empty());
        assert_eq!(sub.to_event_string(), Some("ALL".to_string()));
    }

    #[test]
    fn event_string_typed_only() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .event(EslEventType::ChannelCreate)
            .event(EslEventType::ChannelAnswer);
        assert_eq!(
            sub.to_event_string(),
            Some("CHANNEL_CREATE CHANNEL_ANSWER".to_string())
        );
    }

    #[test]
    fn event_string_custom_only() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .custom_subclass("sofia::register")
            .unwrap()
            .custom_subclass("sofia::unregister")
            .unwrap();
        assert_eq!(
            sub.to_event_string(),
            Some("CUSTOM sofia::register sofia::unregister".to_string())
        );
    }

    #[test]
    fn event_string_mixed() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .event(EslEventType::Heartbeat)
            .custom_subclass("sofia::register")
            .unwrap();
        assert_eq!(
            sub.to_event_string(),
            Some("HEARTBEAT CUSTOM sofia::register".to_string())
        );
    }

    #[test]
    fn event_string_custom_not_duplicated() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .event(EslEventType::Custom)
            .custom_subclass("sofia::register")
            .unwrap();
        // Should not have "CUSTOM" twice
        assert_eq!(
            sub.to_event_string(),
            Some("CUSTOM sofia::register".to_string())
        );
    }

    #[test]
    fn event_string_empty_is_none() {
        let sub = EventSubscription::new(EventFormat::Plain);
        assert_eq!(sub.to_event_string(), None);
    }

    #[test]
    fn filters_preserve_order() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .filter(EventHeader::CallDirection, "inbound")
            .unwrap()
            .filter_raw("X-Custom", "value1")
            .unwrap()
            .filter(EventHeader::ChannelState, "CS_EXECUTE")
            .unwrap();
        assert_eq!(
            sub.filters(),
            &[
                ("Call-Direction".to_string(), "inbound".to_string()),
                ("X-Custom".to_string(), "value1".to_string()),
                ("Channel-State".to_string(), "CS_EXECUTE".to_string()),
            ]
        );
    }

    #[test]
    fn builder_chain() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .events(EslEventType::CHANNEL_EVENTS)
            .event(EslEventType::Heartbeat)
            .custom_subclass("sofia::register")
            .unwrap()
            .filter(EventHeader::CallDirection, "inbound")
            .unwrap()
            .with_format(EventFormat::Json);

        assert_eq!(sub.format(), EventFormat::Json);
        assert!(!sub.is_empty());
        assert!(!sub.is_all());
        assert!(sub
            .event_types()
            .contains(&EslEventType::ChannelCreate));
        assert!(sub
            .event_types()
            .contains(&EslEventType::Heartbeat));
        assert_eq!(sub.custom_subclass_list(), &["sofia::register"]);
        assert_eq!(
            sub.filters()
                .len(),
            1
        );
    }

    #[test]
    fn serde_round_trip_subscription() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .event(EslEventType::ChannelCreate)
            .event(EslEventType::Heartbeat)
            .custom_subclass("sofia::register")
            .unwrap()
            .filter(EventHeader::CallDirection, "inbound")
            .unwrap();

        let json = serde_json::to_string(&sub).unwrap();
        let deserialized: EventSubscription = serde_json::from_str(&json).unwrap();
        assert_eq!(sub, deserialized);
    }

    #[test]
    fn serde_rejects_invalid_subclass() {
        let json =
            r#"{"format":"Plain","events":[],"custom_subclasses":["bad subclass"],"filters":[]}"#;
        let result: Result<EventSubscription, _> = serde_json::from_str(json);
        assert!(result.is_err());
        let err = result
            .unwrap_err()
            .to_string();
        assert!(err.contains("space"), "error should mention space: {err}");
    }

    #[test]
    fn serde_rejects_newline_in_filter() {
        let json = r#"{"format":"Plain","events":[],"custom_subclasses":[],"filters":[["Header","val\n"]]}"#;
        let result: Result<EventSubscription, _> = serde_json::from_str(json);
        assert!(result.is_err());
        let err = result
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("newline"),
            "error should mention newline: {err}"
        );
    }

    #[test]
    fn custom_subclass_rejects_space() {
        let result = EventSubscription::new(EventFormat::Plain).custom_subclass("bad subclass");
        assert!(result.is_err());
    }

    #[test]
    fn custom_subclass_rejects_newline() {
        let result = EventSubscription::new(EventFormat::Plain).custom_subclass("bad\nsubclass");
        assert!(result.is_err());
    }

    #[test]
    fn custom_subclass_rejects_empty() {
        let result = EventSubscription::new(EventFormat::Plain).custom_subclass("");
        assert!(result.is_err());
    }

    #[test]
    fn filter_raw_rejects_newline_in_header() {
        let result = EventSubscription::new(EventFormat::Plain).filter_raw("Bad\nHeader", "value");
        assert!(result.is_err());
    }

    #[test]
    fn filter_raw_rejects_newline_in_value() {
        let result = EventSubscription::new(EventFormat::Plain).filter_raw("Header", "bad\nvalue");
        assert!(result.is_err());
    }

    #[test]
    fn filter_typed_rejects_newline_in_value() {
        let result = EventSubscription::new(EventFormat::Plain)
            .filter(EventHeader::CallDirection, "bad\nvalue");
        assert!(result.is_err());
    }

    #[test]
    fn sofia_event_single() {
        let sub =
            EventSubscription::new(EventFormat::Plain).sofia_event(SofiaEventSubclass::Register);
        assert_eq!(
            sub.to_event_string(),
            Some("CUSTOM sofia::register".to_string())
        );
    }

    #[test]
    fn sofia_events_group() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .sofia_events(SofiaEventSubclass::GATEWAY_EVENTS);
        let event_str = sub
            .to_event_string()
            .unwrap();
        assert!(event_str.starts_with("CUSTOM"));
        assert!(event_str.contains("sofia::gateway_state"));
        assert!(event_str.contains("sofia::gateway_add"));
        assert!(event_str.contains("sofia::gateway_delete"));
        assert!(event_str.contains("sofia::gateway_invalid_digest_req"));
    }

    #[test]
    fn event_raw_wire_string() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .event(EslEventType::Heartbeat)
            .event_raw("NEW_EVENT_NOT_IN_ENUM")
            .unwrap();
        assert_eq!(
            sub.to_event_string(),
            Some("HEARTBEAT NEW_EVENT_NOT_IN_ENUM".to_string())
        );
    }

    #[test]
    fn events_raw_wire_string() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .events_raw(["FUTURE_A", "FUTURE_B"])
            .unwrap();
        assert_eq!(sub.to_event_string(), Some("FUTURE_A FUTURE_B".to_string()));
    }

    #[test]
    fn event_raw_with_custom_subclass() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .event_raw("NEW_EVENT")
            .unwrap()
            .custom_subclass("sofia::register")
            .unwrap();
        assert_eq!(
            sub.to_event_string(),
            Some("NEW_EVENT CUSTOM sofia::register".to_string())
        );
    }

    #[test]
    fn event_raw_rejects_newline() {
        assert!(EventSubscription::new(EventFormat::Plain)
            .event_raw("bad\nevent")
            .is_err());
    }

    #[test]
    fn event_raw_rejects_space() {
        assert!(EventSubscription::new(EventFormat::Plain)
            .event_raw("bad event")
            .is_err());
    }

    #[test]
    fn event_raw_rejects_empty() {
        assert!(EventSubscription::new(EventFormat::Plain)
            .event_raw("")
            .is_err());
    }

    #[test]
    fn events_raw_errors_on_first_invalid() {
        let result =
            EventSubscription::new(EventFormat::Plain).events_raw(["GOOD", "bad event", "OTHER"]);
        assert!(result.is_err());
    }

    #[test]
    fn event_types_raw_mut_mutable() {
        let mut sub = EventSubscription::new(EventFormat::Plain);
        sub.event_types_raw_mut()
            .push("DIRECT_PUSH".to_string());
        assert_eq!(sub.event_types_raw(), &["DIRECT_PUSH".to_string()]);
    }

    #[test]
    fn is_empty_sees_raw_events() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .event_raw("ONLY_RAW")
            .unwrap();
        assert!(!sub.is_empty());
    }

    #[test]
    fn serde_round_trip_with_raw_events() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .event(EslEventType::ChannelCreate)
            .event_raw("FUTURE_EVENT")
            .unwrap()
            .custom_subclass("sofia::register")
            .unwrap();

        let json = serde_json::to_string(&sub).unwrap();
        let deserialized: EventSubscription = serde_json::from_str(&json).unwrap();
        assert_eq!(sub, deserialized);
    }

    #[test]
    fn serde_rejects_invalid_raw_event() {
        let json = r#"{"format":"Plain","events":[],"raw_events":["bad event"],"custom_subclasses":[],"filters":[]}"#;
        let result: Result<EventSubscription, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn serde_missing_raw_events_field_defaults_to_empty() {
        // Back-compat: configs written before raw_events was added must still
        // deserialize.
        let json =
            r#"{"format":"Plain","events":["Heartbeat"],"custom_subclasses":[],"filters":[]}"#;
        let sub: EventSubscription = serde_json::from_str(json).unwrap();
        assert!(sub
            .event_types_raw()
            .is_empty());
    }

    #[test]
    fn sofia_event_mixed_with_typed_events() {
        let sub = EventSubscription::new(EventFormat::Plain)
            .event(EslEventType::Heartbeat)
            .sofia_event(SofiaEventSubclass::GatewayState);
        assert_eq!(
            sub.to_event_string(),
            Some("HEARTBEAT CUSTOM sofia::gateway_state".to_string())
        );
    }
}
