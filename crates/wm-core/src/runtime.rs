use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use flowtile_config_rules::{
    HotkeyBinding, LoadedConfig, WindowRuleInput, bootstrap as config_bootstrap, classify_window,
    default_loaded_config, ensure_default_config, load_from_path, load_or_default,
};
use flowtile_domain::{
    BindControlMode, CorrelationId, DomainEvent, DomainEventPayload, FocusBehavior, MonitorId,
    RuntimeMode, TopologyRole, WindowId, WindowLayer, WindowPlacement, WmState,
};
use flowtile_layout_engine::recompute_workspace;
use flowtile_windows_adapter::{
    ApplyBatchResult, ApplyOperation, ObservationEnvelope, ObservationKind,
    PlatformMonitorSnapshot, PlatformSnapshot, PlatformWindowSnapshot, SnapshotDiff,
    WindowsAdapter, diff_snapshots, needs_activation_apply, needs_geometry_apply,
};

use crate::{CoreDaemonRuntime, RuntimeCycleReport, RuntimeError, StateStore};

impl CoreDaemonRuntime {
    pub fn new(runtime_mode: RuntimeMode) -> Self {
        Self::with_adapter(runtime_mode, WindowsAdapter::new())
    }

    pub fn with_adapter(runtime_mode: RuntimeMode, adapter: WindowsAdapter) -> Self {
        let config_path = workspace_path(config_bootstrap().default_path);
        let had_startup_config = config_path.exists();
        let startup_config = ensure_default_config(&config_path)
            .ok()
            .and_then(|path| load_or_default(&path, 1).ok())
            .unwrap_or_else(|| default_loaded_config(1, config_path.display().to_string()));
        let mut store = StateStore::new(runtime_mode);
        store.state_mut().config_projection = startup_config.projection.clone();

        let mut runtime = Self {
            store,
            adapter,
            active_config: startup_config.clone(),
            last_valid_config: startup_config,
            last_snapshot: None,
            management_enabled: runtime_mode != RuntimeMode::SafeMode,
            consecutive_desync_cycles: 0,
            next_correlation_id: 1,
            next_config_generation: 2,
        };

        if !had_startup_config {
            runtime.push_degraded_reason("config-bootstrap-fallback".to_string());
        }

        runtime
    }

    pub const fn state(&self) -> &WmState {
        self.store.state()
    }

    pub fn hotkeys(&self) -> &[HotkeyBinding] {
        &self.active_config.hotkeys
    }

    pub const fn bind_control_mode(&self) -> BindControlMode {
        self.active_config.projection.bind_control_mode
    }

    pub fn last_snapshot(&self) -> Option<&PlatformSnapshot> {
        self.last_snapshot.as_ref()
    }

    pub const fn management_enabled(&self) -> bool {
        self.management_enabled
    }

    pub fn request_emergency_unwind(&mut self, reason: &str) {
        self.management_enabled = false;
        self.push_degraded_reason(format!("emergency-unwind:{reason}"));
    }

    pub fn dispatch_command(
        &mut self,
        event: DomainEvent,
        dry_run: bool,
        reason: &str,
    ) -> Result<RuntimeCycleReport, RuntimeError> {
        let snapshot = self.adapter.scan_snapshot()?;
        let _ = self.sync_snapshot_with_reason(snapshot.clone(), true, "command-pre-sync")?;
        let transition = self.store.dispatch(event)?;
        let planned_operations = if self.management_enabled {
            self.plan_apply_operations(&snapshot)?
        } else {
            Vec::new()
        };
        let apply_result = if dry_run || !self.management_enabled {
            ApplyBatchResult::default()
        } else {
            self.adapter.apply_operations(&planned_operations)?
        };

        let now = unix_timestamp();
        self.store.state_mut().runtime.last_full_scan_at = Some(now);
        if transition.affected_workspace_id.is_some() || !planned_operations.is_empty() {
            self.store.state_mut().runtime.last_reconcile_at = Some(now);
        }
        self.last_snapshot = Some(snapshot.clone());

        let mut report = RuntimeCycleReport {
            monitor_count: snapshot.monitors.len(),
            observed_window_count: snapshot.windows.len(),
            discovered_windows: 0,
            destroyed_windows: 0,
            focused_hwnd: snapshot.focused_window().map(|window| window.hwnd),
            observation_reason: Some(reason.to_string()),
            planned_operations: planned_operations.len(),
            applied_operations: apply_result.applied,
            apply_failures: apply_result.failures.len(),
            recovery_rescans: 0,
            validation_remaining_operations: 0,
            recovery_actions: Vec::new(),
            management_enabled: self.management_enabled,
            dry_run,
            degraded_reasons: self.store.state().runtime.degraded_flags.clone(),
        };
        self.validate_after_apply(&mut report, dry_run)?;
        Ok(report)
    }

