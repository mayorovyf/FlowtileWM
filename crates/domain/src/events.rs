use crate::{
    ColumnMode, CorrelationId, FocusOrigin, MonitorId, Rect, Size, StateVersion, WidthSemantics,
    WindowClassification, WindowId, WindowLayer,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventCategory {
    PlatformDerived,
    UserInputDerived,
    ConfigRulesDerived,
    IpcDerived,
    SystemRecovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventSource {
    WindowsAdapter,
    InputCommand,
    ConfigRules,
    IpcClient,
    WmCore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainEventName {
    WindowDiscovered,
    WindowDestroyed,
    WindowShown,
    WindowHidden,
    WindowLocationChanged,
    WindowFocusObserved,
    MonitorTopologyChanged,
    SystemSuspend,
    SystemResume,
    ExplorerRestartObserved,
    CmdFocusNext,
    CmdFocusPrev,
    CmdMoveWindow,
    CmdToggleFloating,
    CmdToggleTabbed,
    CmdToggleOverview,
    CmdEmergencyUnwind,
    ConfigReloadRequested,
    ConfigReloadSucceeded,
    ConfigReloadFailed,
    RulesUpdated,
    IpcCommandReceived,
    IpcSnapshotRequested,
    IpcClientConnected,
    IpcClientDisconnected,
    ReconcileRequested,
    FullScanRequested,
    DesyncDetected,
    UiHostCrashed,
    CaptureModuleCrashed,
}

impl DomainEventName {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WindowDiscovered => "EVT-WINDOW-DISCOVERED",
            Self::WindowDestroyed => "EVT-WINDOW-DESTROYED",
            Self::WindowShown => "EVT-WINDOW-SHOWN",
            Self::WindowHidden => "EVT-WINDOW-HIDDEN",
            Self::WindowLocationChanged => "EVT-WINDOW-LOCATION-CHANGED",
            Self::WindowFocusObserved => "EVT-WINDOW-FOCUS-OBSERVED",
            Self::MonitorTopologyChanged => "EVT-MONITOR-TOPOLOGY-CHANGED",
            Self::SystemSuspend => "EVT-SYSTEM-SUSPEND",
            Self::SystemResume => "EVT-SYSTEM-RESUME",
            Self::ExplorerRestartObserved => "EVT-EXPLORER-RESTART-OBSERVED",
            Self::CmdFocusNext => "EVT-CMD-FOCUS-NEXT",
            Self::CmdFocusPrev => "EVT-CMD-FOCUS-PREV",
            Self::CmdMoveWindow => "EVT-CMD-MOVE-WINDOW",
            Self::CmdToggleFloating => "EVT-CMD-TOGGLE-FLOATING",
            Self::CmdToggleTabbed => "EVT-CMD-TOGGLE-TABBED",
            Self::CmdToggleOverview => "EVT-CMD-TOGGLE-OVERVIEW",
            Self::CmdEmergencyUnwind => "EVT-CMD-EMERGENCY-UNWIND",
            Self::ConfigReloadRequested => "EVT-CONFIG-RELOAD-REQUESTED",
            Self::ConfigReloadSucceeded => "EVT-CONFIG-RELOAD-SUCCEEDED",
            Self::ConfigReloadFailed => "EVT-CONFIG-RELOAD-FAILED",
            Self::RulesUpdated => "EVT-RULES-UPDATED",
            Self::IpcCommandReceived => "EVT-IPC-COMMAND-RECEIVED",
            Self::IpcSnapshotRequested => "EVT-IPC-SNAPSHOT-REQUESTED",
            Self::IpcClientConnected => "EVT-IPC-CLIENT-CONNECTED",
            Self::IpcClientDisconnected => "EVT-IPC-CLIENT-DISCONNECTED",
            Self::ReconcileRequested => "EVT-RECONCILE-REQUESTED",
            Self::FullScanRequested => "EVT-FULL-SCAN-REQUESTED",
            Self::DesyncDetected => "EVT-DESYNC-DETECTED",
            Self::UiHostCrashed => "EVT-UI-HOST-CRASHED",
            Self::CaptureModuleCrashed => "EVT-CAPTURE-MODULE-CRASHED",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusBehavior {
    FollowNewWindow,
    PreserveCurrentFocus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowPlacement {
    AppendToFocusedColumn,
    NewColumnAfterFocus {
        mode: ColumnMode,
        width: WidthSemantics,
    },
    NewColumnBeforeFocus {
        mode: ColumnMode,
        width: WidthSemantics,
    },
    AppendToWorkspaceEnd {
        mode: ColumnMode,
        width: WidthSemantics,
    },
}

impl Default for WindowPlacement {
    fn default() -> Self {
        Self::NewColumnAfterFocus {
            mode: ColumnMode::Normal,
            width: WidthSemantics::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowDiscoveredPayload {
    pub monitor_id: MonitorId,
    pub hwnd: u64,
    pub classification: WindowClassification,
    pub desired_size: Size,
    pub last_known_rect: Rect,
    pub placement: WindowPlacement,
    pub focus_behavior: FocusBehavior,
    pub layer: WindowLayer,
    pub managed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowDestroyedPayload {
    pub window_id: WindowId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowFocusObservedPayload {
    pub monitor_id: MonitorId,
    pub window_id: WindowId,
    pub focus_origin: FocusOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainEventPayload {
    WindowDiscovered(WindowDiscoveredPayload),
    WindowDestroyed(WindowDestroyedPayload),
    WindowFocusObserved(WindowFocusObservedPayload),
    ReconcileRequested,
    FullScanRequested,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainEvent {
    pub event_id: DomainEventName,
    pub event_kind: EventCategory,
    pub timestamp: u64,
    pub source: EventSource,
    pub correlation_id: CorrelationId,
    pub payload: DomainEventPayload,
    pub state_version_at_enqueue: Option<StateVersion>,
}

impl DomainEvent {
    pub const fn new(
        event_id: DomainEventName,
        event_kind: EventCategory,
        source: EventSource,
        correlation_id: CorrelationId,
        payload: DomainEventPayload,
    ) -> Self {
        Self {
            event_id,
            event_kind,
            timestamp: 0,
            source,
            correlation_id,
            payload,
            state_version_at_enqueue: None,
        }
    }

    pub fn window_discovered(
        correlation_id: CorrelationId,
        monitor_id: MonitorId,
        hwnd: u64,
        desired_size: Size,
        last_known_rect: Rect,
    ) -> Self {
        Self::window_discovered_with(
            correlation_id,
            monitor_id,
            hwnd,
            desired_size,
            last_known_rect,
            WindowPlacement::default(),
            FocusBehavior::FollowNewWindow,
        )
    }

    pub fn window_discovered_with(
        correlation_id: CorrelationId,
        monitor_id: MonitorId,
        hwnd: u64,
        desired_size: Size,
        last_known_rect: Rect,
        placement: WindowPlacement,
        focus_behavior: FocusBehavior,
    ) -> Self {
        Self::new(
            DomainEventName::WindowDiscovered,
            EventCategory::PlatformDerived,
            EventSource::WindowsAdapter,
            correlation_id,
            DomainEventPayload::WindowDiscovered(WindowDiscoveredPayload {
                monitor_id,
                hwnd,
                classification: WindowClassification::Application,
                desired_size,
                last_known_rect,
                placement,
                focus_behavior,
                layer: WindowLayer::Tiled,
                managed: true,
            }),
        )
    }

    pub fn window_destroyed(correlation_id: CorrelationId, window_id: WindowId) -> Self {
        Self::new(
            DomainEventName::WindowDestroyed,
            EventCategory::PlatformDerived,
            EventSource::WindowsAdapter,
            correlation_id,
            DomainEventPayload::WindowDestroyed(WindowDestroyedPayload { window_id }),
        )
    }

    pub fn window_focus_observed(
        correlation_id: CorrelationId,
        monitor_id: MonitorId,
        window_id: WindowId,
    ) -> Self {
        Self::new(
            DomainEventName::WindowFocusObserved,
            EventCategory::PlatformDerived,
            EventSource::WindowsAdapter,
            correlation_id,
            DomainEventPayload::WindowFocusObserved(WindowFocusObservedPayload {
                monitor_id,
                window_id,
                focus_origin: FocusOrigin::PlatformObservation,
            }),
        )
    }
}
