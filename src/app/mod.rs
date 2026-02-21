//! Application execution via `sendmsg` — the dptools family of commands.
//!
//! These produce [`EslCommand::Execute`](crate::command::EslCommand::Execute)
//! values for use with [`EslClient::sendmsg()`](crate::EslClient::sendmsg).

pub mod dptools;
