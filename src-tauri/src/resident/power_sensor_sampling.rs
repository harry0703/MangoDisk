//! CPU/system power and cooling sensors share a bounded owner independent of frequency.
use super::{
    sampling_reading::{NativeReading, Observed},
    sampling_schedule::Demand,
};
pub struct Observation {
    pub(super) fan_speed:
        Option<NativeReading<mangodisk_platform::system_resources::fans::FanSample>>,
    pub(super) system_power:
        Option<NativeReading<mangodisk_platform::system_resources::power::SystemPower>>,
    pub(super) cpu_power: Option<
        NativeReading<mangodisk_platform::system_resources::component_power::ComponentPower>,
    >,
}

#[derive(Default)]
pub struct Reader {
    cpu_power: mangodisk_platform::system_resources::component_power::ComponentPowerReader,
    power: mangodisk_platform::system_resources::power::SystemPowerReader,
    fans: Option<Box<mangodisk_platform::system_resources::fans::FanReader>>,
}

impl Reader {
    pub fn read(&mut self, demand: &Demand) -> Observation {
        let Self {
            power,
            cpu_power,
            fans,
        } = self;
        Observation {
            cpu_power: if demand.component_power {
                Observed::read(|| cpu_power.read()).into_optional()
            } else {
                *cpu_power = Default::default();
                None
            },
            fan_speed: if demand.fan_speed {
                Some(Observed::read(|| {
                    fans.get_or_insert_with(Box::default).read()
                }))
            } else {
                *fans = None;
                None
            },
            system_power: if demand.system_power {
                Some(Observed::read(|| power.read()))
            } else {
                *power = Default::default();
                None
            },
        }
    }
}

#[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
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
            power
                .read(&power_demand)
                .system_power
                .unwrap()
                .value
                .unwrap();
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
                    let watts = sample.system_power.unwrap().value.unwrap().watts;
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
