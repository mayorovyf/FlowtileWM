#![forbid(unsafe_code)]

use core::fmt;

pub const VERSION_LINE: &str = "v.0.0.1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeMode {
    WmOnly,
    ExtendedShell,
    SafeMode,
}

impl RuntimeMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WmOnly => "wm-only",
            Self::ExtendedShell => "extended-shell",
            Self::SafeMode => "safe-mode",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "wm-only" => Some(Self::WmOnly),
            "extended-shell" => Some(Self::ExtendedShell),
            "safe-mode" => Some(Self::SafeMode),
            _ => None,
        }
    }
}

impl fmt::Display for RuntimeMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct StateVersion(u64);

impl StateVersion {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapProfile {
    pub runtime_mode: RuntimeMode,
    pub state_version: StateVersion,
    pub version_line: &'static str,
}

impl BootstrapProfile {
    pub const fn new(runtime_mode: RuntimeMode) -> Self {
        Self {
            runtime_mode,
            state_version: StateVersion::new(0),
            version_line: VERSION_LINE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BootstrapProfile, RuntimeMode, StateVersion, VERSION_LINE};

    #[test]
    fn parses_known_runtime_mode() {
        assert_eq!(
            RuntimeMode::parse("extended-shell"),
            Some(RuntimeMode::ExtendedShell)
        );
    }

    #[test]
    fn builds_bootstrap_profile() {
        let profile = BootstrapProfile::new(RuntimeMode::WmOnly);
        assert_eq!(profile.runtime_mode, RuntimeMode::WmOnly);
        assert_eq!(profile.state_version, StateVersion::new(0));
        assert_eq!(profile.version_line, VERSION_LINE);
    }
}
