//! Desktop demand policy derived from committed preferences and panel visibility.
use super::ResidentState;
use crate::resident::panel_page::PanelPage;
use crate::resident::{preference_schema::DisplayMetricId, sampling_schedule::Demand};
use mangodisk_core::system_resources::metrics::MetricId;
use std::sync::atomic::Ordering;

impl ResidentState {
    pub(super) fn cpu_processes_visible(&self) -> bool {
        self.panel_open.load(Ordering::Relaxed)
            && *self.panel_page.lock().unwrap_or_else(|e| e.into_inner()) == PanelPage::Cpu
    }

    pub(super) fn demands(&self, warm_icons: bool) -> [Demand; 5] {
        let preferences = self
            .preferences
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let panel_open = self.panel_open.load(Ordering::Relaxed);
        let page = *self
            .panel_page
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let selected = page.metric();
        let power_page = panel_open && page == PanelPage::Power;
        let catalogue = self.catalogue.load(Ordering::Relaxed);
        // Keep usage and temperature histories continuous while resident monitoring
        // is enabled. Display choices and panel visibility only affect presentation.
        let gpu_sampling = preferences.enabled
            && (cfg!(any(target_os = "macos", windows))
                || (DisplayMetricId::Gpu.supported_on_platform()
                    && (preferences.shows(MetricId::Gpu) || panel_open)));
        // Display selection must not interrupt real temperature history. Windows
        // requires explicit capability consent even when resident display is hidden.
        let temperature_sampling = (cfg!(windows) && preferences.windows_cpu_temperature_enabled)
            || (cfg!(target_os = "macos") && preferences.enabled);
        MetricId::ALL.map(|metric| Demand {
            frequency: metric == MetricId::Cpu
                && preferences.enabled
                && (preferences.shows_cpu_frequency()
                    || (panel_open && selected == Some(MetricId::Cpu))),
            fan_speed: ((cfg!(target_os = "macos") && metric == MetricId::Cpu)
                || (cfg!(windows) && metric == MetricId::Gpu))
                && preferences.enabled
                && preferences.shows_fan_speed(),
            // Keep the minute of power history independent of popup lifetime.
            // The existing auxiliary worker retains its two-second cadence.
            system_power: DisplayMetricId::SystemPower.supported_on_platform()
                && metric == MetricId::Cpu
                && preferences.enabled,
            component_power: cfg!(target_os = "macos")
                && preferences.enabled
                && matches!(metric, MetricId::Cpu | MetricId::Gpu),
            battery: DisplayMetricId::BatteryPower.supported_on_platform()
                && metric == MetricId::Cpu
                && preferences.enabled
                && (preferences.shows_battery() || power_page),
            prepare_battery: DisplayMetricId::BatteryPower.supported_on_platform()
                && metric == MetricId::Cpu
                && preferences.enabled,
            disk_health: preferences.enabled && metric == MetricId::Disk,
            disk_io: preferences.enabled && metric == MetricId::Disk,
            temperature: (preferences.enabled
                && metric == MetricId::Disk
                && DisplayMetricId::DiskTemperature.supported_on_platform()
                && (preferences.shows_disk_temperature()
                    || (cfg!(any(target_os = "macos", windows))
                        && panel_open
                        && selected == Some(MetricId::Disk))))
                || (temperature_sampling && metric == MetricId::Cpu)
                || (gpu_sampling
                    && cfg!(any(target_os = "macos", windows))
                    && metric == MetricId::Gpu),
            active: (preferences.enabled && metric != MetricId::Gpu)
                || (metric == MetricId::Cpu && temperature_sampling)
                || (metric == MetricId::Gpu && gpu_sampling)
                || (metric == MetricId::Network && catalogue & 1 != 0)
                || (metric == MetricId::Disk && catalogue & 2 != 0)
                || (metric == MetricId::Gpu && catalogue & 4 != 0),
            detailed: preferences.enabled
                && ((metric == MetricId::Memory
                    && ((panel_open && selected == Some(metric)) || warm_icons))
                    || (matches!(metric, MetricId::Cpu | MetricId::Gpu | MetricId::Disk)
                        && panel_open
                        && selected == Some(metric))),
            catalogue_only: metric == MetricId::Gpu && !gpu_sampling,
            temperature_device: (metric == MetricId::Disk)
                .then(|| preferences.disk_temperature_device.clone())
                .flatten(),
            selection: match metric {
                MetricId::Network => preferences.network_interface.clone(),
                MetricId::Disk => preferences.disk_volume.clone(),
                MetricId::Gpu => preferences.gpu_adapter.clone(),
                _ => None,
            },
            volumes: None,
        })
    }
}
