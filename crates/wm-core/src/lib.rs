#![forbid(unsafe_code)]

use flowtile_config_rules::bootstrap as config_bootstrap;
use flowtile_diagnostics::{
    DiagnosticRecord, bootstrap as diagnostics_bootstrap, layout_recomputed, transition_applied,
};
use flowtile_domain::{
    BootstrapProfile, Column, ColumnId, ColumnMode, DomainEvent, DomainEventPayload, FocusBehavior,
    FocusOrigin, MonitorId, RuntimeMode, StateVersion, WidthSemantics, WindowId, WindowLayer,
    WindowNode, WindowPlacement, WmState, WorkspaceId,
};
use flowtile_ipc::bootstrap as ipc_bootstrap;
use flowtile_layout_engine::{
    LayoutError, WorkspaceLayoutProjection, bootstrap_modes, preserves_insert_invariant,
    recompute_workspace,
};
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
        let state = WmState::new(runtime_mode);

        Self {
            profile: state.bootstrap_profile(),
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

#[derive(Debug, Eq, PartialEq)]
pub enum CoreError {
    UnknownMonitor(MonitorId),
    UnknownWorkspace(WorkspaceId),
    UnknownColumn(ColumnId),
    UnknownWindow(WindowId),
    NoActiveWorkspace(MonitorId),
    InvalidEvent(&'static str),
    Layout(LayoutError),
}

impl From<LayoutError> for CoreError {
    fn from(value: LayoutError) -> Self {
        Self::Layout(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionResult {
    pub state_version: StateVersion,
    pub affected_workspace_id: Option<WorkspaceId>,
    pub layout_projection: Option<WorkspaceLayoutProjection>,
    pub diagnostics: Vec<DiagnosticRecord>,
}

#[derive(Clone, Debug)]
pub struct StateStore {
    state: WmState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NewColumnRequest {
    anchor_column_id: Option<ColumnId>,
    before_anchor: bool,
    mode: ColumnMode,
    width_semantics: WidthSemantics,
    preserve_focus_position: bool,
}

impl StateStore {
    pub fn new(runtime_mode: RuntimeMode) -> Self {
        Self {
            state: WmState::new(runtime_mode),
        }
    }

    pub const fn state(&self) -> &WmState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut WmState {
        &mut self.state
    }

    pub fn dispatch(&mut self, event: DomainEvent) -> Result<TransitionResult, CoreError> {
        let affected_workspace_id = self.apply_event(&event)?;
        let state_version = self.state.bump_state_version();
        let mut diagnostics = vec![transition_applied(
            state_version,
            event.correlation_id,
            event.event_id.as_str(),
        )];
        let layout_projection = if let Some(workspace_id) = affected_workspace_id {
            let projection = recompute_workspace(&self.state, workspace_id)?;
            diagnostics.push(layout_recomputed(
                state_version,
                event.correlation_id,
                workspace_id,
                projection.window_geometries.len(),
            ));
            Some(projection)
        } else {
            None
        };

        self.state.diagnostics_summary.total_records += diagnostics.len() as u64;
        self.state.diagnostics_summary.last_transition_label =
            Some(event.event_id.as_str().to_string());
        self.state.diagnostics_summary.last_state_version = state_version;

        Ok(TransitionResult {
            state_version,
            affected_workspace_id,
            layout_projection,
            diagnostics,
        })
    }

    fn apply_event(&mut self, event: &DomainEvent) -> Result<Option<WorkspaceId>, CoreError> {
        match &event.payload {
            DomainEventPayload::WindowDiscovered(payload) => self.handle_window_discovered(payload),
            DomainEventPayload::WindowDestroyed(payload) => self.handle_window_destroyed(payload),
            DomainEventPayload::WindowFocusObserved(payload) => {
                self.handle_window_focus_observed(payload)
            }
            DomainEventPayload::ReconcileRequested => Ok(None),
            DomainEventPayload::FullScanRequested => Ok(None),
        }
    }

    fn handle_window_discovered(
        &mut self,
        payload: &flowtile_domain::WindowDiscoveredPayload,
    ) -> Result<Option<WorkspaceId>, CoreError> {
        if !self.state.monitors.contains_key(&payload.monitor_id) {
            return Err(CoreError::UnknownMonitor(payload.monitor_id));
        }

        let workspace_id = self
            .state
            .active_workspace_id_for_monitor(payload.monitor_id)
            .ok_or(CoreError::NoActiveWorkspace(payload.monitor_id))?;
        let should_preserve_current_focus =
            matches!(payload.focus_behavior, FocusBehavior::PreserveCurrentFocus)
                && self.focused_window_in_workspace(workspace_id).is_some();
        let focused_column_id = self.focused_column_in_workspace(workspace_id);
        let window_id = self.state.allocate_window_id();

        let target_column_id = match payload.layer {
            WindowLayer::Floating => {
                let workspace = self
                    .state
                    .workspaces
                    .get_mut(&workspace_id)
                    .ok_or(CoreError::UnknownWorkspace(workspace_id))?;
                workspace.floating_layer.ordered_window_ids.push(window_id);
                workspace.floating_layer.z_hints.insert(
                    window_id,
                    workspace.floating_layer.ordered_window_ids.len() as u32,
                );
                None
            }
            _ => Some(match payload.placement {
                WindowPlacement::AppendToFocusedColumn => {
                    if let Some(column_id) = focused_column_id {
                        let column = self
                            .state
                            .layout
                            .columns
                            .get_mut(&column_id)
                            .ok_or(CoreError::UnknownColumn(column_id))?;
                        column.ordered_window_ids.push(window_id);
                        if column.mode == ColumnMode::Tabbed {
                            column.tab_selection = Some(window_id);
                        }
                        column_id
                    } else {
                        self.insert_new_column(
                            workspace_id,
                            window_id,
                            NewColumnRequest {
                                anchor_column_id: None,
                                before_anchor: false,
                                mode: ColumnMode::Normal,
                                width_semantics: WidthSemantics::default(),
                                preserve_focus_position: false,
                            },
                        )?
                    }
                }
                WindowPlacement::NewColumnAfterFocus { mode, width } => self.insert_new_column(
                    workspace_id,
                    window_id,
                    NewColumnRequest {
                        anchor_column_id: focused_column_id,
                        before_anchor: false,
                        mode,
                        width_semantics: width,
                        preserve_focus_position: false,
                    },
                )?,
                WindowPlacement::NewColumnBeforeFocus { mode, width } => self.insert_new_column(
                    workspace_id,
                    window_id,
                    NewColumnRequest {
                        anchor_column_id: focused_column_id,
                        before_anchor: true,
                        mode,
                        width_semantics: width,
                        preserve_focus_position: should_preserve_current_focus,
                    },
                )?,
                WindowPlacement::AppendToWorkspaceEnd { mode, width } => self.insert_new_column(
                    workspace_id,
                    window_id,
                    NewColumnRequest {
                        anchor_column_id: None,
                        before_anchor: false,
                        mode,
                        width_semantics: width,
                        preserve_focus_position: false,
                    },
                )?,
            }),
        };

        self.state.windows.insert(
            window_id,
            WindowNode {
                id: window_id,
                current_hwnd_binding: Some(payload.hwnd),
                classification: payload.classification,
                layer: payload.layer,
                workspace_id,
                column_id: target_column_id,
                is_managed: payload.managed,
                is_floating: payload.layer == WindowLayer::Floating,
                is_fullscreen: payload.layer == WindowLayer::Fullscreen,
                restore_target: None,
                last_known_rect: payload.last_known_rect,
                desired_size: payload.desired_size,
            },
        );

        self.state
            .focus
            .active_workspace_by_monitor
            .insert(payload.monitor_id, workspace_id);

        if let Some(workspace_set_id) = self.state.workspace_set_id_for_monitor(payload.monitor_id)
            && let Some(workspace_set) = self.state.workspace_sets.get_mut(&workspace_set_id)
        {
            workspace_set.active_workspace_id = workspace_id;
        }

        if !should_preserve_current_focus {
            self.state.focus.focused_monitor_id = Some(payload.monitor_id);
            self.state.focus.focused_window_id = Some(window_id);
            self.state.focus.focused_column_id = target_column_id;
            self.state.focus.focus_origin = FocusOrigin::ReducerDefault;
        }

        self.state.ensure_tail_workspace(payload.monitor_id);
        Ok(Some(workspace_id))
    }

    fn handle_window_destroyed(
        &mut self,
        payload: &flowtile_domain::WindowDestroyedPayload,
    ) -> Result<Option<WorkspaceId>, CoreError> {
        let window = self
            .state
            .windows
            .remove(&payload.window_id)
            .ok_or(CoreError::UnknownWindow(payload.window_id))?;
        let workspace_id = window.workspace_id;
        let monitor_id = self
            .state
            .workspaces
            .get(&workspace_id)
            .map(|workspace| workspace.monitor_id)
            .ok_or(CoreError::UnknownWorkspace(workspace_id))?;

        if let Some(column_id) = window.column_id {
            let mut column_is_empty = false;
            if let Some(column) = self.state.layout.columns.get_mut(&column_id) {
                column
                    .ordered_window_ids
                    .retain(|window_id| *window_id != payload.window_id);
                if column.tab_selection == Some(payload.window_id) {
                    column.tab_selection = column.ordered_window_ids.first().copied();
                }
                column_is_empty = column.ordered_window_ids.is_empty();
            }

            if column_is_empty {
                self.state.layout.columns.remove(&column_id);
                let workspace = self
                    .state
                    .workspaces
                    .get_mut(&workspace_id)
                    .ok_or(CoreError::UnknownWorkspace(workspace_id))?;
                workspace
                    .strip
                    .ordered_column_ids
                    .retain(|existing_id| *existing_id != column_id);
            }
        } else {
            let workspace = self
                .state
                .workspaces
                .get_mut(&workspace_id)
                .ok_or(CoreError::UnknownWorkspace(workspace_id))?;
            workspace
                .floating_layer
                .ordered_window_ids
                .retain(|window_id| *window_id != payload.window_id);
            workspace.floating_layer.z_hints.remove(&payload.window_id);
        }

        if self.state.focus.focused_window_id == Some(payload.window_id) {
            self.retarget_focus_after_destroy(workspace_id)?;
        }

        self.state.ensure_tail_workspace(monitor_id);
        Ok(Some(workspace_id))
    }

    fn handle_window_focus_observed(
        &mut self,
        payload: &flowtile_domain::WindowFocusObservedPayload,
    ) -> Result<Option<WorkspaceId>, CoreError> {
        if !self.state.monitors.contains_key(&payload.monitor_id) {
            return Err(CoreError::UnknownMonitor(payload.monitor_id));
        }

        let window = self
            .state
            .windows
            .get(&payload.window_id)
            .ok_or(CoreError::UnknownWindow(payload.window_id))?;
        let workspace_id = window.workspace_id;
        let workspace = self
            .state
            .workspaces
            .get(&workspace_id)
            .ok_or(CoreError::UnknownWorkspace(workspace_id))?;

        if workspace.monitor_id != payload.monitor_id {
            return Err(CoreError::InvalidEvent(
                "focused window monitor does not match workspace monitor",
            ));
        }

        self.state.focus.focused_monitor_id = Some(payload.monitor_id);
        self.state.focus.focused_window_id = Some(payload.window_id);
        self.state.focus.focused_column_id = window.column_id;
        self.state.focus.focus_origin = payload.focus_origin;
        self.state
            .focus
            .active_workspace_by_monitor
            .insert(payload.monitor_id, workspace_id);

        if let Some(workspace_set_id) = self.state.workspace_set_id_for_monitor(payload.monitor_id)
            && let Some(workspace_set) = self.state.workspace_sets.get_mut(&workspace_set_id)
        {
            workspace_set.active_workspace_id = workspace_id;
        }

        Ok(Some(workspace_id))
    }

    fn insert_new_column(
        &mut self,
        workspace_id: WorkspaceId,
        window_id: WindowId,
        request: NewColumnRequest,
    ) -> Result<ColumnId, CoreError> {
        let column_id = self.state.allocate_column_id();
        self.state.layout.columns.insert(
            column_id,
            Column::new(
                column_id,
                request.mode,
                request.width_semantics,
                vec![window_id],
            ),
        );

        let workspace = self
            .state
            .workspaces
            .get_mut(&workspace_id)
            .ok_or(CoreError::UnknownWorkspace(workspace_id))?;
        let insert_index = request
            .anchor_column_id
            .and_then(|anchor| {
                workspace
                    .strip
                    .ordered_column_ids
                    .iter()
                    .position(|column_id| *column_id == anchor)
                    .map(|index| {
                        if request.before_anchor {
                            index
                        } else {
                            index + 1
                        }
                    })
            })
            .unwrap_or({
                if request.before_anchor {
                    0
                } else {
                    workspace.strip.ordered_column_ids.len()
                }
            });

        workspace
            .strip
            .ordered_column_ids
            .insert(insert_index, column_id);

        if request.preserve_focus_position
            && request.before_anchor
            && request.anchor_column_id.is_some()
        {
            let width = request
                .width_semantics
                .resolve(workspace.strip.visible_region.width)
                .min(i32::MAX as u32) as i32;
            workspace.strip.scroll_offset = workspace.strip.scroll_offset.saturating_add(width);
        }

        Ok(column_id)
    }

    fn focused_window_in_workspace(&self, workspace_id: WorkspaceId) -> Option<WindowId> {
        self.state.focus.focused_window_id.filter(|window_id| {
            self.state
                .windows
                .get(window_id)
                .is_some_and(|window| window.workspace_id == workspace_id)
        })
    }

    fn focused_column_in_workspace(&self, workspace_id: WorkspaceId) -> Option<ColumnId> {
        let focused_column_id = self.state.focus.focused_column_id?;
        let workspace = self.state.workspaces.get(&workspace_id)?;
        workspace
            .strip
            .ordered_column_ids
            .contains(&focused_column_id)
            .then_some(focused_column_id)
    }

    fn retarget_focus_after_destroy(&mut self, workspace_id: WorkspaceId) -> Result<(), CoreError> {
        let workspace = self
            .state
            .workspaces
            .get(&workspace_id)
            .ok_or(CoreError::UnknownWorkspace(workspace_id))?;
        let monitor_id = workspace.monitor_id;
        let next_focus = workspace
            .strip
            .ordered_column_ids
            .iter()
            .find_map(|column_id| {
                let column = self.state.layout.columns.get(column_id)?;
                let window_id = column
                    .tab_selection
                    .or_else(|| column.ordered_window_ids.first().copied())?;
                Some((window_id, Some(*column_id)))
            })
            .or_else(|| {
                workspace
                    .floating_layer
                    .ordered_window_ids
                    .first()
                    .copied()
                    .map(|window_id| (window_id, None))
            });

        self.state.focus.focused_monitor_id = Some(monitor_id);
        self.state.focus.focus_origin = FocusOrigin::ReducerDefault;

        if let Some((window_id, column_id)) = next_focus {
            self.state.focus.focused_window_id = Some(window_id);
            self.state.focus.focused_column_id = column_id;
        } else {
            self.state.focus.focused_window_id = None;
            self.state.focus.focused_column_id = None;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use flowtile_domain::{
        ColumnMode, CorrelationId, DomainEvent, FocusBehavior, Rect, RuntimeMode, Size,
        WidthSemantics, WindowPlacement,
    };

    use super::{CoreDaemonBootstrap, StateStore};

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

    #[test]
    fn discovery_creates_tail_workspace_and_diagnostics() {
        let mut store = StateStore::new(RuntimeMode::WmOnly);
        let monitor_id = store
            .state_mut()
            .add_monitor(Rect::new(0, 0, 1600, 900), 96, true);

        let result = store
            .dispatch(DomainEvent::window_discovered_with(
                CorrelationId::new(1),
                monitor_id,
                100,
                Size::new(420, 900),
                Rect::new(0, 0, 420, 900),
                WindowPlacement::AppendToWorkspaceEnd {
                    mode: ColumnMode::Normal,
                    width: WidthSemantics::Fixed(420),
                },
                FocusBehavior::FollowNewWindow,
            ))
            .expect("dispatch should succeed");

        let workspace_set_id = store
            .state()
            .workspace_set_id_for_monitor(monitor_id)
            .expect("workspace set should exist");
        let workspace_set = store
            .state()
            .workspace_sets
            .get(&workspace_set_id)
            .expect("workspace set should exist");
        let tail_workspace_id = *workspace_set
            .ordered_workspace_ids
            .last()
            .expect("tail workspace should exist");

        assert_eq!(result.state_version.get(), 1);
        assert_eq!(result.diagnostics.len(), 2);
        assert_eq!(store.state().workspaces.len(), 2);
        assert!(store.state().is_workspace_empty(tail_workspace_id));
        assert!(
            store
                .state()
                .workspaces
                .get(&tail_workspace_id)
                .expect("tail workspace should exist")
                .is_ephemeral_empty_tail
        );
    }

    #[test]
    fn inserting_new_column_does_not_resize_existing_column() {
        let mut store = StateStore::new(RuntimeMode::WmOnly);
        let monitor_id = store
            .state_mut()
            .add_monitor(Rect::new(0, 0, 1600, 900), 96, true);

        let first = store
            .dispatch(DomainEvent::window_discovered_with(
                CorrelationId::new(1),
                monitor_id,
                100,
                Size::new(420, 900),
                Rect::new(0, 0, 420, 900),
                WindowPlacement::AppendToWorkspaceEnd {
                    mode: ColumnMode::Normal,
                    width: WidthSemantics::Fixed(420),
                },
                FocusBehavior::FollowNewWindow,
            ))
            .expect("first dispatch should succeed");
        let first_window_id = store
            .state()
            .focus
            .focused_window_id
            .expect("first window should be focused");
        let first_rect_before = geometry_x_width(
            first
                .layout_projection
                .as_ref()
                .expect("layout should exist"),
            first_window_id,
        );

        let second = store
            .dispatch(DomainEvent::window_discovered_with(
                CorrelationId::new(2),
                monitor_id,
                101,
                Size::new(360, 900),
                Rect::new(420, 0, 360, 900),
                WindowPlacement::NewColumnAfterFocus {
                    mode: ColumnMode::Normal,
                    width: WidthSemantics::Fixed(360),
                },
                FocusBehavior::FollowNewWindow,
            ))
            .expect("second dispatch should succeed");
        let first_rect_after = geometry_x_width(
            second
                .layout_projection
                .as_ref()
                .expect("layout should exist"),
            first_window_id,
        );

        assert_eq!(first_rect_before, first_rect_after);
    }

    #[test]
    fn inserting_before_focus_keeps_visual_position_stable() {
        let mut store = StateStore::new(RuntimeMode::WmOnly);
        let monitor_id = store
            .state_mut()
            .add_monitor(Rect::new(0, 0, 1600, 900), 96, true);

        store
            .dispatch(DomainEvent::window_discovered_with(
                CorrelationId::new(1),
                monitor_id,
                100,
                Size::new(420, 900),
                Rect::new(0, 0, 420, 900),
                WindowPlacement::AppendToWorkspaceEnd {
                    mode: ColumnMode::Normal,
                    width: WidthSemantics::Fixed(420),
                },
                FocusBehavior::FollowNewWindow,
            ))
            .expect("first dispatch should succeed");

        let second = store
            .dispatch(DomainEvent::window_discovered_with(
                CorrelationId::new(2),
                monitor_id,
                101,
                Size::new(360, 900),
                Rect::new(420, 0, 360, 900),
                WindowPlacement::NewColumnAfterFocus {
                    mode: ColumnMode::Normal,
                    width: WidthSemantics::Fixed(360),
                },
                FocusBehavior::FollowNewWindow,
            ))
            .expect("second dispatch should succeed");
        let focused_window_id = store
            .state()
            .focus
            .focused_window_id
            .expect("second window should be focused");
        let focused_x_before = geometry_x_width(
            second
                .layout_projection
                .as_ref()
                .expect("layout should exist"),
            focused_window_id,
        )
        .0;

        let third = store
            .dispatch(DomainEvent::window_discovered_with(
                CorrelationId::new(3),
                monitor_id,
                102,
                Size::new(220, 900),
                Rect::new(0, 0, 220, 900),
                WindowPlacement::NewColumnBeforeFocus {
                    mode: ColumnMode::Normal,
                    width: WidthSemantics::Fixed(220),
                },
                FocusBehavior::PreserveCurrentFocus,
            ))
            .expect("third dispatch should succeed");
        let focused_x_after = geometry_x_width(
            third
                .layout_projection
                .as_ref()
                .expect("layout should exist"),
            focused_window_id,
        )
        .0;

        assert_eq!(focused_x_before, focused_x_after);
        assert_eq!(
            store.state().focus.focused_window_id,
            Some(focused_window_id)
        );
        assert_eq!(
            third
                .layout_projection
                .as_ref()
                .expect("layout should exist")
                .scroll_offset,
            220
        );
    }

    #[test]
    fn destroying_last_window_collapses_extra_empty_tail() {
        let mut store = StateStore::new(RuntimeMode::WmOnly);
        let monitor_id = store
            .state_mut()
            .add_monitor(Rect::new(0, 0, 1600, 900), 96, true);

        store
            .dispatch(DomainEvent::window_discovered_with(
                CorrelationId::new(1),
                monitor_id,
                100,
                Size::new(420, 900),
                Rect::new(0, 0, 420, 900),
                WindowPlacement::AppendToWorkspaceEnd {
                    mode: ColumnMode::Normal,
                    width: WidthSemantics::Fixed(420),
                },
                FocusBehavior::FollowNewWindow,
            ))
            .expect("dispatch should succeed");
        let window_id = store
            .state()
            .focus
            .focused_window_id
            .expect("window should be focused");

        store
            .dispatch(DomainEvent::window_destroyed(
                CorrelationId::new(2),
                window_id,
            ))
            .expect("destroy should succeed");

        let workspace_set_id = store
            .state()
            .workspace_set_id_for_monitor(monitor_id)
            .expect("workspace set should exist");
        let workspace_set = store
            .state()
            .workspace_sets
            .get(&workspace_set_id)
            .expect("workspace set should exist");
        let remaining_workspace_id = workspace_set.active_workspace_id;

        assert_eq!(workspace_set.ordered_workspace_ids.len(), 1);
        assert!(store.state().is_workspace_empty(remaining_workspace_id));
        assert_eq!(store.state().focus.focused_window_id, None);
    }

    fn geometry_x_width(
        projection: &WorkspaceLayoutProjection,
        window_id: flowtile_domain::WindowId,
    ) -> (i32, u32) {
        projection
            .window_geometries
            .iter()
            .find(|geometry| geometry.window_id == window_id)
            .map(|geometry| (geometry.rect.x, geometry.rect.width))
            .expect("window geometry should exist")
    }

    use flowtile_layout_engine::WorkspaceLayoutProjection;
}
