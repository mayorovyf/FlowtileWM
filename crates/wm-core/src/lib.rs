#![forbid(unsafe_code)]

use flowtile_config_rules::bootstrap as config_bootstrap;
use flowtile_diagnostics::bootstrap as diagnostics_bootstrap;
use flowtile_domain::{BootstrapProfile, RuntimeMode};
use flowtile_ipc::bootstrap as ipc_bootstrap;
use flowtile_layout_engine::{ColumnMode, bootstrap_modes, preserves_insert_invariant};
use flowtile_windows_adapter::bootstrap as windows_bootstrap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreDaemonBootstrap {
    pub profile: BootstrapProfile,
    pub config_path: &'static str,
    pub ipc_command_count: usize,
    pub adapter_discovery_api: &'static str,
    pub diagnostics_channel_count: usize,
    pub layout_modes: [ColumnMode; 4],
}

impl CoreDaemonBootstrap {
    pub fn new(runtime_mode: RuntimeMode) -> Self {
        let config = config_bootstrap();
        let diagnostics = diagnostics_bootstrap();
        let ipc = ipc_bootstrap();
        let adapter = windows_bootstrap();

        Self {
            profile: BootstrapProfile::new(runtime_mode),
            config_path: config.default_path,
            ipc_command_count: ipc.commands.len(),
            adapter_discovery_api: adapter.discovery_api,
            diagnostics_channel_count: diagnostics.channels.len(),
            layout_modes: bootstrap_modes(),
        }
    }

    pub fn summary_lines(&self) -> Vec<String> {
        let modes = self
            .layout_modes
            .iter()
            .map(|mode| mode.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        vec![
            format!("version line: {}", self.profile.version_line),
            format!("runtime mode: {}", self.profile.runtime_mode),
            format!("state version: {}", self.profile.state_version.get()),
            format!("config path: {}", self.config_path),
            format!("layout modes prepared: {modes}"),
            format!(
                "insert invariant visible in bootstrap: {}",
                preserves_insert_invariant()
            ),
            format!(
                "windows adapter discovery API: {}",
                self.adapter_discovery_api
            ),
            format!("ipc commands prepared: {}", self.ipc_command_count),
            format!(
                "diagnostics channels prepared: {}",
                self.diagnostics_channel_count
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use flowtile_domain::RuntimeMode;

    use super::CoreDaemonBootstrap;

    #[test]
    fn builds_summary_without_product_logic() {
        let bootstrap = CoreDaemonBootstrap::new(RuntimeMode::ExtendedShell);
        let summary = bootstrap.summary_lines();
        assert!(summary.iter().any(|line| line.contains("extended-shell")));
        assert!(
            summary
                .iter()
                .any(|line| line.contains("ipc commands prepared"))
        );
    }
}
