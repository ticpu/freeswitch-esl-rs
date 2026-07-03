//! Typed Sofia event subclasses and state enums.
//!
//! mod_sofia fires `CUSTOM` events with `Event-Subclass` headers like
//! `sofia::register` and `sofia::gateway_state`. This module provides typed
//! enums for those subclass values and for the structured state headers
//! they carry.

mod channel_name;
mod event_subclass;
mod gateway;
mod sip_user;

pub use channel_name::SofiaChannelName;
pub use event_subclass::{ParseSofiaEventSubclassError, SofiaEventSubclass};
pub use gateway::{
    GatewayPingStatus, GatewayRegState, ParseGatewayPingStatusError, ParseGatewayRegStateError,
};
pub use sip_user::{ParseSipUserPingStatusError, SipUserPingStatus};