    pub fn reload_config(&mut self, dry_run: bool) -> Result<RuntimeCycleReport, RuntimeError> {
        let config_path = self.store.state().config_projection.source_path.clone();
        let correlation_id = self.next_correlation_id();
        let _ = self.store.dispatch(DomainEvent::config_reload_requested(
            correlation_id,
            flowtile_domain::EventSource::InputCommand,
            Some(config_path.clone()),
        ))?;

        match load_from_path(&config_path, self.next_config_generation) {
            Ok(loaded_config) => {
                ensure_supported_bind_control_mode(loaded_config.projection.bind_control_mode)?;
                let changed_sections = diff_config_sections(&self.active_config, &loaded_config);
                let rule_ids = loaded_config
                    .rules
                    .iter()
                    .map(|rule| rule.id.clone())
                    .collect::<Vec<_>>();
                self.active_config = loaded_config.clone();
                self.last_valid_config = loaded_config.clone();
                self.next_config_generation += 1;

                let reload_succeeded_correlation = self.next_correlation_id();
                self.store.dispatch(DomainEvent::config_reload_succeeded(
                    reload_succeeded_correlation,
                    loaded_config.projection.config_version,
                    changed_sections,
                    loaded_config.projection.clone(),
                ))?;
                let rules_updated_correlation = self.next_correlation_id();
                self.store.dispatch(DomainEvent::rules_updated(
                    rules_updated_correlation,
                    loaded_config.projection.config_version,
                    rule_ids,
                    loaded_config.projection.active_rule_count,
                ))?;

                let report_correlation = self.next_correlation_id();
                self.dispatch_command(
                    DomainEvent::config_reload_requested(
                        report_correlation,
                        flowtile_domain::EventSource::ConfigRules,
                        Some(config_path),
                    ),
                    dry_run,
                    "config-reload",
                )
            }
            Err(error) => {
                let failure_correlation = self.next_correlation_id();
                let _ = self.store.dispatch(DomainEvent::config_reload_failed(
                    failure_correlation,
                    "config-reload-failed",
                    error.to_string(),
                ));
                self.active_config = self.last_valid_config.clone();
                self.push_degraded_reason("config-reload-failed".to_string());
                Err(RuntimeError::Config(error.to_string()))
            }
        }
    }

    pub fn scan_and_sync(&mut self, dry_run: bool) -> Result<RuntimeCycleReport, RuntimeError> {
        let snapshot = self.adapter.scan_snapshot()?;
        let mut report = self.sync_snapshot_with_reason(snapshot, dry_run, "full-scan")?;
        self.validate_after_apply(&mut report, dry_run)?;
        Ok(report)
    }

    pub fn apply_observation(
        &mut self,
        observation: ObservationEnvelope,
        dry_run: bool,
    ) -> Result<Option<RuntimeCycleReport>, RuntimeError> {
        match observation.kind {
            ObservationKind::Snapshot => {
                let Some(snapshot) = observation.snapshot else {
                    self.push_degraded_reason(format!(
                        "observer-missing-snapshot:{}",
                        normalize_reason_token(&observation.reason)
                    ));
                    return Ok(None);
                };

                let mut report =
                    self.sync_snapshot_with_reason(snapshot, dry_run, &observation.reason)?;
                self.validate_after_apply(&mut report, dry_run)?;
                Ok(Some(report))
            }
            ObservationKind::Suspend => {
                self.push_degraded_reason("system-suspend".to_string());
                Ok(None)
            }
            ObservationKind::Resume => {
                self.push_degraded_reason("system-resume".to_string());
                let mut report = self.scan_and_sync(dry_run)?;
                report.observation_reason = Some(observation.reason);
                Ok(Some(report))
            }
            ObservationKind::Warning => {
                self.push_degraded_reason(format!(
                    "observer-warning:{}",
                    normalize_reason_token(&observation.reason)
                ));
                if let Some(message) = observation.message {
                    self.push_degraded_reason(format!(
                        "observer-detail:{}",
                        normalize_reason_token(&message)
                    ));
                }
                Ok(None)
            }
        }
    }

