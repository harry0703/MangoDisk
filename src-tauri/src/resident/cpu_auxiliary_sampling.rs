//! CPU details and auxiliary sensors have one bounded owner, independent of utilization.
use super::sampling_schedule::Demand;
use mangodisk_platform::system_resources::cpu::details::{CpuDetails, CpuDetailsReader};
use std::time::{Duration, Instant};

fn battery_due(requested: bool, present: Option<bool>, elapsed: Option<Duration>) -> bool {
    present != Some(false)
        && (requested || elapsed.is_none_or(|age| age >= Duration::from_secs(30)))
}

pub struct Observation {
    pub(super) battery_present: Option<bool>,
    pub(super) fan_speed: Option<
        mangodisk_platform::PlatformResult<mangodisk_platform::system_resources::fans::FanSample>,
    >,
    pub(super) system_power: Option<
        mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::power::SystemPower,
        >,
    >,
    pub(super) battery: Option<
        mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::battery::BatterySample,
        >,
    >,
    pub(super) cpu_power: Option<
        mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::component_power::ComponentPower,
        >,
    >,
    pub(super) details: CpuDetails,
}

#[derive(Default)]
pub struct Reader {
    battery_presence: mangodisk_platform::system_resources::battery::BatteryPresenceReader,
    cpu_power: mangodisk_platform::system_resources::component_power::ComponentPowerReader,
    power: mangodisk_platform::system_resources::power::SystemPowerReader,
    fans: Option<Box<mangodisk_platform::system_resources::fans::FanReader>>,
    battery: mangodisk_platform::system_resources::battery::BatteryReader,
    battery_attempted_at: Option<Instant>,
    details: Box<CpuDetailsReader>,
}

impl Reader {
    pub fn read(&mut self, demand: &Demand) -> Observation {
        let Self {
            battery_presence,
            power,
            cpu_power,
            fans,
            battery,
            battery_attempted_at,
            details,
        } = self;
        let details = details.read(demand.frequency, demand.temperature);
        // Read battery last so a slow CPU detail query cannot age its timestamp.
        let battery_present = battery_presence.read();
        // Prepare a real observation before the first panel visit. Hidden panels
        // only need a low-frequency snapshot; visible battery metrics stay live.
        let battery_sample = if cfg!(any(target_os = "macos", windows))
            && (demand.battery || demand.prepare_battery)
            && battery_due(
                demand.battery,
                battery_present,
                battery_attempted_at.map(|at| at.elapsed()),
            ) {
            *battery_attempted_at = Some(Instant::now());
            Some(battery.read())
        } else {
            if battery_present == Some(false) || (!demand.battery && !demand.prepare_battery) {
                *battery = Default::default();
                *battery_attempted_at = None;
            }
            None
        };
        Observation {
            cpu_power: if demand.component_power {
                cpu_power.read()
            } else {
                *cpu_power = Default::default();
                None
            },
            battery_present,
            fan_speed: if demand.fan_speed {
                Some(fans.get_or_insert_with(Box::default).read())
            } else {
                *fans = None;
                None
            },
            system_power: if demand.system_power {
                Some(power.read())
            } else {
                *power = Default::default();
                None
            },
            battery: battery_sample,
            details,
        }
    }
}

#[cfg(test)]
mod cadence_tests {
    use super::*;

    #[test]
    fn hidden_battery_preparation_is_bounded_and_visible_requests_are_immediate() {
        assert!(battery_due(false, None, None));
        assert!(battery_due(false, Some(true), None));
        assert!(!battery_due(
            false,
            Some(true),
            Some(Duration::from_secs(29))
        ));
        assert!(battery_due(
            false,
            Some(true),
            Some(Duration::from_secs(30))
        ));
        assert!(battery_due(
            true,
            Some(true),
            Some(Duration::from_millis(1))
        ));
        assert!(!battery_due(true, Some(false), None));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore = "requires a native battery; reproduces CPU-only background demand"]
    fn native_cpu_only_demand_must_not_read_battery() {
        let mut reader = Reader::default();
        let observation = reader.read(&Demand {
            active: true,
            temperature: true,
            ..Default::default()
        });
        assert_eq!(observation.battery_present, Some(true));
        assert!(
            observation.battery.is_none(),
            "CPU-only demand must not prepare battery data"
        );
    }

    #[test]
    #[ignore = "requires a native battery; measures warm-up and hidden sampling cost"]
    fn native_battery_prepares_before_panel_and_reuses_idle_reader() {
        let mut reader = Reader::default();
        let idle = Demand {
            active: true,
            prepare_battery: true,
            ..Default::default()
        };
        let started = Instant::now();
        let first = reader.read(&idle);
        let cold_us = started.elapsed().as_micros();
        assert_eq!(first.battery_present, Some(true));
        assert!(first.battery.unwrap().unwrap().details.is_some());
        let mut costs = Vec::new();
        for _ in 0..200 {
            let started = Instant::now();
            assert!(reader.read(&idle).battery.is_none());
            costs.push(started.elapsed().as_micros());
        }
        costs.sort_unstable();
        let started = Instant::now();
        let live = reader.read(&Demand {
            battery: true,
            ..idle
        });
        assert!(live.battery.unwrap().unwrap().details.is_some());
        eprintln!("native_battery_readiness cold_us={cold_us} hidden_p50_us={} hidden_p95_us={} live_us={} hidden_queries=0 hidden_polls=200", costs[100], costs[190], started.elapsed().as_micros());
    }

    #[test]
    #[ignore = "requires native Apple SMC PSTR; reports incremental sampling cost"]
    fn native_system_power_incremental_cost() {
        let baseline_demand = Demand {
            active: true,
            temperature: true,
            ..Default::default()
        };
        let power_demand = Demand {
            system_power: true,
            ..baseline_demand.clone()
        };
        let mut baseline = Reader::default();
        let mut power = Reader::default();
        for _ in 0..10 {
            baseline.read(&baseline_demand);
            power.read(&power_demand).system_power.unwrap().unwrap();
        }
        let mut baseline_us = Vec::new();
        let mut power_us = Vec::new();
        for round in 0..200 {
            // Alternate order to reduce thermal and scheduler bias between cases.
            for with_power in [round % 2 == 0, round % 2 != 0] {
                let started = Instant::now();
                if with_power {
                    let sample = power.read(&power_demand);
                    power_us.push(started.elapsed().as_micros());
                    let watts = sample.system_power.unwrap().unwrap().watts;
                    assert!(watts.is_finite() && watts >= 0.0);
                } else {
                    let sample = baseline.read(&baseline_demand);
                    baseline_us.push(started.elapsed().as_micros());
                    assert!(sample.system_power.is_none());
                }
            }
        }
        for (name, mut samples) in [("baseline", baseline_us), ("power", power_us)] {
            let total: u128 = samples.iter().sum();
            samples.sort_unstable();
            eprintln!(
                "native_auxiliary_cost case={name} samples={} mean_us={} p50_us={} p95_us={} max_us={}",
                samples.len(),
                total / samples.len() as u128,
                samples[samples.len() / 2],
                samples[samples.len() * 95 / 100],
                samples.last().unwrap()
            );
        }
    }
}
