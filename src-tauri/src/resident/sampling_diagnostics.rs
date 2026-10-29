//! Bounded query timings stay separated so cheap sensors cannot hide slow enumeration.
use mangodisk_core::system_resources::metrics::MetricId;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy)]
pub enum Query {
    Cpu,
    Gpu,
    Memory,
    Network,
    Disk,
    DiskIo,
    CpuProcesses,
    CpuAuxiliary,
    DiskSensors,
}
impl From<MetricId> for Query {
    fn from(metric: MetricId) -> Self {
        match metric {
            MetricId::Cpu => Self::Cpu,
            MetricId::Gpu => Self::Gpu,
            MetricId::Memory => Self::Memory,
            MetricId::Network => Self::Network,
            MetricId::Disk => Self::Disk,
        }
    }
}
#[derive(Default)]
struct Timings {
    durations: VecDeque<u64>,
    discarded: u64,
}
#[derive(Default)]
pub struct SamplingDiagnostics {
    queries: [Timings; 9],
}
impl SamplingDiagnostics {
    pub fn record(&mut self, query: Query, duration_ms: u64) {
        let timings = &mut self.queries[query as usize];
        if timings.durations.len() == 1200 {
            timings.durations.pop_front();
        }
        timings.durations.push_back(duration_ms);
    }
    pub fn discard(&mut self, query: Query) {
        self.queries[query as usize].discarded += 1;
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn log_and_reset(&mut self, window_ms: u128) {
        for query in [
            Query::Cpu,
            Query::Gpu,
            Query::Memory,
            Query::Network,
            Query::Disk,
            Query::DiskIo,
            Query::CpuProcesses,
            Query::CpuAuxiliary,
            Query::DiskSensors,
        ] {
            let timings = &self.queries[query as usize];
            if timings.durations.is_empty() && timings.discarded == 0 {
                continue;
            }
            let mut durations: Vec<_> = timings.durations.iter().copied().collect();
            durations.sort_unstable();
            let percentile = |percent| {
                durations
                    .get(durations.len().saturating_sub(1) * percent / 100)
                    .copied()
                    .unwrap_or(0)
            };
            log::info!("resident_query_summary query={query:?} window_ms={window_ms} samples={} p50_ms={} p95_ms={} max_ms={} discarded={}", durations.len(), percentile(50), percentile(95), durations.last().copied().unwrap_or(0), timings.discarded);
        }
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_storage_is_bounded_and_sensors_remain_separate() {
        let mut diagnostics = SamplingDiagnostics::default();
        for duration in 0..2400 {
            diagnostics.record(Query::CpuProcesses, duration);
        }
        diagnostics.record(Query::Network, 1);
        diagnostics.discard(Query::CpuProcesses);
        let timings = &diagnostics.queries[Query::CpuProcesses as usize];
        assert_eq!(timings.durations.len(), 1200);
        assert_eq!(timings.durations.front(), Some(&1200));
        assert_eq!(timings.discarded, 1);
        assert_eq!(
            diagnostics.queries[Query::Network as usize].durations.len(),
            1
        );
        diagnostics.reset();
        assert!(diagnostics
            .queries
            .iter()
            .all(|timings| timings.durations.is_empty() && timings.discarded == 0));
    }
}
