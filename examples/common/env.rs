//! Where an example connects. Kept apart from the connect helper beside it so
//! the two examples that build their own connection can include this alone.

use freeswitch_esl_tokio::{DEFAULT_ESL_PASSWORD, DEFAULT_ESL_PORT};

/// `ESL_HOST` / `ESL_PORT` / `ESL_PASSWORD`, with the crate's defaults.
pub struct EslEnv {
    /// A bare IPv6 literal needs no brackets here: `EslClient::connect` takes
    /// the host and the port separately.
    pub host: String,
    /// ESL listener port.
    pub port: u16,
    /// ESL password, cleartext on the wire.
    pub password: String,
}

impl EslEnv {
    /// A malformed `ESL_PORT` is an error rather than a fall back to the
    /// default: a typo that quietly connected somewhere else would read as
    /// FreeSWITCH refusing the connection.
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            host: std::env::var("ESL_HOST").unwrap_or_else(|_| "localhost".to_string()),
            port: match std::env::var("ESL_PORT") {
                Ok(value) => value
                    .parse()
                    .map_err(|_| format!("ESL_PORT is not a port number: {value}"))?,
                Err(_) => DEFAULT_ESL_PORT,
            },
            password: std::env::var("ESL_PASSWORD")
                .unwrap_or_else(|_| DEFAULT_ESL_PASSWORD.to_string()),
        })
    }
}