    pub fn sync_snapshot(
        &mut self,
        snapshot: PlatformSnapshot,
        dry_run: bool,
    ) -> Result<RuntimeCycleReport, RuntimeError> {
        self.sync_snapshot_with_reason(snapshot, dry_run, "external-snapshot")
    }

    fn sync_snapshot_with_reason(
        &mut self,
        mut snapshot: PlatformSnapshot,
        dry_run: bool,
        observation_reason: &str,
    ) -> Result<RuntimeCycleReport, RuntimeError> {
        snapshot.sort_for_stability();
        self.note_observation_reason(observation_reason);
        self.sync_monitors_from_snapshot(&snapshot.monitors)?;

        let had_previous_snapshot = self.last_snapshot.is_some();
        let diff = self
            .last_snapshot
            .as_ref()
            .map(|previous| diff_snapshots(previous, &snapshot))
            .unwrap_or_else(|| SnapshotDiff::initial(&snapshot));

        let discovered_windows = self.ingest_created_windows(&diff.created_windows)?;
        let destroyed_windows = self.ingest_destroyed_windows(&diff.destroyed_hwnds)?;

        if had_previous_snapshot && diff.monitor_topology_changed {
            self.push_degraded_reason("monitor-topology-changed".to_string());
        }

        if let Some(focused_hwnd) = diff.focused_hwnd {
            self.observe_focus(focused_hwnd)?;
        }

        let planned_operations = if self.management_enabled {
            self.plan_apply_operations(&snapshot)?
        } else {
            Vec::new()
        };
        let apply_result = if dry_run || !self.management_enabled {
            ApplyBatchResult::default()
        } else {
            self.adapter.apply_operations(&planned_operations)?
        };

        let now = unix_timestamp();
        self.store.state_mut().runtime.last_full_scan_at = Some(now);
        if !planned_operations.is_empty() {
            self.store.state_mut().runtime.last_reconcile_at = Some(now);
        }
        self.last_snapshot = Some(snapshot.clone());

        Ok(RuntimeCycleReport {
            monitor_count: snapshot.monitors.len(),
            observed_window_count: snapshot.windows.len(),
            discovered_windows,
            destroyed_windows,
            focused_hwnd: snapshot.focused_window().map(|window| window.hwnd),
            observation_reason: Some(observation_reason.to_string()),
            planned_operations: planned_operations.len(),
            applied_operations: apply_result.applied,
            apply_failures: apply_result.failures.len(),
            recovery_rescans: 0,
            validation_remaining_operations: 0,
            recovery_actions: Vec::new(),
            management_enabled: self.management_enabled,
            dry_run,
            degraded_reasons: self.store.state().runtime.degraded_flags.clone(),
        })
    }

    fn validate_after_apply(
        &mut self,
        report: &mut RuntimeCycleReport,
        dry_run: bool,
    ) -> Result<(), RuntimeError> {
        if dry_run || !self.management_enabled || report.planned_operations == 0 {
            return Ok(());
        }

        let validation_snapshot = self.adapter.scan_snapshot()?;
        report.recovery_rescans += 1;
        let remaining_operations = self.plan_apply_operations(&validation_snapshot)?;
        report.validation_remaining_operations = remaining_operations.len();

        if remaining_operations.is_empty() {
            self.consecutive_desync_cycles = 0;
            self.last_snapshot = Some(validation_snapshot);
            report
                .recovery_actions
                .push("post-apply-validation-clean".to_string());
            report.degraded_reasons = self.store.state().runtime.degraded_flags.clone();
            return Ok(());
        }

        self.push_degraded_reason("desync:post-apply-diverged".to_string());
        report.recovery_actions.push(format!(
            "targeted-retry:{}-ops-remain",
            remaining_operations.len()
        ));

        let retry_result = self.adapter.apply_operations(&remaining_operations)?;
        report.applied_operations += retry_result.applied;
        report.apply_failures += retry_result.failures.len();

        let post_retry_snapshot = self.adapter.scan_snapshot()?;
        report.recovery_rescans += 1;
        let post_retry_remaining = self.plan_apply_operations(&post_retry_snapshot)?;
        report.validation_remaining_operations = post_retry_remaining.len();
        self.last_snapshot = Some(post_retry_snapshot);

        if post_retry_remaining.is_empty() {
            self.consecutive_desync_cycles = 0;
            report.recovery_actions.push("retry-recovered".to_string());
        } else {
            self.consecutive_desync_cycles += 1;
            self.push_degraded_reason(format!(
                "desync:remaining-operations:{}",
                post_retry_remaining.len()
            ));
            report.recovery_actions.push(format!(
                "full-scan-escalation:{}-ops-still-diverged",
                post_retry_remaining.len()
            ));

            if report.apply_failures > 0 || self.consecutive_desync_cycles >= 2 {
                self.request_emergency_unwind("desync-recovery-escalated");
                report.recovery_actions.push("safe-mode-unwind".to_string());
            }
        }

        report.management_enabled = self.management_enabled;
        report.degraded_reasons = self.store.state().runtime.degraded_flags.clone();
        Ok(())
    }

