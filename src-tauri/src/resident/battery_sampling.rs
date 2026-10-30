//! Battery preparation and live readings have one independent, bounded native owner.
use super::{sampling_reading::Observed, sampling_schedule::Demand};
use mangodisk_platform::system_resources::battery::{
    BatteryPresenceReader, BatteryReader, BatterySample,
};

#[derive(Default)]
pub struct Reader {
    presence: BatteryPresenceReader,
    battery: BatteryReader,
}
pub struct Observation {
    pub present: Observed<Option<bool>>,
    pub sample: Option<Observed<mangodisk_platform::PlatformResult<BatterySample>>>,
}
impl Reader {
    pub fn read(&mut self, demand: &Demand) -> Observation {
        let present = Observed::read(|| self.presence.read());
        let sample = if (demand.battery || demand.prepare_battery) && present.value != Some(false) {
            Some(Observed::read(|| self.battery.read()))
        } else {
            self.battery = Default::default();
            None
        };
        Observation { present, sample }
    }
}
