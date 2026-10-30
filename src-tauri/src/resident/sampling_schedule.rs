//! Deterministic demand and in-flight accounting shared by every metric worker.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Demand {
    pub active: bool,
    pub detailed: bool,
    pub frequency: bool,
    pub temperature: bool,
    pub battery: bool,
    pub prepare_battery: bool,
    pub disk_health: bool,
    pub disk_io: bool,
    pub system_power: bool,
    pub component_power: bool,
    pub fan_speed: bool,
    pub catalogue_only: bool,
    pub selection: Option<String>,
    pub temperature_device: Option<String>,
    pub volumes:
        Option<std::sync::Arc<[mangodisk_platform::system_resources::disk::ResourceVolume]>>,
}

#[derive(Debug, Default)]
pub struct SamplingSlot {
    pub demand: Demand,
    generation: u64,
    baseline_generation: u64,
    in_flight: Option<u64>,
    due_ms: u64,
}

impl SamplingSlot {
    pub fn update(&mut self, demand: Demand) -> bool {
        if self.demand == demand {
            return false;
        }
        // Detail visibility changes result ownership, not the native counter source.
        if self.demand.active != demand.active
            || self.demand.selection != demand.selection
            || self.demand.catalogue_only != demand.catalogue_only
        {
            self.baseline_generation = self.baseline_generation.wrapping_add(1);
        }
        self.demand = demand;
        self.generation = self.generation.wrapping_add(1);
        self.due_ms = 0;
        // Keep the old job in flight until it actually exits. A timeout or
        // changed device must not spawn more uncancellable native queries.
        true
    }

    pub fn resume(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.baseline_generation = self.baseline_generation.wrapping_add(1);
        self.due_ms = 0;
        // The old native call still owns its in-flight slot until it exits.
    }

    /// Only a confirmed exited thread can relinquish an unfinished native query.
    pub fn retire(&mut self) {
        self.in_flight = None;
        self.resume();
    }

    pub fn baseline_generation(&self) -> u64 {
        self.baseline_generation
    }

    pub fn begin(&mut self, now_ms: u64, interval_ms: u64) -> Option<u64> {
        if !self.demand.active || self.in_flight.is_some() || now_ms < self.due_ms {
            return None;
        }
        self.in_flight = Some(self.generation);
        self.due_ms = now_ms.saturating_add(interval_ms);
        Some(self.generation)
    }

    /// Bring forward a bounded recovery attempt after a baseline-only sample.
    pub fn retry_after(&mut self, now_ms: u64, delay_ms: u64) {
        self.due_ms = self.due_ms.min(now_ms.saturating_add(delay_ms));
    }

    pub fn postpone_after(&mut self, now_ms: u64, delay_ms: u64) {
        self.due_ms = self.due_ms.max(now_ms.saturating_add(delay_ms));
    }

    pub fn request_now(&mut self) {
        // An in-flight observation already satisfies the request. Do not queue
        // another expensive query behind it when users reopen or switch tabs.
        if self.in_flight.is_none() {
            self.due_ms = 0;
        }
    }

    /// Sleep until the next due query, with a one-second freshness/appearance check.
    pub fn wait_ms(&self, now_ms: u64) -> u64 {
        if self.demand.active && self.in_flight.is_none() {
            self.due_ms.saturating_sub(now_ms).clamp(1, 1000)
        } else {
            1000
        }
    }

