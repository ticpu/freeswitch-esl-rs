//! The connection preamble every example that talks to a switch begins with.
//!
//! Included as `mod common;`. It lives in a directory rather than beside the
//! examples because cargo builds every `examples/*.rs` as its own binary.

mod env;

use freeswitch_esl_tokio::{EslClient, EslError, EslEventStream};
use tracing::{error, info};

/// Connect using the environment, naming the likely cause when nothing
/// answers -- the first failure a reader who has not started FreeSWITCH hits.
pub async fn connect_from_env() -> Result<(EslClient, EslEventStream), Box<dyn std::error::Error>> {
    let env = env::EslEnv::from_env()?;

    match EslClient::connect(&env.host, env.port, &env.password).await {
        Ok(pair) => {
            info!("connected to {}:{}", env.host, env.port);
            Ok(pair)
        }
        Err(EslError::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            error!(
                "nothing listening on {}:{} (set ESL_HOST / ESL_PORT)",
                env.host, env.port
            );
            Err(e.into())
        }
        Err(e) => Err(e.into()),
    }
}