    fn ingest_created_windows(
        &mut self,
        windows: &[PlatformWindowSnapshot],
    ) -> Result<usize, RuntimeError> {
        let mut discovered_windows = 0;

        for window in windows {
            if self.find_window_id_by_hwnd(window.hwnd).is_some() {
                continue;
            }

            let Some(monitor_id) = self.monitor_id_by_binding(&window.monitor_binding) else {
                self.push_degraded_reason(format!(
                    "missing-monitor-binding:{}",
                    window.monitor_binding
                ));
                continue;
            };

            let decision = classify_window(
                &self.active_config.rules,
                &WindowRuleInput {
                    process_name: window.process_name.clone(),
                    class_name: window.class_name.clone(),
                    title: window.title.clone(),
                },
                &self.active_config.projection,
            );
            let correlation_id = self.next_correlation_id();
            self.store.dispatch(DomainEvent::new(
                flowtile_domain::DomainEventName::WindowDiscovered,
                flowtile_domain::EventCategory::PlatformDerived,
                flowtile_domain::EventSource::WindowsAdapter,
                correlation_id,
                DomainEventPayload::WindowDiscovered(flowtile_domain::WindowDiscoveredPayload {
                    monitor_id,
                    hwnd: window.hwnd,
                    classification: if decision.layer == WindowLayer::Tiled {
                        flowtile_domain::WindowClassification::Application
                    } else {
                        flowtile_domain::WindowClassification::Utility
                    },
                    desired_size: flowtile_domain::Size::new(window.rect.width, window.rect.height),
                    last_known_rect: window.rect,
                    placement: WindowPlacement::AppendToWorkspaceEnd {
                        mode: decision.column_mode,
                        width: decision.width_semantics,
                    },
                    focus_behavior: FocusBehavior::PreserveCurrentFocus,
                    layer: decision.layer,
                    managed: decision.managed,
                }),
            ))?;
            discovered_windows += 1;
        }

        Ok(discovered_windows)
    }

    fn ingest_destroyed_windows(&mut self, hwnds: &[u64]) -> Result<usize, RuntimeError> {
        let mut destroyed_windows = 0;

        for hwnd in hwnds {
            let Some(window_id) = self.find_window_id_by_hwnd(*hwnd) else {
                continue;
            };

            let correlation_id = self.next_correlation_id();
            self.store
                .dispatch(DomainEvent::window_destroyed(correlation_id, window_id))?;
            destroyed_windows += 1;
        }

        Ok(destroyed_windows)
    }

    fn observe_focus(&mut self, hwnd: u64) -> Result<(), RuntimeError> {
        let Some(window_id) = self.find_window_id_by_hwnd(hwnd) else {
            return Ok(());
        };
        let Some(window) = self.store.state().windows.get(&window_id) else {
            return Ok(());
        };
        let workspace_id = window.workspace_id;
        let Some(workspace) = self.store.state().workspaces.get(&workspace_id) else {
            return Ok(());
        };
        let monitor_id = workspace.monitor_id;

        let correlation_id = self.next_correlation_id();
        self.store.dispatch(DomainEvent::window_focus_observed(
            correlation_id,
            monitor_id,
            window_id,
        ))?;
        Ok(())
    }

