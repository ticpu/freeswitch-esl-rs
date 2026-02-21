//! FreeSWITCH dptools application commands (`answer`, `hangup`, `playback`, etc.).

use crate::command::EslCommand;

/// Constructors for common dptools application commands.
///
/// Each method returns an [`EslCommand::Execute`] ready for
/// [`EslClient::sendmsg()`](crate::EslClient::sendmsg).
/// The `uuid` field is `None` — set it on the command or pass it to `sendmsg()`.
pub struct AppCommand;

impl AppCommand {
    pub fn answer() -> EslCommand {
        EslCommand::Execute {
            app: "answer".to_string(),
            args: None,
            uuid: None,
        }
    }

    /// `cause`: hangup cause string (e.g. `NORMAL_CLEARING`). `None` uses default.
    pub fn hangup(cause: Option<&str>) -> EslCommand {
        EslCommand::Execute {
            app: "hangup".to_string(),
            args: cause.map(|c| c.to_string()),
            uuid: None,
        }
    }

    /// `file`: path, `tone_stream://`, or any FreeSWITCH file-like URI.
    pub fn playback(file: &str) -> EslCommand {
        EslCommand::Execute {
            app: "playback".to_string(),
            args: Some(file.to_string()),
            uuid: None,
        }
    }

    /// `destination`: dial string for the B-leg (e.g. `sofia/gateway/gw/number`).
    pub fn bridge(destination: &str) -> EslCommand {
        EslCommand::Execute {
            app: "bridge".to_string(),
            args: Some(destination.to_string()),
            uuid: None,
        }
    }

    pub fn set_var(name: &str, value: &str) -> EslCommand {
        EslCommand::Execute {
            app: "set".to_string(),
            args: Some(format!("{}={}", name, value)),
            uuid: None,
        }
    }

    pub fn park() -> EslCommand {
        EslCommand::Execute {
            app: "park".to_string(),
            args: None,
            uuid: None,
        }
    }

    pub fn transfer(extension: &str, dialplan: Option<&str>, context: Option<&str>) -> EslCommand {
        let mut args = extension.to_string();
        if let Some(dp) = dialplan {
            args.push(' ');
            args.push_str(dp);
        }
        if let Some(ctx) = context {
            args.push(' ');
            args.push_str(ctx);
        }

        EslCommand::Execute {
            app: "transfer".to_string(),
            args: Some(args),
            uuid: None,
        }
    }
}
