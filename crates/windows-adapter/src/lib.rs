#![forbid(unsafe_code)]

pub const PRIMARY_DISCOVERY_API: &str = "SetWinEventHook";
pub const FALLBACK_DISCOVERY_PATH: &str = "full-window-scan";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowsAdapterBootstrap {
    pub discovery_api: &'static str,
    pub fallback_path: &'static str,
    pub batches_geometry_operations: bool,
    pub owns_product_policy: bool,
}

pub const fn bootstrap() -> WindowsAdapterBootstrap {
    WindowsAdapterBootstrap {
        discovery_api: PRIMARY_DISCOVERY_API,
        fallback_path: FALLBACK_DISCOVERY_PATH,
        batches_geometry_operations: true,
        owns_product_policy: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{PRIMARY_DISCOVERY_API, bootstrap};

    #[test]
    fn keeps_adapter_non_authoritative() {
        let bootstrap = bootstrap();
        assert_eq!(bootstrap.discovery_api, PRIMARY_DISCOVERY_API);
        assert!(bootstrap.batches_geometry_operations);
        assert!(!bootstrap.owns_product_policy);
    }
}
