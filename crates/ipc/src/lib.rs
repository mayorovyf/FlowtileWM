#![forbid(unsafe_code)]

pub const PROTOCOL_VERSION: u32 = 1;
pub const TRANSPORT: &str = "local-named-pipe";
pub const COMMANDS: &[&str] = &[
    "get_outputs",
    "get_workspaces",
    "get_windows",
    "get_focus",
    "move_window",
    "consume_window",
    "expel_window",
    "toggle_floating",
    "toggle_tabbed",
    "toggle_overview",
    "capture_action",
    "reload_config",
    "dump_diagnostics",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IpcBootstrap {
    pub transport: &'static str,
    pub protocol_version: u32,
    pub commands: Vec<&'static str>,
}

pub fn bootstrap() -> IpcBootstrap {
    IpcBootstrap {
        transport: TRANSPORT,
        protocol_version: PROTOCOL_VERSION,
        commands: COMMANDS.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::{PROTOCOL_VERSION, TRANSPORT, bootstrap};

    #[test]
    fn exposes_bootstrap_transport_and_version() {
        let bootstrap = bootstrap();
        assert_eq!(bootstrap.transport, TRANSPORT);
        assert_eq!(bootstrap.protocol_version, PROTOCOL_VERSION);
        assert!(bootstrap.commands.contains(&"toggle_overview"));
    }
}
