//! Bounded query timings stay separated so cheap sensors cannot hide slow enumeration.
use std::collections::VecDeque;

pub use super::sampling_policy::Task as Query;
#[derive(Default)]
struct Timings {
    durations: VecDeque<u64>,
    discarded: u64,
    queue_ms: VecDeque<u64>,
    delivery_ms: VecDeque<u64>,
}
#[derive(Default)]
pub struct SamplingDiagnostics {
    queries: [Timings; Query::ALL.len()],
}
impl SamplingDiagnostics {
    pub fn record(&mut self, query: Query, duration_ms: u64) {
        let timings = &mut self.queries[query as usize];
        if timings.durations.len() == 1200 {
            timings.durations.pop_front();
        }
        timings.durations.push_back(duration_ms);
    }
    pub fn completed(&mut self, completion: &super::sampling_workers::Completion, now_ms: u64) {
        self.record(completion.task, completion.duration_ms);
        let timings = &mut self.queries[completion.task as usize];
        for (values, value) in [
            (&mut timings.queue_ms, completion.queue_ms),
            (
                &mut timings.delivery_ms,
                now_ms.saturating_sub(completion.monotonic_ms),
            ),
        ] {
            if values.len() == 1200 {
                values.pop_front();
            }
            values.push_back(value);
        }
    }
    pub fn discard(&mut self, query: Query) {
        self.queries[query as usize].discarded += 1;
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn log_and_reset(&mut self, window_ms: u128) {
        for query in Query::ALL {
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
            let percentile_of = |values: &VecDeque<u64>| {
                let mut sorted: Vec<_> = values.iter().copied().collect();
                sorted.sort_unstable();
                sorted
                    .get(sorted.len().saturating_sub(1) * 95 / 100)
                    .map_or_else(|| "unmeasured".to_owned(), u64::to_string)
            };
            let queue_p95_ms = percentile_of(&timings.queue_ms);
            let delivery_p95_ms = percentile_of(&timings.delivery_ms);
            log::info!("resident_query_summary query={query:?} window_ms={window_ms} samples={} p50_ms={} p95_ms={} max_ms={} queue_p95_ms={queue_p95_ms} delivery_p95_ms={delivery_p95_ms} discarded={}", durations.len(), percentile(50), percentile(95), durations.last().copied().unwrap_or(0), timings.discarded);
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