    pub fn complete(&mut self, generation: u64) -> bool {
        if self.in_flight != Some(generation) {
            return false;
        }
        self.in_flight = None;
        self.demand.active && generation == self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_and_sensor_choices_keep_the_counter_baseline_but_reject_old_results() {
        let mut slot = SamplingSlot::default();
        slot.update(Demand {
            active: true,
            ..Default::default()
        });
        let baseline = slot.baseline_generation();
        let in_flight = slot.begin(1000, 2000).unwrap();
        slot.update(Demand {
            active: true,
            detailed: true,
            temperature: true,
            fan_speed: true,
            ..Default::default()
        });
        assert_eq!(slot.baseline_generation(), baseline);
        assert!(slot.begin(1001, 2000).is_none());
        assert!(
            !slot.complete(in_flight),
            "old detail results cannot replace the new demand"
        );
        assert!(slot.begin(1001, 2000).is_some());
        slot.update(Demand {
            active: true,
            selection: Some("second-adapter".into()),
            ..Default::default()
        });
        assert_ne!(slot.baseline_generation(), baseline);
    }

    #[test]
    fn disable_and_reenable_always_prime_a_new_counter_source() {
        let mut slot = SamplingSlot::default();
        slot.update(Demand {
            active: true,
            ..Default::default()
        });
        let first = slot.baseline_generation();
        slot.update(Demand::default());
        assert_ne!(slot.baseline_generation(), first);
        let stopped = slot.baseline_generation();
        slot.update(Demand {
            active: true,
            ..Default::default()
        });
        assert_ne!(slot.baseline_generation(), stopped);
    }

    #[test]
    fn opening_details_samples_now_without_queuing_behind_in_flight_work() {
        let mut slot = SamplingSlot::default();
        slot.update(Demand {
            active: true,
            ..Default::default()
        });
        let first = slot.begin(0, 4000).unwrap();
        slot.request_now();
        assert!(slot.begin(500, 2000).is_none());
        assert!(slot.complete(first));
        assert!(slot.begin(500, 4000).is_none());
        slot.request_now();
        let opened = slot.begin(500, 2000).unwrap();
        assert!(slot.complete(opened));
        assert!(slot.begin(2499, 2000).is_none());
        assert!(slot.begin(2500, 2000).is_some());
    }

    #[test]
    fn baseline_retry_waits_for_its_delay_and_never_overlaps_a_job() {
        let mut slot = SamplingSlot::default();
        slot.update(Demand {
            active: true,
            ..Default::default()
        });
        let token = slot.begin(0, 2000).unwrap();
        slot.retry_after(0, 250);
        assert!(slot.begin(250, 2000).is_none());
        assert!(slot.complete(token));
        assert!(slot.begin(249, 2000).is_none());
        assert!(slot.begin(250, 2000).is_some());
        slot.update(Demand::default());
        assert!(slot.begin(5000, 2000).is_none());
    }

    #[test]
    fn disabling_drops_late_results_without_overlapping_a_slow_query() {
        let mut slot = SamplingSlot::default();
        slot.update(Demand {
            active: true,
            ..Default::default()
        });
        let token = slot.begin(0, 1000).unwrap();
        assert!(slot.begin(60_000, 1000).is_none());
        slot.update(Demand::default());
        assert!(!slot.complete(token));
        assert!(slot.begin(60_000, 1000).is_none());
        slot.update(Demand {
            active: true,
            ..Default::default()
        });
        assert!(slot.begin(60_000, 1000).is_some());
    }

    #[test]
    fn changed_device_waits_for_previous_job_and_repeated_wakes_preserve_cadence() {
        let mut slot = SamplingSlot::default();
        slot.update(Demand {
            active: true,
            selection: Some("one".into()),
            ..Default::default()
        });
        let first = slot.begin(0, 1000).unwrap();
        slot.update(Demand {
            active: true,
            selection: Some("two".into()),
            ..Default::default()
        });
        assert!(slot.begin(1000, 1000).is_none());
        assert!(!slot.complete(first));
        let second = slot.begin(1000, 1000).unwrap();
        assert!(slot.complete(second));
        for time in 1000..2000 {
            assert!(slot.begin(time, 1000).is_none());
        }
        assert!(slot.begin(2000, 1000).is_some());
    }
    #[test]
    fn changing_temperature_device_rejects_old_results_without_resetting_capacity_baseline() {
        let mut slot = SamplingSlot::default();
        slot.update(Demand {
            active: true,
            ..Default::default()
        });
        let baseline = slot.baseline_generation();
        let in_flight = slot.begin(1000, 30_000).unwrap();
        slot.update(Demand {
            active: true,
            temperature_device: Some("other-disk".into()),
            ..Default::default()
        });
        assert_eq!(slot.baseline_generation(), baseline);
        assert!(slot.begin(1001, 30_000).is_none());
        assert!(!slot.complete(in_flight));
        assert!(slot.begin(1002, 30_000).is_some());
    }
}
