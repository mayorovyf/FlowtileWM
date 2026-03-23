#![forbid(unsafe_code)]

pub const CHANNELS: &[&str] = &[
    "structured-logs",
    "state-dumps",
    "health-reporting",
    "watchdog-signals",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticsBootstrap {
    pub channels: Vec<&'static str>,
    pub supports_state_dump: bool,
}

pub fn bootstrap() -> DiagnosticsBootstrap {
    DiagnosticsBootstrap {
        channels: CHANNELS.to_vec(),
        supports_state_dump: true,
    }
}

#[cfg(test)]
mod tests {
    use super::bootstrap;

    #[test]
    fn keeps_state_dump_in_bootstrap_contract() {
        let bootstrap = bootstrap();
        assert!(bootstrap.supports_state_dump);
        assert!(bootstrap.channels.contains(&"structured-logs"));
    }
}
