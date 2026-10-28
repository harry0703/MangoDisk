//! CPU details and auxiliary sensors have one bounded owner, independent of utilization.
use super::sampling_schedule::Demand;
use mangodisk_platform::system_resources::cpu::details::{CpuDetails, CpuDetailsReader};

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
            details,
        } = self;
        let details = details.read(demand.frequency, demand.temperature);
        // Read battery last so a slow CPU detail query cannot age its timestamp.
        let battery_present = battery_presence.read();
        let battery_sample = if demand.battery && battery_present != Some(false) {
            Some(battery.read())
        } else {
            *battery = Default::default();
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::time::Instant;

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
