//! Versioned per-metric protocol. A missing value never means a valid zero.

use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetricId {
    Cpu,
    Gpu,
    Memory,
    Network,
    Disk,
}

impl MetricId {
    pub const ALL: [Self; 5] = [
        Self::Cpu,
        Self::Gpu,
        Self::Memory,
        Self::Network,
        Self::Disk,
    ];

    pub const fn interval_ms(self) -> u64 {
        match self {
            Self::Cpu | Self::Gpu => 2_000,
            Self::Memory => 3_000,
            Self::Network => 1_000,
            Self::Disk => 30_000,
        }
    }

    pub const fn freshness_ms(self) -> u64 {
        match self {
            Self::Cpu | Self::Gpu | Self::Network => 5_000,
            Self::Memory => 10_000,
            Self::Disk => 90_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MetricStatus {
    Loading,
    Ready,
    Stale,
    Disconnected,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricReading<T> {
    pub status: MetricStatus,
    pub sampled_at_ms: Option<u64>,
    pub value: Option<T>,
}

impl<T> Default for MetricReading<T> {
    fn default() -> Self {
        Self {
            status: MetricStatus::Loading,
            sampled_at_ms: None,
            value: None,
        }
    }
}

impl<T> MetricReading<T> {
    pub fn ready(value: T, sampled_at_ms: u64) -> Self {
        Self {
            status: MetricStatus::Ready,
            sampled_at_ms: Some(sampled_at_ms),
            value: Some(value),
        }
    }

    /// Retain the last observation after failure or expiry, never after source invalidation.
    pub(crate) fn mark_unavailable(&mut self, status: MetricStatus) {
        self.status = status;
        if !matches!(status, MetricStatus::Failed | MetricStatus::Stale) {
            self.value = None;
            self.sampled_at_ms = None;
        }
    }

    pub fn expire(&mut self, now_ms: u64, lifetime_ms: u64) {
        if self.status == MetricStatus::Ready
            && self.sampled_at_ms.is_some_and(|sampled| {
                now_ms < sampled || now_ms.saturating_sub(sampled) > lifetime_ms
            })
        {
            self.status = MetricStatus::Stale;
        }
    }
    pub(crate) fn expire_retained(
        &mut self,
        now_ms: u64,
        lifetime_ms: u64,
        retention_ms: u64,
    ) -> bool {
        let status = self.status;
        self.expire(now_ms, lifetime_ms);
        let expired = self
            .sampled_at_ms
            .is_some_and(|at| now_ms < at || now_ms - at > retention_ms);
        if expired {
            self.value = None;
            self.sampled_at_ms = None;
        }
        expired || status != self.status
    }
}

/// Volatile temperature diagnostics remain visible briefly, with their original age.
pub(crate) const TEMPERATURE_RETENTION_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuUsage {
    pub used_percent: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendPoint {
    pub sampled_at_ms: u64,
    pub primary: f64,
    pub secondary: Option<f64>,
    /// A real interruption precedes this sample, independently of retry cadence.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub gap_before: bool,
}

// The UI plays a buffered minute and needs its offscreen endpoints again after
// reopening. Keep a small margin in Core as well as in the mounted chart.
const HISTORY_RETENTION_MS: u64 = 80_000;
const HISTORY_POINT_LIMIT: usize = 96;

/// Immutable snapshots share their allocation without changing JSON arrays.
pub type TrendHistory = Arc<[TrendPoint]>;

/// Both time and count bounds apply, including duplicate or backwards wall times.
#[derive(Debug, Default)]
pub struct Trend {
    points: VecDeque<TrendPoint>,
    cached: TrendHistory,
    cached_from: usize,
    changed: bool,
}

impl Trend {
    pub fn push(&mut self, point: TrendPoint) {
        if self
            .points
            .back()
            .is_some_and(|last| last.sampled_at_ms >= point.sampled_at_ms)
        {
            self.points.clear();
        }
        self.points.push_back(point);
        self.changed = true;
        while self.points.len() > HISTORY_POINT_LIMIT
            || self.points.front().is_some_and(|first| {
                point.sampled_at_ms.saturating_sub(first.sampled_at_ms) > HISTORY_RETENTION_MS
            })
        {
            self.points.pop_front();
        }
    }

    pub fn snapshot(&mut self, now_ms: u64) -> TrendHistory {
        let from = self
            .points
            .iter()
            .position(|point| now_ms.saturating_sub(point.sampled_at_ms) <= HISTORY_RETENTION_MS)
            .unwrap_or(self.points.len());
        if self.changed || from != self.cached_from {
            self.cached = self.points.iter().skip(from).copied().collect();
            self.cached_from = from;
            self.changed = false;
        }
        Arc::clone(&self.cached)
    }

    pub fn clear(&mut self) {
        self.points.clear();
        self.cached = Default::default();
        self.cached_from = 0;
        self.changed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_share_unchanged_history_and_preserve_older_publications() {
        let mut trend = Trend::default();
        trend.push(TrendPoint {
            sampled_at_ms: 1000,
            primary: 42.0,
            secondary: None,
            gap_before: false,
        });
        let first = trend.snapshot(1000);
        assert!(Arc::ptr_eq(&first, &trend.snapshot(2000)));
        trend.push(TrendPoint {
            sampled_at_ms: 3000,
            primary: 84.0,
            secondary: None,
            gap_before: false,
        });
        let second = trend.snapshot(3000);
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 2);
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            r#"[{"sampledAtMs":1000,"primary":42.0,"secondary":null}]"#
        );
        trend.clear();
        assert!(trend.snapshot(4000).is_empty());
        assert_eq!(second.len(), 2);
    }

    #[test]
    fn cached_history_keeps_exact_retention_and_backwards_clock_semantics() {
        let mut trend = Trend::default();
        for sampled_at_ms in [1000, 2000] {
            trend.push(TrendPoint {
                sampled_at_ms,
                primary: 1.0,
                secondary: None,
                gap_before: false,
            });
        }
        let original = trend.snapshot(2000);
        assert!(Arc::ptr_eq(&original, &trend.snapshot(81000)));
        let expired = trend.snapshot(81001);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].sampled_at_ms, 2000);
        assert_eq!(original.len(), 2);
        assert_eq!(trend.snapshot(2000).len(), 2);
        assert!(trend.snapshot(82001).is_empty());
    }

    #[test]
    fn reopened_charts_receive_the_buffered_minute_and_offscreen_endpoints() {
        let mut trend = Trend::default();
        for sampled_at_ms in (0..=120_000).step_by(1000) {
            trend.push(TrendPoint {
                sampled_at_ms,
                primary: 10.0,
                secondary: None,
                gap_before: false,
            });
        }
        let history = trend.snapshot(120_500);
        assert!(history[0].sampled_at_ms < 120_500 - 60_000 - 3_250 - 6_000);
        assert!(history.len() <= HISTORY_POINT_LIMIT);
        assert!(trend.snapshot(201_000).is_empty());
    }

    #[test]
    fn a_backwards_wall_clock_never_keeps_a_future_sample_ready() {
        let mut reading = MetricReading::ready(1, 1000);
        reading.expire(999, 5000);
        assert_eq!(reading.status, MetricStatus::Stale);
    }

    #[test]
    fn missing_sample_is_loading_and_expiry_preserves_diagnostic_value() {
        let missing = MetricReading::<CpuUsage>::default();
        assert_eq!(missing.status, MetricStatus::Loading);
        assert!(missing.value.is_none());
        let mut value = MetricReading::ready(CpuUsage { used_percent: 12.0 }, 1_000);
        value.expire(6_000, MetricId::Cpu.freshness_ms());
        assert_eq!(value.status, MetricStatus::Ready);
        value.expire(6_001, MetricId::Cpu.freshness_ms());
        assert_eq!(value.status, MetricStatus::Stale);
        assert_eq!(value.value.unwrap().used_percent, 12.0);
    }

    #[test]
    fn trends_are_bounded_and_never_fabricate_old_history() {
        let mut trend = Trend::default();
        for sampled_at_ms in 0..500 {
            trend.push(TrendPoint {
                sampled_at_ms,
                primary: 1.0,
                secondary: None,
                gap_before: false,
            });
        }
        assert_eq!(trend.snapshot(500).len(), HISTORY_POINT_LIMIT);
        assert!(trend.snapshot(81_000).is_empty());
        trend.push(TrendPoint {
            sampled_at_ms: 1,
            primary: 2.0,
            secondary: None,
            gap_before: false,
        });
        assert_eq!(trend.snapshot(1).len(), 1);
    }
}
