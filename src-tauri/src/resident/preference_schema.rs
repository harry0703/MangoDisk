//! Current preference models with strict validation and explicit release upgrades.
use mangodisk_core::system_resources::metrics::MetricId;
use serde::{Deserialize, Serialize};

mod migrations;
pub use migrations::decode;

pub(super) const PREFERENCE_SCHEMA_VERSION: u32 = 42;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowsDisplayMode {
    #[default]
    Taskbar,
    Floating,
    Both,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MacOsDisplayMode {
    #[default]
    MenuBar,
    Floating,
    Both,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinuxDisplayMode {
    #[default]
    Tray,
    Floating,
    Both,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FloatingLayout {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FloatingFontSize {
    #[default]
    Standard,
    Larger,
    Large,
    ExtraLarge,
}

impl FloatingFontSize {
    #[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
    pub(super) fn row_height(self) -> f64 {
        match self {
            Self::Standard => 20.0,
            Self::Larger => 23.0,
            Self::Large => 26.0,
            Self::ExtraLarge => 30.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FloatingBackground {
    #[default]
    Default,
    DarkGray,
    InkBlue,
    WarmWhite,
    LightGray,
    Black,
    White,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FloatingPreferences {
    pub layout: FloatingLayout,
    pub always_on_top: bool,
    pub locked: bool,
    pub background_opacity: u8,
    pub snap_to_edges: bool,
    pub font_size: FloatingFontSize,
    pub bold: bool,
    pub dividers: bool,
    pub click_through: bool,
    pub hide_on_fullscreen: bool,
    pub background: FloatingBackground,
}

impl Default for FloatingPreferences {
    fn default() -> Self {
        Self {
            layout: FloatingLayout::Horizontal,
            always_on_top: true,
            locked: false,
            background_opacity: 100,
            snap_to_edges: true,
            font_size: FloatingFontSize::Standard,
            bold: false,
            dividers: true,
            click_through: false,
            hide_on_fullscreen: true,
            background: FloatingBackground::Default,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskbarPosition {
    #[default]
    Auto,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FanDisplayMode {
    #[default]
    Highest,
    Average,
    Selected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum GpuDisplaySelection {
    #[default]
    Automatic,
    Selected {
        #[serde(rename = "adapterIds")]
        adapter_ids: Vec<String>,
    },
}

impl GpuDisplaySelection {
    fn valid(&self) -> bool {
        match self {
            Self::Selected { adapter_ids } => {
                !adapter_ids.is_empty()
                    && adapter_ids.len() <= 32
                    && adapter_ids
                        .iter()
                        .all(|id| !id.is_empty() && id.len() <= 1024 && !id.contains('\0'))
                    && adapter_ids
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == adapter_ids.len()
            }
            _ => true,
        }
    }
}

/// Display selection is independent of the five resource sampling workers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayMetricId {
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
    Disk,
    DiskTemperature,
    Network,
    BatteryTemperature,
    ChargingPower,
    BatteryPower,
    SystemPower,
    FanSpeed,
}

impl DisplayMetricId {
    pub fn metric(self) -> Option<MetricId> {
        match self {
            Self::Cpu => Some(MetricId::Cpu),
            Self::CpuFrequency
            | Self::CpuEfficiencyFrequency
            | Self::CpuPerformanceFrequency
            | Self::CpuSuperFrequency
            | Self::DiskTemperature
            | Self::CpuTemperature
            | Self::GpuTemperature
            | Self::BatteryTemperature
            | Self::ChargingPower
            | Self::BatteryPower
            | Self::SystemPower
            | Self::CpuPower
            | Self::GpuPower
            | Self::FanSpeed => None,
            Self::Gpu => Some(MetricId::Gpu),
            Self::Memory => Some(MetricId::Memory),
            Self::Disk => Some(MetricId::Disk),
            Self::Network => Some(MetricId::Network),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisplayMetric {
    pub id: DisplayMetricId,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicNetworkPreferences {
    pub enabled: bool,
}

impl Default for PublicNetworkPreferences {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResidentPreferences {
    pub schema_version: u32,
    pub revision: u64,
    pub windows_cpu_temperature_enabled: bool,
    pub enabled: bool,
    pub show_icon: bool,
    pub windows_display_mode: WindowsDisplayMode,
    pub macos_display_mode: MacOsDisplayMode,
    pub linux_display_mode: LinuxDisplayMode,
    pub floating: FloatingPreferences,
    pub taskbar_position: TaskbarPosition,
    pub taskbar_monitor: Option<String>,
    pub taskbar_background: bool,
    pub taskbar_compact: bool,
    pub menu_bar_compact: bool,
    pub usage_colors: bool,
    pub color_rules: Vec<super::color_rules::MetricColorRule>,
    pub metrics: Vec<DisplayMetric>,
    pub network_interface: Option<String>,
    pub public_network: PublicNetworkPreferences,
    pub disk_volume: Option<String>,
    pub disk_temperature_device: Option<String>,
    pub gpu_adapter: Option<String>,
    pub gpu_display: GpuDisplaySelection,
    pub gpu_temperature_display: GpuDisplaySelection,
    pub fan_display: FanDisplayMode,
    pub fan_ids: Vec<u8>,
}

const DISPLAY_ORDER: [DisplayMetricId; 19] = [
    DisplayMetricId::Cpu,
    DisplayMetricId::CpuFrequency,
    DisplayMetricId::CpuEfficiencyFrequency,
    DisplayMetricId::CpuPerformanceFrequency,
    DisplayMetricId::CpuSuperFrequency,
    DisplayMetricId::CpuTemperature,
    DisplayMetricId::CpuPower,
    DisplayMetricId::Gpu,
    DisplayMetricId::GpuTemperature,
    DisplayMetricId::GpuPower,
    #[cfg(windows)]
    DisplayMetricId::FanSpeed,
    DisplayMetricId::Memory,
    DisplayMetricId::Disk,
    DisplayMetricId::DiskTemperature,
    DisplayMetricId::BatteryTemperature,
    DisplayMetricId::ChargingPower,
    DisplayMetricId::BatteryPower,
    DisplayMetricId::SystemPower,
    #[cfg(not(windows))]
    DisplayMetricId::FanSpeed,
    DisplayMetricId::Network,
];

impl Default for ResidentPreferences {
    fn default() -> Self {
        Self {
            schema_version: PREFERENCE_SCHEMA_VERSION,
            revision: 0,
            windows_cpu_temperature_enabled: false,
            fan_display: FanDisplayMode::Highest,
            fan_ids: vec![0],
            enabled: true,
            show_icon: true,
            windows_display_mode: WindowsDisplayMode::default(),
            macos_display_mode: MacOsDisplayMode::default(),
            linux_display_mode: LinuxDisplayMode::default(),
            floating: FloatingPreferences::default(),
            taskbar_position: TaskbarPosition::default(),
            taskbar_monitor: None,
            taskbar_background: true,
            taskbar_compact: false,
            menu_bar_compact: false,
            usage_colors: true,
            color_rules: super::color_rules::defaults(),
            metrics: DISPLAY_ORDER
                .into_iter()
                .map(|id| DisplayMetric {
                    id,
                    enabled: matches!(
                        id,
                        DisplayMetricId::Cpu
                            | DisplayMetricId::Memory
                            | DisplayMetricId::Disk
                            | DisplayMetricId::Network
                    ),
                })
                .collect(),
            network_interface: None,
            public_network: PublicNetworkPreferences::default(),
            disk_volume: None,
            disk_temperature_device: None,
            gpu_adapter: None,
            gpu_display: GpuDisplaySelection::default(),
            gpu_temperature_display: GpuDisplaySelection::default(),
        }
    }
}

impl DisplayMetricId {
    pub(super) fn frequency_class(
        self,
    ) -> Option<mangodisk_platform::system_resources::cpu::details::CpuFrequencyClass> {
        use mangodisk_platform::system_resources::cpu::details::CpuFrequencyClass;
        match self {
            Self::CpuFrequency => Some(CpuFrequencyClass::Average),
            Self::CpuEfficiencyFrequency => Some(CpuFrequencyClass::Efficiency),
            Self::CpuPerformanceFrequency => Some(CpuFrequencyClass::Performance),
            Self::CpuSuperFrequency => Some(CpuFrequencyClass::Super),
            _ => None,
        }
    }
    pub fn supported_on_platform(self) -> bool {
        match self {
            Self::CpuPower | Self::GpuPower | Self::SystemPower => cfg!(target_os = "macos"),
            Self::CpuFrequency => cfg!(windows),
            Self::CpuEfficiencyFrequency
            | Self::CpuPerformanceFrequency
            | Self::CpuSuperFrequency => cfg!(all(target_os = "macos", target_arch = "aarch64")),
            Self::Gpu
            | Self::CpuTemperature
            | Self::GpuTemperature
            | Self::DiskTemperature
            | Self::BatteryTemperature
            | Self::ChargingPower
            | Self::BatteryPower
            | Self::FanSpeed => cfg!(any(target_os = "macos", windows)),
            Self::Cpu | Self::Memory | Self::Network | Self::Disk => true,
        }
    }
}

impl ResidentPreferences {
    pub fn shows(&self, id: MetricId) -> bool {
        self.metrics
            .iter()
            .any(|metric| metric.id.metric() == Some(id) && metric.enabled)
    }

    pub fn effective_icon(&self) -> bool {
        self.show_icon
            || (cfg!(windows) && self.windows_display_mode != WindowsDisplayMode::Taskbar)
            || (cfg!(target_os = "macos") && self.macos_display_mode != MacOsDisplayMode::MenuBar)
            || (cfg!(target_os = "linux") && self.linux_display_mode != LinuxDisplayMode::Tray)
            || !self.metrics.iter().any(|metric| {
                metric.enabled
                    && (metric.id != DisplayMetricId::CpuTemperature
                        || self.temperature_display_authorized())
            })
    }

    fn temperature_display_authorized(&self) -> bool {
        !cfg!(windows) || self.windows_cpu_temperature_enabled
    }

    pub fn shows_cpu_temperature(&self) -> bool {
        self.temperature_display_authorized() && self.shows_display(DisplayMetricId::CpuTemperature)
    }

    pub fn shows_disk_temperature(&self) -> bool {
        self.shows_display(DisplayMetricId::DiskTemperature)
    }

    pub fn shows_gpu_temperature(&self) -> bool {
        self.shows_display(DisplayMetricId::GpuTemperature)
    }

    pub fn shows_battery_item(&self, id: DisplayMetricId) -> bool {
        self.shows_display(id)
    }

    pub fn shows_fan_speed(&self) -> bool {
        self.shows_display(DisplayMetricId::FanSpeed)
    }

    pub fn shows_system_power(&self) -> bool {
        self.shows_display(DisplayMetricId::SystemPower)
    }

    pub(super) fn shows_cpu_frequency(&self) -> bool {
        self.metrics.iter().any(|item| {
            item.enabled && item.id.frequency_class().is_some() && item.id.supported_on_platform()
        })
    }

    pub(super) fn shows_display(&self, id: DisplayMetricId) -> bool {
        self.metrics
            .iter()
            .any(|metric| metric.id == id && metric.enabled)
    }

    pub fn shows_battery(&self) -> bool {
        self.shows_battery_item(DisplayMetricId::BatteryTemperature)
            || self.shows_battery_item(DisplayMetricId::ChargingPower)
            || self.shows_battery_item(DisplayMetricId::BatteryPower)
    }

    pub fn normalize(mut self) -> Result<Self, &'static str> {
        if self.schema_version != PREFERENCE_SCHEMA_VERSION {
            return Err("preferences_version");
        }
        if !(50..=100).contains(&self.floating.background_opacity)
            || !super::color_rules::valid(&self.color_rules)
            || self.metrics.len() != DISPLAY_ORDER.len()
            || self.fan_ids.is_empty()
            || self.fan_ids.len() > 2
            || self
                .fan_ids
                .iter()
                .any(|id| *id >= mangodisk_platform::system_resources::fans::MAX_FANS)
            || (self.fan_ids.len() == 2 && self.fan_ids[0] == self.fan_ids[1])
            || [
                &self.taskbar_monitor,
                &self.network_interface,
                &self.disk_volume,
                &self.disk_temperature_device,
                &self.gpu_adapter,
            ]
            .iter()
            .any(|id| {
                id.as_ref()
                    .is_some_and(|id| id.is_empty() || id.len() > 1024 || id.contains('\0'))
            })
        {
            return Err("preferences_invalid");
        }
        if !self.gpu_display.valid() || !self.gpu_temperature_display.valid() {
            return Err("preferences_invalid");
        }
        self.fan_ids.sort_unstable();
        for id in DISPLAY_ORDER {
            if self.metrics.iter().filter(|metric| metric.id == id).count() != 1 {
                return Err("preferences_invalid");
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
#[path = "preference_schema_tests.rs"]
mod tests;