    pub(crate) fn plan_apply_operations(
        &self,
        snapshot: &PlatformSnapshot,
    ) -> Result<Vec<ApplyOperation>, RuntimeError> {
        let actual_windows = snapshot
            .windows
            .iter()
            .map(|window| (window.hwnd, window))
            .collect::<std::collections::HashMap<_, _>>();
        let desired_focused_hwnd = self
            .store
            .state()
            .focus
            .focused_window_id
            .and_then(|window_id| self.store.state().windows.get(&window_id))
            .filter(|window| window.is_managed)
            .and_then(|window| window.current_hwnd_binding);
        let actual_focused_hwnd = snapshot.focused_window().map(|window| window.hwnd);
        let mut operations = Vec::new();

        for workspace in self.store.state().workspaces.values() {
            if self.store.state().is_workspace_empty(workspace.id) {
                continue;
            }

            let projection = recompute_workspace(self.store.state(), workspace.id)?;
            for geometry in projection.window_geometries {
                let Some(window) = self.store.state().windows.get(&geometry.window_id) else {
                    continue;
                };
                if !window.is_managed {
                    continue;
                }
                let Some(hwnd) = window.current_hwnd_binding else {
                    continue;
                };
                let Some(actual_window) = actual_windows.get(&hwnd) else {
                    continue;
                };
                let activate = desired_focused_hwnd
                    .filter(|desired_hwnd| *desired_hwnd == hwnd)
                    .is_some_and(|desired_hwnd| {
                        needs_activation_apply(actual_focused_hwnd, desired_hwnd)
                    });
                if needs_geometry_apply(actual_window.rect, geometry.rect) || activate {
                    operations.push(ApplyOperation {
                        hwnd,
                        rect: geometry.rect,
                        activate,
                    });
                }
            }
        }

        Ok(operations)
    }

    fn sync_monitors_from_snapshot(
        &mut self,
        monitors: &[PlatformMonitorSnapshot],
    ) -> Result<(), RuntimeError> {
        if monitors.is_empty() {
            return Err(RuntimeError::NoPlatformMonitors);
        }

        let known_bindings = self
            .store
            .state()
            .monitors
            .values()
            .filter_map(|monitor| monitor.platform_binding.clone())
            .collect::<Vec<_>>();

        for monitor_snapshot in monitors {
            if let Some(monitor_id) = self.monitor_id_by_binding(&monitor_snapshot.binding) {
                let workspace_set_id = {
                    let state = self.store.state_mut();
                    let monitor = state
                        .monitors
                        .get_mut(&monitor_id)
                        .expect("known monitor should exist");
                    monitor.platform_binding = Some(monitor_snapshot.binding.clone());
                    monitor.work_area_rect = monitor_snapshot.work_area_rect;
                    monitor.dpi = monitor_snapshot.dpi;
                    monitor.is_primary_hint = monitor_snapshot.is_primary;
                    monitor.topology_role = if monitor_snapshot.is_primary {
                        TopologyRole::Primary
                    } else {
                        TopologyRole::Secondary
                    };
                    monitor.workspace_set_id
                };

                let workspace_ids = self
                    .store
                    .state()
                    .workspace_sets
                    .get(&workspace_set_id)
                    .map(|workspace_set| workspace_set.ordered_workspace_ids.clone())
                    .unwrap_or_default();

                for workspace_id in workspace_ids {
                    if let Some(workspace) =
                        self.store.state_mut().workspaces.get_mut(&workspace_id)
                    {
                        workspace.monitor_id = monitor_id;
                        workspace.strip.visible_region = monitor_snapshot.work_area_rect;
                    }
                }
            } else {
                let monitor_id = self.store.state_mut().add_monitor(
                    monitor_snapshot.work_area_rect,
                    monitor_snapshot.dpi,
                    monitor_snapshot.is_primary,
                );
                if let Some(monitor) = self.store.state_mut().monitors.get_mut(&monitor_id) {
                    monitor.platform_binding = Some(monitor_snapshot.binding.clone());
                }
            }
        }

        let fallback_monitor = monitors
            .iter()
            .find(|monitor| monitor.is_primary)
            .or_else(|| monitors.first())
            .cloned();

        for missing_binding in known_bindings
            .into_iter()
            .filter(|binding| !monitors.iter().any(|monitor| monitor.binding == *binding))
        {
            self.push_degraded_reason(format!("missing-monitor:{missing_binding}"));

            let Some(fallback_monitor) = &fallback_monitor else {
                continue;
            };
            let Some(monitor_id) = self.monitor_id_by_binding(&missing_binding) else {
                continue;
            };

            let workspace_set_id = {
                let state = self.store.state_mut();
                let monitor = state
                    .monitors
                    .get_mut(&monitor_id)
                    .expect("known monitor should exist");
                monitor.work_area_rect = fallback_monitor.work_area_rect;
                monitor.dpi = fallback_monitor.dpi;
                monitor.is_primary_hint = false;
                monitor.topology_role = TopologyRole::Secondary;
                monitor.workspace_set_id
            };

            let workspace_ids = self
                .store
                .state()
                .workspace_sets
                .get(&workspace_set_id)
                .map(|workspace_set| workspace_set.ordered_workspace_ids.clone())
                .unwrap_or_default();

            for workspace_id in workspace_ids {
                if let Some(workspace) = self.store.state_mut().workspaces.get_mut(&workspace_id) {
                    workspace.strip.visible_region = fallback_monitor.work_area_rect;
                }
            }
        }

        if let Some(primary_monitor) = monitors
            .iter()
            .find(|monitor| monitor.is_primary)
            .or_else(|| monitors.first())
            && let Some(monitor_id) = self.monitor_id_by_binding(&primary_monitor.binding)
            && self.store.state().focus.focused_monitor_id.is_none()
        {
            self.store.state_mut().focus.focused_monitor_id = Some(monitor_id);
        }

        Ok(())
    }

