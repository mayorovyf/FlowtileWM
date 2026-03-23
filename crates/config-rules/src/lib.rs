#![forbid(unsafe_code)]

pub const PREFERRED_CONFIG_FORMAT: &str = "KDL";
pub const FALLBACK_CONFIG_FORMAT: &str = "TOML";
pub const DEFAULT_CONFIG_PATH: &str = "config/flowtile.kdl";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigBootstrap {
    pub preferred_format: &'static str,
    pub fallback_format: &'static str,
    pub default_path: &'static str,
    pub supports_live_reload: bool,
    pub supports_rollback: bool,
}

pub const fn bootstrap() -> ConfigBootstrap {
    ConfigBootstrap {
        preferred_format: PREFERRED_CONFIG_FORMAT,
        fallback_format: FALLBACK_CONFIG_FORMAT,
        default_path: DEFAULT_CONFIG_PATH,
        supports_live_reload: true,
        supports_rollback: true,
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_CONFIG_PATH, PREFERRED_CONFIG_FORMAT, bootstrap};

    #[test]
    fn exposes_expected_bootstrap_contract() {
        let bootstrap = bootstrap();
        assert_eq!(bootstrap.preferred_format, PREFERRED_CONFIG_FORMAT);
        assert_eq!(bootstrap.default_path, DEFAULT_CONFIG_PATH);
        assert!(bootstrap.supports_live_reload);
        assert!(bootstrap.supports_rollback);
    }
}
