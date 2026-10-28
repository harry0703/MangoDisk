//! Native display models shared by menu-bar, tray, taskbar, and panel adapters.
use mangodisk_core::system_resources::metrics::MetricId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayId {
    App,
    Cpu,
    CpuFrequency,
    CpuEfficiencyFrequency,
    CpuPerformanceFrequency,
    CpuSuperFrequency,
    CpuTemperature,
    CpuPower,
    Gpu,
    GpuTemperature,
    GpuPower,
    Memory,
    Upload,
    Download,
    Disk,
    DiskTemperature,
    BatteryTemperature,
    ChargingPower,
    BatteryPower,
    SystemPower,
    FanSpeed,
}

impl DisplayId {
    pub const ALL: [Self; 21] = [
        Self::App,
        Self::Cpu,
        Self::CpuFrequency,
        Self::CpuEfficiencyFrequency,
        Self::CpuPerformanceFrequency,
        Self::CpuSuperFrequency,
        Self::CpuTemperature,
        Self::CpuPower,
        Self::Gpu,
        Self::GpuTemperature,
        Self::GpuPower,
        Self::Memory,
        Self::Upload,
        Self::Download,
        Self::Disk,
        Self::DiskTemperature,
        Self::BatteryTemperature,
        Self::ChargingPower,
        Self::BatteryPower,
        Self::SystemPower,
        Self::FanSpeed,
    ];
    pub fn tray_id(self) -> &'static str {
        match self {
            Self::App => "resident",
            Self::Cpu => "resident-cpu",
            Self::CpuFrequency => "resident-cpu-frequency",
            Self::CpuEfficiencyFrequency => "resident-cpu-efficiency-frequency",
            Self::CpuPerformanceFrequency => "resident-cpu-performance-frequency",
            Self::CpuSuperFrequency => "resident-cpu-super-frequency",
            Self::CpuPower => "resident-cpu-power",
            Self::GpuPower => "resident-gpu-power",
            Self::CpuTemperature => "resident-cpu-temperature",
            Self::Gpu => "resident-gpu",
            Self::GpuTemperature => "resident-gpu-temperature",
            Self::Memory => "resident-memory",
            Self::Upload => "resident-upload",
            Self::Download => "resident-download",
            Self::Disk => "resident-disk",
            Self::DiskTemperature => "resident-disk-temperature",
            Self::BatteryTemperature => "resident-battery-temperature",
            Self::ChargingPower => "resident-charging-power",
            Self::BatteryPower => "resident-battery-power",
            Self::SystemPower => "resident-system-power",
            Self::FanSpeed => "resident-fan-speed",
        }
    }
    pub fn page(self) -> Option<crate::resident::panel_page::PanelPage> {
        use crate::resident::panel_page::PanelPage;
        match self {
            Self::BatteryTemperature
            | Self::ChargingPower
            | Self::BatteryPower
            | Self::SystemPower => Some(PanelPage::Power),
            _ => self.metric().map(PanelPage::from),
        }
    }
    pub fn metric(self) -> Option<MetricId> {
        match self {
            Self::App
            | Self::BatteryTemperature
            | Self::ChargingPower
            | Self::BatteryPower
            | Self::SystemPower
            | Self::FanSpeed => None,
            Self::Cpu
            | Self::CpuPower
            | Self::CpuTemperature
            | Self::CpuFrequency
            | Self::CpuEfficiencyFrequency
            | Self::CpuPerformanceFrequency
            | Self::CpuSuperFrequency => Some(MetricId::Cpu),
            Self::Gpu | Self::GpuPower | Self::GpuTemperature => Some(MetricId::Gpu),
            Self::Memory => Some(MetricId::Memory),
            Self::Upload | Self::Download => Some(MetricId::Network),
            Self::Disk | Self::DiskTemperature => Some(MetricId::Disk),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FanRow {
    pub id: u8,
    pub text: String,
    pub color_value: Option<u16>,
    pub tone: super::usage_color::UsageTone,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuDisplayDevice {
    pub id: String,
    pub name: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<FrequencyPresentation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_device: Option<GpuDisplayDevice>,
    pub id: DisplayId,
    pub tone: super::usage_color::UsageTone,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_state: Option<mangodisk_platform::system_resources::battery::BatteryPowerState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan_rows: Option<[FanRow; 2]>,
    pub color_value: Option<u16>,
    pub marker: String,
    pub digits: String,
    pub text: String,
    pub tooltip: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrequencyPresentation {
    pub idle_label: String,
    pub state: FrequencyDisplayState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrequencyDisplayState {
    Active,
    Idle,
    Unavailable,
}
