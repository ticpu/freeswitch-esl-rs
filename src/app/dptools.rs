use crate::command::EslCommand;

/// Execute application commands
pub struct AppCommand;

impl AppCommand {
    /// Answer call
    pub fn answer() -> EslCommand {
        EslCommand::Execute {
            app: "answer".to_string(),
            args: None,
            uuid: None,
        }
    }

    /// Hangup call
    pub fn hangup(cause: Option<&str>) -> EslCommand {
        EslCommand::Execute {
            app: "hangup".to_string(),
            args: cause.map(|c| c.to_string()),
            uuid: None,
        }
    }

    /// Play audio file
    pub fn playback(file: &str) -> EslCommand {
        EslCommand::Execute {
            app: "playback".to_string(),
            args: Some(file.to_string()),
            uuid: None,
        }
    }

    /// Bridge two channels
    pub fn bridge(destination: &str) -> EslCommand {
        EslCommand::Execute {
            app: "bridge".to_string(),
            args: Some(destination.to_string()),
            uuid: None,
        }
    }

    /// Set channel variable
    pub fn set_var(name: &str, value: &str) -> EslCommand {
        EslCommand::Execute {
            app: "set".to_string(),
            args: Some(format!("{}={}", name, value)),
            uuid: None,
        }
    }

    /// Park call
    pub fn park() -> EslCommand {
        EslCommand::Execute {
            app: "park".to_string(),
            args: None,
            uuid: None,
        }
    }

    /// Transfer call
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
