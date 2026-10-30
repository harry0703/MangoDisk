//! Aggregate real sensor cadence separately from request cadence and cache hits.
use crate::system_resources::cpu::temperature::{
    CpuTemperature, CpuTemperatureSource, TEMPERATURE_INTERVAL_MS,
};
use crate::{PlatformError, PlatformErrorCode};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SUMMARY_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Diagnostics {
    previous_error: Option<(&'static str, PlatformError, u64)>,
    previous_sample: Option<(CpuTemperatureSource, u64)>,
    previous_request: Option<Instant>,
    reported_at: Option<Instant>,
    requests: u64,
    samples: u64,
    cache_hits: u64,
    cache_hits_since_sample: u64,
    max_sample_interval_ms: u64,
    max_request_interval_ms: u64,
    max_age_ms: u64,
}

impl Diagnostics {
    pub(super) fn observe_error(
        &mut self,
        model: Option<&str>,
        stage: &'static str,
        error: &PlatformError,
        retry_after_ms: u64,
    ) {
        if self
            .previous_error
            .as_ref()
            .is_some_and(|(previous_stage, previous, previous_retry)| {
                *previous_stage == stage
                    && *previous_retry == retry_after_ms
                    && previous.code() == error.code()
                    && previous.diagnostic() == error.diagnostic()
            })
        {
            return;
        }
        let model = crate::diagnostics::text(model.unwrap_or("unknown"));
        let diagnostic = crate::diagnostics::text(error);
        if error.code() == PlatformErrorCode::Unsupported {
            log::info!(
                "cpu_temperature_unsupported model={model} stage={stage} code={:?} reason={diagnostic} retry_after_ms={retry_after_ms}",
                error.code()
            );
        } else {
            log::warn!(
                "cpu_temperature_read_failed model={model} stage={stage} code={:?} error={diagnostic} retry_after_ms={retry_after_ms}",
                error.code()
            );
        }
        self.previous_error = Some((stage, error.clone(), retry_after_ms));
    }

    pub(super) fn observe(&mut self, value: &CpuTemperature, requested_at: Instant) {
        let observed_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.observe_at(value, requested_at, observed_at_ms);
    }

    fn observe_at(&mut self, value: &CpuTemperature, requested_at: Instant, observed_at_ms: u64) {
        if self.previous_error.take().is_some() {
            log::info!(
                "cpu_temperature_recovered source={:?} kind={:?} sensor_count={}",
                value.source,
                value.kind,
                value.sensor_count
            );
        }
        let sampled_at_ms = value.sampled_at_ms.unwrap_or(observed_at_ms);
        let request_interval_ms = self.previous_request.map_or(0, |previous| {
            requested_at.duration_since(previous).as_millis() as u64
        });
        let previous_sampled_at_ms = self
            .previous_sample
            .filter(|(source, _)| *source == value.source)
            .map(|(_, sampled)| sampled);
        let cached = previous_sampled_at_ms == Some(sampled_at_ms);
        let sample_interval_ms =
            previous_sampled_at_ms.map_or(0, |previous| sampled_at_ms.saturating_sub(previous));
        let age_ms = observed_at_ms.saturating_sub(sampled_at_ms);
        if self.previous_sample.is_none() {
            log::info!("cpu_temperature_first_sample source={:?} sampled_at_ms={sampled_at_ms} observed_at_ms={observed_at_ms} age_ms={age_ms}", value.source);
        }
        self.requests += 1;
        self.max_request_interval_ms = self.max_request_interval_ms.max(request_interval_ms);
        self.max_age_ms = self.max_age_ms.max(age_ms);
        if cached {
            // One cache-hit detail per window suffices; the summary counts the rest.
            if self.cache_hits == 0 {
                log::info!("cpu_temperature_cache_hit source={:?} sampled_at_ms={sampled_at_ms} observed_at_ms={observed_at_ms} age_ms={age_ms} request_interval_ms={request_interval_ms}", value.source);
            }
            self.cache_hits += 1;
            self.cache_hits_since_sample += 1;
        } else {
            self.samples += 1;
            self.max_sample_interval_ms = self.max_sample_interval_ms.max(sample_interval_ms);
            if sample_interval_ms > TEMPERATURE_INTERVAL_MS * 2 {
                log::warn!("cpu_temperature_sample_gap source={:?} previous_sampled_at_ms={} sampled_at_ms={sampled_at_ms} observed_at_ms={observed_at_ms} sample_interval_ms={sample_interval_ms} request_interval_ms={request_interval_ms} age_ms={age_ms} cache_hits_since_sample={}", value.source, previous_sampled_at_ms.unwrap_or(0), self.cache_hits_since_sample);
            }
            self.cache_hits_since_sample = 0;
        }
        self.previous_sample = Some((value.source, sampled_at_ms));
        self.previous_request = Some(requested_at);
        let reported_at = self.reported_at.get_or_insert(requested_at);
        if requested_at.duration_since(*reported_at) >= SUMMARY_INTERVAL {
            log::info!("cpu_temperature_sampling_summary source={:?} window_ms={} requests={} native_samples={} cache_hits={} max_sample_interval_ms={} max_request_interval_ms={} max_age_ms={} sampled_at_ms={sampled_at_ms}", value.source, requested_at.duration_since(*reported_at).as_millis(), self.requests, self.samples, self.cache_hits, self.max_sample_interval_ms, self.max_request_interval_ms, self.max_age_ms);
            *reported_at = requested_at;
            self.requests = 0;
            self.samples = 0;
            self.cache_hits = 0;
            self.max_sample_interval_ms = 0;
            self.max_request_interval_ms = 0;
            self.max_age_ms = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_resources::cpu::temperature::CpuTemperatureKind;

    fn reading(sampled_at_ms: u64) -> CpuTemperature {
        CpuTemperature {
            celsius: 64.0,
            kind: CpuTemperatureKind::Package,
            source: CpuTemperatureSource::AmdZenPawnIo,
            sensor_count: 1,
            sampled_at_ms: Some(sampled_at_ms),
        }
    }

    #[test]
    fn cached_observations_do_not_advance_the_native_sample_clock() {
        let start = Instant::now();
        let mut diagnostics = Diagnostics::default();
        diagnostics.observe_at(&reading(1000), start, 1001);
        diagnostics.observe_at(&reading(1000), start + Duration::from_millis(4000), 5001);
        diagnostics.observe_at(&reading(9001), start + Duration::from_millis(8000), 9002);
        assert_eq!(diagnostics.requests, 3);
        assert_eq!(diagnostics.samples, 2);
        assert_eq!(diagnostics.cache_hits, 1);
        assert_eq!(diagnostics.max_sample_interval_ms, 8001);
        assert_eq!(diagnostics.max_request_interval_ms, 4000);
        assert_eq!(diagnostics.max_age_ms, 4001);
        assert_eq!(diagnostics.cache_hits_since_sample, 0);
    }

    #[test]
    fn summaries_keep_the_previous_sample_for_cross_window_gaps() {
        let start = Instant::now();
        let mut diagnostics = Diagnostics::default();
        diagnostics.observe_at(&reading(1000), start, 1001);
        diagnostics.observe_at(&reading(61000), start + Duration::from_secs(60), 61001);
        assert_eq!(diagnostics.requests, 0);
        diagnostics.observe_at(&reading(61000), start + Duration::from_secs(64), 65001);
        diagnostics.observe_at(&reading(69001), start + Duration::from_secs(68), 69002);
        assert_eq!(diagnostics.samples, 1);
        assert_eq!(diagnostics.cache_hits, 1);
        assert_eq!(diagnostics.max_sample_interval_ms, 8001);
    }

    #[test]
    #[ignore = "Run in an isolated process to own the global logger"]
    fn temperature_failure_logs_are_bounded_and_report_recovery() {
        struct Evidence(std::sync::Mutex<Vec<(log::Level, String)>>);
        impl log::Log for Evidence {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                self.0
                    .lock()
                    .unwrap()
                    .push((record.level(), record.args().to_string()));
            }
            fn flush(&self) {}
        }
        static LOG: Evidence = Evidence(std::sync::Mutex::new(Vec::new()));
        log::set_logger(&LOG).expect("isolated diagnostic process owns the logger");
        log::set_max_level(log::LevelFilter::Info);
        let mut diagnostics = Diagnostics::default();
        let unavailable =
            PlatformError::new(PlatformErrorCode::Unsupported, "sensor map unavailable");
        let absent =
            PlatformError::new(PlatformErrorCode::Unsupported, "identified sensors absent");
        let failed = PlatformError::new(PlatformErrorCode::OperationFailed, "SMC native_code=0x1");
        for error in [&unavailable, &absent, &failed] {
            for _ in 0..3 {
                diagnostics.observe_error(Some("Apple M7"), "initialize", error, 30_000);
            }
        }
        assert!(diagnostics.previous_sample.is_none());
        assert_eq!(diagnostics.requests, 0);
        diagnostics.observe_at(&reading(1000), Instant::now(), 1001);
        assert!(diagnostics.previous_error.is_none());
        diagnostics.observe_error(Some("Apple M7"), "initialize", &unavailable, 30_000);
        diagnostics.observe_error(Some("Apple M7"), "initialize", &unavailable, 30_000);
        diagnostics.observe_error(Some("Apple M7"), "read", &unavailable, 30_000);
        diagnostics.observe_error(Some("Apple M7"), "read", &unavailable, 30_000);
        let logs = LOG.0.lock().unwrap();
        let unsupported: Vec<_> = logs
            .iter()
            .filter(|(_, line)| line.starts_with("cpu_temperature_unsupported "))
            .collect();
        assert_eq!(unsupported.len(), 4);
        assert_eq!(
            unsupported
                .iter()
                .filter(|(_, line)| line.contains("stage=read "))
                .count(),
            1
        );
        for (level, line) in unsupported {
            assert_eq!(*level, log::Level::Info);
            assert!(line.contains("model=\"Apple M7\""));
            assert!(line.contains("code=Unsupported"));
            assert!(line.contains("reason="));
            assert!(line.contains("retry_after_ms={retry_after_ms}"));
        }
        assert_eq!(
            logs.iter()
                .filter(|(level, line)| *level == log::Level::Warn
                    && line.starts_with("cpu_temperature_read_failed "))
                .count(),
            1
        );
        assert_eq!(
            logs.iter()
                .filter(|(_, line)| line.starts_with("cpu_temperature_recovered "))
                .count(),
            1
        );
    }
}
