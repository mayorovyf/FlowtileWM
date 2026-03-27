#![forbid(unsafe_code)]

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use flowtile_domain::{CorrelationId, StateVersion, WorkspaceId};
use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PerfMetricSnapshot {
    pub metric: String,
    pub samples: u64,
    pub total_duration_us: u64,
    pub average_duration_us: u64,
    pub max_duration_us: u64,
    pub last_duration_us: u64,
    pub error_count: u64,
    pub skip_count: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PerfTelemetrySnapshot {
    pub metrics: Vec<PerfMetricSnapshot>,
}

#[derive(Debug, Default)]
pub struct AtomicPerfMetric {
    samples: AtomicU64,
    total_duration_us: AtomicU64,
    max_duration_us: AtomicU64,
    last_duration_us: AtomicU64,
    error_count: AtomicU64,
    skip_count: AtomicU64,
}

impl AtomicPerfMetric {
    pub const fn new() -> Self {
        Self {
            samples: AtomicU64::new(0),
            total_duration_us: AtomicU64::new(0),
            max_duration_us: AtomicU64::new(0),
            last_duration_us: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            skip_count: AtomicU64::new(0),
        }
    }

    pub fn record_duration(&self, duration: Duration) {
        let duration_us = duration_to_micros(duration);
        saturating_increment(&self.samples);
        saturating_add(&self.total_duration_us, duration_us);
        self.last_duration_us.store(duration_us, Ordering::Relaxed);
        self.max_duration_us
            .fetch_max(duration_us, Ordering::Relaxed);
    }

    pub fn record_error(&self) {
        saturating_increment(&self.error_count);
    }

    pub fn record_skip(&self) {
        saturating_increment(&self.skip_count);
    }

    pub fn snapshot(&self, metric: impl Into<String>) -> PerfMetricSnapshot {
        let samples = self.samples.load(Ordering::Relaxed);
        let total_duration_us = self.total_duration_us.load(Ordering::Relaxed);

        PerfMetricSnapshot {
            metric: metric.into(),
            samples,
            total_duration_us,
            average_duration_us: average_duration_us(total_duration_us, samples),
            max_duration_us: self.max_duration_us.load(Ordering::Relaxed),
            last_duration_us: self.last_duration_us.load(Ordering::Relaxed),
            error_count: self.error_count.load(Ordering::Relaxed),
            skip_count: self.skip_count.load(Ordering::Relaxed),
        }
    }
}

fn duration_to_micros(duration: Duration) -> u64 {
    duration.as_micros().min(u64::MAX as u128) as u64
}

fn average_duration_us(total_duration_us: u64, samples: u64) -> u64 {
    if samples == 0 {
        0
    } else {
        total_duration_us / samples
    }
}

fn saturating_increment(target: &AtomicU64) {
    saturating_add(target, 1);
}

fn saturating_add(target: &AtomicU64, value: u64) {
    let _ = target.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(value))
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use flowtile_domain::{CorrelationId, StateVersion, WorkspaceId};

    use super::{
        AtomicPerfMetric, bootstrap, layout_recomputed, transition_applied, validation_error,
    };

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

    #[test]
    fn perf_metric_tracks_duration_error_and_skip_counts() {
        let metric = AtomicPerfMetric::new();

        metric.record_duration(Duration::from_micros(500));
        metric.record_duration(Duration::from_micros(1_500));
        metric.record_error();
        metric.record_skip();

        let snapshot = metric.snapshot("runtime.command-cycle");
        assert_eq!(snapshot.metric, "runtime.command-cycle");
        assert_eq!(snapshot.samples, 2);
        assert_eq!(snapshot.total_duration_us, 2_000);
        assert_eq!(snapshot.average_duration_us, 1_000);
        assert_eq!(snapshot.max_duration_us, 1_500);
        assert_eq!(snapshot.last_duration_us, 1_500);
        assert_eq!(snapshot.error_count, 1);
        assert_eq!(snapshot.skip_count, 1);
    }
}