    fn monitor_id_by_binding(&self, binding: &str) -> Option<MonitorId> {
        self.store
            .state()
            .monitors
            .iter()
            .find_map(|(monitor_id, monitor)| {
                (monitor.platform_binding.as_deref() == Some(binding)).then_some(*monitor_id)
            })
    }

    fn find_window_id_by_hwnd(&self, hwnd: u64) -> Option<WindowId> {
        self.store
            .state()
            .windows
            .iter()
            .find_map(|(window_id, window)| {
                (window.current_hwnd_binding == Some(hwnd)).then_some(*window_id)
            })
    }

    fn push_degraded_reason(&mut self, reason: String) {
        if !self.store.state().runtime.degraded_flags.contains(&reason) {
            self.store.state_mut().runtime.degraded_flags.push(reason);
        }
    }

    fn note_observation_reason(&mut self, reason: &str) {
        let token = normalize_reason_token(reason);
        if token.contains("resume") {
            self.push_degraded_reason("resume-revalidation".to_string());
        }
        if token.contains("display") {
            self.push_degraded_reason("display-settings-changed".to_string());
        }
        if token.contains("monitor") {
            self.push_degraded_reason("monitor-topology-revalidation".to_string());
        }
    }

    fn next_correlation_id(&mut self) -> CorrelationId {
        let correlation_id = CorrelationId::new(self.next_correlation_id);
        self.next_correlation_id += 1;
        correlation_id
    }
}

fn diff_config_sections(previous: &LoadedConfig, current: &LoadedConfig) -> Vec<String> {
    let mut changed_sections = Vec::new();

    if previous.projection.strip_scroll_step != current.projection.strip_scroll_step
        || previous.projection.default_column_mode != current.projection.default_column_mode
        || previous.projection.default_column_width != current.projection.default_column_width
    {
        changed_sections.push("layout".to_string());
    }
    if previous.projection.bind_control_mode != current.projection.bind_control_mode
        || previous.hotkeys != current.hotkeys
    {
        changed_sections.push("input".to_string());
    }
    if previous.rules != current.rules {
        changed_sections.push("rules".to_string());
    }
    if changed_sections.is_empty() {
        changed_sections.push("none".to_string());
    }

    changed_sections
}

fn ensure_supported_bind_control_mode(
    bind_control_mode: BindControlMode,
) -> Result<(), RuntimeError> {
    match bind_control_mode {
        BindControlMode::Coexistence => Ok(()),
        _ => Err(RuntimeError::Config(format!(
            "bind control mode '{}' is not supported by the current runtime slice; only 'coexistence' is available",
            bind_control_mode.as_str()
        ))),
    }
}

fn workspace_path(relative_path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(relative_path)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn normalize_reason_token(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
}
