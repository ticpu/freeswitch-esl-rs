//! Application execution via `sendmsg` — the dptools family of commands.
//!
//! These produce `sendmsg` commands for use with
//! [`EslClient::send_command()`](crate::EslClient::send_command).

pub mod dptools;
