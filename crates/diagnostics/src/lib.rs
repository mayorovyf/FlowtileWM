#![forbid(unsafe_code)]

use flowtile_domain::{CorrelationId, StateVersion, WorkspaceId};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticStage {
    EventTransition,
    LayoutRecompute,
    Validation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRecord {
    pub level: DiagnosticLevel,
    pub stage: DiagnosticStage,
    pub message: String,
    pub state_version: Option<StateVersion>,
    pub correlation_id: Option<CorrelationId>,
}

pub fn transition_applied(
    state_version: StateVersion,
    correlation_id: CorrelationId,
    event_label: &str,
) -> DiagnosticRecord {
    DiagnosticRecord {
        level: DiagnosticLevel::Info,
        stage: DiagnosticStage::EventTransition,
        message: format!("applied transition for {event_label}"),
        state_version: Some(state_version),
        correlation_id: Some(correlation_id),
    }
}

pub fn layout_recomputed(
    state_version: StateVersion,
    correlation_id: CorrelationId,
    workspace_id: WorkspaceId,
    window_count: usize,
) -> DiagnosticRecord {
    DiagnosticRecord {
        level: DiagnosticLevel::Info,
        stage: DiagnosticStage::LayoutRecompute,
        message: format!(
            "recomputed workspace {} with {} projected windows",
            workspace_id, window_count
        ),
        state_version: Some(state_version),
        correlation_id: Some(correlation_id),
    }
}

pub fn validation_error(message: impl Into<String>) -> DiagnosticRecord {
    DiagnosticRecord {
        level: DiagnosticLevel::Error,
        stage: DiagnosticStage::Validation,
        message: message.into(),
        state_version: None,
        correlation_id: None,
    }
}

#[cfg(test)]
mod tests {
    use flowtile_domain::{CorrelationId, StateVersion, WorkspaceId};

    use super::{bootstrap, layout_recomputed, transition_applied, validation_error};

    #[test]
    fn keeps_state_dump_in_bootstrap_contract() {
        let bootstrap = bootstrap();
        assert!(bootstrap.supports_state_dump);
        assert!(bootstrap.channels.contains(&"structured-logs"));
    }

    #[test]
    fn emits_transition_and_layout_records() {
        let transition = transition_applied(
            StateVersion::new(3),
            CorrelationId::new(7),
            "EVT-WINDOW-DISCOVERED",
        );
        let layout = layout_recomputed(
            StateVersion::new(3),
            CorrelationId::new(7),
            WorkspaceId::new(2),
            3,
        );

        assert!(transition.message.contains("EVT-WINDOW-DISCOVERED"));
        assert!(layout.message.contains("workspace 2"));
    }

    #[test]
    fn emits_validation_record_without_state_version() {
        let record = validation_error("missing active workspace");
        assert!(record.state_version.is_none());
        assert!(record.message.contains("missing active workspace"));
    }
}
