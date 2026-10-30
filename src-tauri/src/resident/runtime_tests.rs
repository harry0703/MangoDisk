use super::*;
use mangodisk_core::system_resources::metrics::MetricId;
use std::time::Duration;

#[test]
fn component_power_collects_on_intel_and_apple_silicon_with_panels_hidden() {
    use crate::resident::preference_schema::DisplayMetricId;
    for metric in [DisplayMetricId::CpuPower, DisplayMetricId::GpuPower] {
        assert_eq!(metric.supported_on_platform(), cfg!(target_os = "macos"));
    }
    let state = test_state();
    for panel_open in [false, true] {
        state.panel_open.store(panel_open, Ordering::Relaxed);
        for selected in MetricId::ALL {
            *state.panel_page.lock().unwrap() = selected.into();
            for (metric, demand) in MetricId::ALL.into_iter().zip(state.demands(false)) {
                assert_eq!(
                    demand.component_power,
                    cfg!(target_os = "macos") && matches!(metric, MetricId::Cpu | MetricId::Gpu)
                );
            }
        }
    }
    state.preferences.lock().unwrap().enabled = false;
    assert!(state
        .demands(false)
        .iter()
        .all(|demand| !demand.component_power));
}

#[test]
fn disk_temperature_only_collects_when_selected_and_resident_enabled() {
    let state = test_state();
    assert!(!state.demands(false)[4].temperature);
    *state.panel_page.lock().unwrap() = MetricId::Disk.into();
    assert_eq!(
        state.demands(false)[4].temperature,
        cfg!(any(target_os = "macos", windows))
    );
    state.panel_open.store(false, Ordering::Relaxed);
    assert!(!state.demands(false)[4].temperature);

    {
        let mut preferences = state.preferences.lock().unwrap();
        preferences.disk_volume = Some("chosen-volume".into());
        preferences
            .metrics
            .iter_mut()
            .find(|metric| {
                metric.id == crate::resident::preference_schema::DisplayMetricId::DiskTemperature
            })
            .unwrap()
            .enabled = true;
    }
    let demand = state.demands(false)[4].clone();
    assert_eq!(demand.temperature, cfg!(any(target_os = "macos", windows)));
    assert_eq!(demand.selection.as_deref(), Some("chosen-volume"));
    assert_eq!(MetricId::Disk.interval_ms(), 30_000);
    state.preferences.lock().unwrap().enabled = false;
    state.catalogue.store(2, Ordering::Relaxed);
    let catalogue = state.demands(false)[4].clone();
    assert!(catalogue.active);
    assert!(!catalogue.temperature);
    assert!(!catalogue.disk_health);
}

#[test]
fn disk_health_prepares_only_the_selected_disk_while_resident_is_enabled() {
    let state = test_state();
    state.panel_open.store(false, Ordering::Relaxed);
    let demands = state.demands(false);
    assert!(demands[4].disk_health);
    assert!(!demands[4].detailed);
    assert!(demands[..4].iter().all(|demand| !demand.disk_health));
    state.preferences.lock().unwrap().enabled = false;
    state.catalogue.store(2, Ordering::Relaxed);
    assert!(state
        .demands(false)
        .iter()
        .all(|demand| !demand.disk_health));
}

#[test]
fn battery_preparation_requires_enabled_resident_even_with_cpu_temperature_consent() {
    let state = test_state();
    let demands = state.demands(false);
    assert_eq!(
        demands[0].prepare_battery,
        cfg!(any(target_os = "macos", windows))
    );
    assert!(demands[1..].iter().all(|demand| !demand.prepare_battery));
    let mut preferences = state.preferences.lock().unwrap();
    preferences.enabled = false;
    preferences.windows_cpu_temperature_enabled = true;
    drop(preferences);
    let disabled = state.demands(false);
    assert!(!disabled[0].battery && !disabled[0].prepare_battery);
    #[cfg(windows)]
    assert!(disabled[0].active && disabled[0].temperature);
}

#[test]
fn temperature_history_demand_survives_hiding_and_switching_panels() {
    let state = test_state();
    state
        .preferences
        .lock()
        .unwrap()
        .windows_cpu_temperature_enabled = true;
    for panel_open in [false, true] {
        state.panel_open.store(panel_open, Ordering::Relaxed);
        for selected in MetricId::ALL {
            *state.panel_page.lock().unwrap() = selected.into();
            let cpu = state.demands(false)[0].clone();
            assert!(cpu.active);
            assert_eq!(cpu.temperature, cfg!(any(windows, target_os = "macos")));
            assert_eq!(cpu.detailed, panel_open && selected == MetricId::Cpu);
        }
    }
}

#[test]
fn macos_temperature_history_is_independent_of_display_selection() {
    let state = test_state();
    state.panel_open.store(false, Ordering::Relaxed);
    state
        .preferences
        .lock()
        .unwrap()
        .metrics
        .iter_mut()
        .find(|metric| {
            metric.id == crate::resident::preference_schema::DisplayMetricId::CpuTemperature
        })
        .unwrap()
        .enabled = true;
    let cpu = state.demands(false)[0].clone();
    assert!(cpu.active);
    assert!(!cpu.detailed);
    assert_eq!(cpu.temperature, cfg!(target_os = "macos"));
    *state.panel_page.lock().unwrap() = MetricId::Memory.into();
    state.panel_open.store(true, Ordering::Relaxed);
    assert_eq!(
        state.demands(false)[0].temperature,
        cfg!(target_os = "macos")
    );
    state
        .preferences
        .lock()
        .unwrap()
        .metrics
        .iter_mut()
        .find(|metric| {
            metric.id == crate::resident::preference_schema::DisplayMetricId::CpuTemperature
        })
        .unwrap()
        .enabled = false;
    assert_eq!(
        state.demands(false)[0].temperature,
        cfg!(target_os = "macos")
    );
    *state.panel_page.lock().unwrap() = MetricId::Cpu.into();
    assert_eq!(
        state.demands(false)[0].temperature,
        cfg!(target_os = "macos")
    );
    state.preferences.lock().unwrap().enabled = false;
    assert!(state
        .demands(false)
        .iter()
        .all(|demand| !demand.active && !demand.temperature));
}

#[test]
fn gpu_temperature_demand_survives_hidden_items_and_tab_changes() {
    let state = test_state();
    for panel_open in [true, false] {
        state.panel_open.store(panel_open, Ordering::Relaxed);
        for selected in MetricId::ALL {
            *state.panel_page.lock().unwrap() = selected.into();
            let gpu = state.demands(false)[1].clone();
            assert_eq!(gpu.temperature, cfg!(any(target_os = "macos", windows)));
            assert_eq!(gpu.active, cfg!(any(target_os = "macos", windows)));
        }
    }
    state.preferences.lock().unwrap().enabled = false;
    assert!(!state.demands(false)[1].temperature);
}

fn test_state() -> ResidentState {
    let (wake, _) = mpsc::sync_channel(1);
    let mut preferences = ResidentPreferences::default();
    for metric in &mut preferences.metrics {
        metric.enabled = false;
    }
    ResidentState {
        preference_update: Mutex::new(()),
        network_traffic: Mutex::new(None),
        preferences: Mutex::new(preferences),
        display_initialized: AtomicBool::new(true),
        panel_open: AtomicBool::new(true),
        panel_ready: AtomicBool::new(true),
        panel_creation: Mutex::new(()),
        panel_action: Mutex::new(()),
        panel_requested_at: Mutex::new(None),
        #[cfg(windows)]
        panel_hide_blurs: std::sync::atomic::AtomicU32::new(0),
        panel_page: Mutex::new(crate::resident::panel_page::PanelPage::Cpu),
        panel_source: Mutex::new(String::new()),
        reading: Mutex::new(ResidentReading {
            revision: 0,
            resources: ResourceReadings::default(),
        }),
        published_reading: Mutex::new(ResidentReading {
            revision: 0,
            resources: ResourceReadings::default(),
        }),
        catalogue: AtomicU8::new(0),
        process_refresh: AtomicBool::new(false),
        wake,
    }
}

#[test]
fn stalled_preference_update_does_not_block_sampling_demand() {
    let state = Arc::new(test_state());
    let update = state.preference_update.lock().unwrap();
    let sampler = state.clone();
    let (sampled, samples) = mpsc::channel();
    let (committed, commit) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        sampled
            .send((
                sampler.demands(false),
                crate::resident::sampling_policy::Task::DiskIo.demand(
                    &sampler.demands(false),
                    &mangodisk_core::system_resources::readings::ResourceCache::default(),
                ),
            ))
            .unwrap();
        commit.recv().unwrap();
        (
            sampler.demands(false),
            crate::resident::sampling_policy::Task::DiskIo.demand(
                &sampler.demands(false),
                &mangodisk_core::system_resources::readings::ResourceCache::default(),
            ),
        )
    });
    // Keep the same gate held by native application/save/rollback. The
    // sampler must finish before it is released, even with every entry off.
    // Release before asserting so a regression cannot strand a test thread.
    let before = samples.recv_timeout(Duration::from_secs(2));
    state.preferences.lock().unwrap().enabled = false;
    drop(update);
    committed.send(()).unwrap();
    let (after, disk_after) = thread.join().unwrap();
    let (before, disk_before) = before.expect("sampling waited for the preference transaction");
    for (metric, demand) in MetricId::ALL.into_iter().zip(before) {
        assert_eq!(
            demand.active,
            metric != MetricId::Gpu || cfg!(any(target_os = "macos", windows))
        );
    }
    assert!(disk_before.active);
    assert!(after.iter().all(|demand| !demand.active));
    assert!(!disk_after.active);
}

#[test]
fn process_detail_visibility_never_changes_overview_cadence() {
    let state = test_state();
    state
        .preferences
        .lock()
        .unwrap()
        .windows_cpu_temperature_enabled = true;
    for selected in MetricId::ALL {
        *state.panel_page.lock().unwrap() = selected.into();
        for (metric, demand) in MetricId::ALL.into_iter().zip(state.demands(false)) {
            assert_eq!(
                demand.active,
                metric != MetricId::Gpu || cfg!(any(target_os = "macos", windows))
            );
            assert_eq!(
                demand.temperature,
                (metric == MetricId::Cpu && cfg!(any(windows, target_os = "macos")))
                    || (metric == MetricId::Gpu && cfg!(any(target_os = "macos", windows)))
                    || (metric == MetricId::Disk
                        && selected == MetricId::Disk
                        && cfg!(any(target_os = "macos", windows)))
            );
            assert_eq!(
                demand.detailed,
                matches!(
                    metric,
                    MetricId::Cpu | MetricId::Memory | MetricId::Gpu | MetricId::Disk
                ) && metric == selected
            );
        }
        assert_eq!(state.cpu_processes_visible(), selected == MetricId::Cpu);
        assert_eq!(MetricId::Cpu.interval_ms(), 2000);
        assert_eq!(MetricId::Memory.interval_ms(), 3000);
    }
    state.panel_open.store(false, Ordering::Relaxed);
    assert!(!state.cpu_processes_visible());
    for (metric, demand) in MetricId::ALL.into_iter().zip(state.demands(false)) {
        assert_eq!(
            demand.active,
            metric != MetricId::Gpu || cfg!(any(target_os = "macos", windows))
        );
        assert!(!demand.detailed);
        assert_eq!(
            demand.temperature,
            (metric == MetricId::Cpu && cfg!(any(windows, target_os = "macos")))
                || (metric == MetricId::Gpu && cfg!(any(target_os = "macos", windows)))
        );
    }
    assert!(
        crate::resident::sampling_policy::Task::DiskIo
            .demand(
                &state.demands(false),
                &mangodisk_core::system_resources::readings::ResourceCache::default()
            )
            .active
    );
    state.preferences.lock().unwrap().enabled = false;
    assert!(
        !crate::resident::sampling_policy::Task::DiskIo
            .demand(
                &state.demands(false),
                &mangodisk_core::system_resources::readings::ResourceCache::default()
            )
            .active
    );
    for (metric, demand) in MetricId::ALL.into_iter().zip(state.demands(false)) {
        assert_eq!(demand.active, cfg!(windows) && metric == MetricId::Cpu);
        assert_eq!(demand.temperature, cfg!(windows) && metric == MetricId::Cpu);
        assert!(!demand.detailed);
    }
}

#[cfg(windows)]
#[test]
fn windows_temperature_requires_opt_in_for_panels_and_resident_display() {
    let state = test_state();
    state
        .preferences
        .lock()
        .unwrap()
        .metrics
        .iter_mut()
        .find(|row| row.id == crate::resident::preference_schema::DisplayMetricId::CpuTemperature)
        .unwrap()
        .enabled = true;
    for selected in MetricId::ALL {
        *state.panel_page.lock().unwrap() = selected.into();
        assert!(!state.demands(false)[0].temperature);
    }
    state
        .preferences
        .lock()
        .unwrap()
        .windows_cpu_temperature_enabled = true;
    state.panel_open.store(false, Ordering::Relaxed);
    state.preferences.lock().unwrap().enabled = false;
    state
        .preferences
        .lock()
        .unwrap()
        .metrics
        .iter_mut()
        .for_each(|row| row.enabled = false);
    let cpu = state.demands(false)[0].clone();
    assert!(cpu.active && cpu.temperature);
    assert!(!cpu.detailed);
    state
        .preferences
        .lock()
        .unwrap()
        .windows_cpu_temperature_enabled = false;
    assert!(!state.demands(false)[0].temperature);
}

#[cfg(not(any(target_os = "macos", windows)))]
#[test]
fn gpu_catalogue_can_be_requested_without_enabling_its_native_display() {
    let state = test_state();
    state.panel_open.store(false, Ordering::Relaxed);
    state.preferences.lock().unwrap().gpu_adapter = Some("gpu-fixed".into());
    state.request_catalogue();
    let gpu = state
        .demands(false)
        .into_iter()
        .zip(MetricId::ALL)
        .find(|(_, metric)| *metric == MetricId::Gpu)
        .unwrap()
        .0;
    assert!(gpu.active);
    assert_eq!(gpu.selection.as_deref(), Some("gpu-fixed"));
    assert!(!gpu.detailed);
    assert!(gpu.catalogue_only);
    state.catalogue.fetch_and(!4, Ordering::Relaxed);
    assert!(
        !state
            .demands(false)
            .into_iter()
            .zip(MetricId::ALL)
            .find(|(_, metric)| *metric == MetricId::Gpu)
            .unwrap()
            .0
            .active
    );
}
#[cfg(any(target_os = "macos", windows))]
#[test]
fn gpu_history_remains_active_across_hidden_panels_with_display_disabled() {
    let state = test_state();
    let gpu = MetricId::ALL
        .iter()
        .position(|metric| *metric == MetricId::Gpu)
        .unwrap();
    assert!(!state.preferences.lock().unwrap().shows(MetricId::Gpu));
    for panel_open in [true, false, true, false, true] {
        state.panel_open.store(panel_open, Ordering::Relaxed);
        *state.panel_page.lock().unwrap() = MetricId::Gpu.into();
        let demand = state.demands(false)[gpu].clone();
        assert!(demand.active, "hidden panels must retain real GPU history");
        assert!(!demand.catalogue_only);
        assert_eq!(demand.detailed, panel_open);
    }
    state.preferences.lock().unwrap().enabled = false;
    assert!(!state.demands(false)[gpu].active);
}

#[test]
fn gpu_background_demand_retains_history_until_global_disable() {
    let state = test_state();
    let gpu = MetricId::ALL
        .iter()
        .position(|metric| *metric == MetricId::Gpu)
        .unwrap();
    assert_eq!(
        state.demands(false)[gpu].active,
        cfg!(any(target_os = "macos", windows))
    );
    state.panel_open.store(false, Ordering::Relaxed);
    assert_eq!(
        state.demands(false)[gpu].active,
        cfg!(any(target_os = "macos", windows))
    );
    state
        .preferences
        .lock()
        .unwrap()
        .metrics
        .iter_mut()
        .find(|metric| metric.id.metric() == Some(MetricId::Gpu))
        .unwrap()
        .enabled = true;
    assert_eq!(
        state.demands(false)[gpu].active,
        cfg!(any(target_os = "macos", windows))
    );
    assert_eq!(MetricId::Gpu.interval_ms(), 2000);
    state.preferences.lock().unwrap().enabled = false;
    assert!(!state.demands(false)[gpu].active);
}

#[test]
fn fixed_disk_temperature_source_does_not_change_capacity_or_other_demands() {
    let state = test_state();
    let before = state.demands(false);
    state.preferences.lock().unwrap().disk_temperature_device = Some("physical-device".into());
    let after = state.demands(false);
    assert_eq!(&before[..4], &after[..4]);
    assert_eq!(after[4].selection, before[4].selection);
    assert_eq!(
        after[4].temperature_device.as_deref(),
        Some("physical-device")
    );
    assert_eq!(after[4].active, before[4].active);
    assert_eq!(after[4].temperature, before[4].temperature);
    assert_eq!(MetricId::Disk.interval_ms(), 30_000);
}

#[test]
fn display_catalogue_respects_optional_sensor_choices_and_platform_capabilities() {
    use crate::resident::preference_schema::DisplayMetricId;
    let state = test_state();
    state.panel_open.store(false, Ordering::Relaxed);
    let before = state.demands(false);
    assert!(!before[0].battery);
    assert_eq!(before[0].system_power, cfg!(target_os = "macos"));
    assert!(before.iter().all(|demand| !demand.fan_speed));
    {
        let mut preferences = state.preferences.lock().unwrap();
        for metric in &mut preferences.metrics {
            if matches!(
                metric.id,
                DisplayMetricId::BatteryTemperature
                    | DisplayMetricId::ChargingPower
                    | DisplayMetricId::BatteryPower
                    | DisplayMetricId::SystemPower
                    | DisplayMetricId::FanSpeed
            ) {
                metric.enabled = true;
            }
        }
    }
    let after = state.demands(false);
    assert_eq!(after[0].battery, cfg!(any(target_os = "macos", windows)));
    assert_eq!(after[0].system_power, cfg!(target_os = "macos"));
    assert_eq!(after[0].fan_speed, cfg!(target_os = "macos"));
    assert_eq!(after[1].fan_speed, cfg!(windows));
    for (previous, current) in before.iter().zip(&after) {
        assert_eq!(previous.active, current.active);
        assert_eq!(previous.temperature, current.temperature);
        assert_eq!(previous.detailed, current.detailed);
    }
    state.preferences.lock().unwrap().enabled = false;
    assert!(state
        .demands(false)
        .iter()
        .all(|demand| !demand.battery && !demand.system_power && !demand.fan_speed));
}

#[test]
fn system_power_history_survives_hidden_panels_and_all_page_selections() {
    use crate::resident::panel_page::PanelPage;
    let state = test_state();
    let before = state.preferences.lock().unwrap().clone();
    assert!(!before.shows_system_power());
    for panel_open in [false, true, false, true] {
        state.panel_open.store(panel_open, Ordering::Relaxed);
        for page in [
            PanelPage::Overview,
            PanelPage::Cpu,
            PanelPage::Gpu,
            PanelPage::Memory,
            PanelPage::Disk,
            PanelPage::Network,
            PanelPage::Power,
        ] {
            *state.panel_page.lock().unwrap() = page;
            let demands = state.demands(false);
            assert_eq!(demands[0].system_power, cfg!(target_os = "macos"));
            assert!(demands[1..].iter().all(|demand| !demand.system_power));
        }
    }
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(&*state.preferences.lock().unwrap()).unwrap()
    );
    state.preferences.lock().unwrap().enabled = false;
    assert!(state
        .demands(false)
        .iter()
        .all(|demand| !demand.system_power));
}

#[test]
fn power_page_releases_battery_details_but_keeps_system_power_when_hidden() {
    use crate::resident::panel_page::PanelPage;
    let state = test_state();
    let before = state.preferences.lock().unwrap().clone();
    assert!(!before.shows_battery());
    assert!(!before.shows_system_power());
    *state.panel_page.lock().unwrap() = PanelPage::Power;
    state.panel_open.store(true, Ordering::Relaxed);
    let demand = state.demands(false);
    assert_eq!(demand[0].battery, cfg!(any(target_os = "macos", windows)));
    assert_eq!(demand[0].system_power, cfg!(target_os = "macos"));
    assert!(!demand[0].detailed);
    assert!(!state.cpu_processes_visible());
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(&*state.preferences.lock().unwrap()).unwrap()
    );
    state.panel_open.store(false, Ordering::Relaxed);
    let hidden = state.demands(false);
    assert!(!hidden[0].battery);
    assert_eq!(hidden[0].system_power, cfg!(target_os = "macos"));
    state.preferences.lock().unwrap().enabled = false;
    state.panel_open.store(true, Ordering::Relaxed);
    let disabled = state.demands(false);
    assert!(!disabled[0].battery && !disabled[0].system_power);
}

#[test]
fn floating_display_reuses_overview_demand_without_enabling_details() {
    use crate::resident::preference_schema::WindowsDisplayMode;
    let state = test_state();
    state.panel_open.store(false, Ordering::Relaxed);
    let baseline = state.demands(false);
    state.preferences.lock().unwrap().windows_display_mode = WindowsDisplayMode::Floating;
    assert_eq!(state.demands(false), baseline);
    assert!(state.demands(false).iter().all(|demand| !demand.detailed));
    state.panel_open.store(true, Ordering::Relaxed);
    *state.panel_page.lock().unwrap() = MetricId::Memory.into();
    assert!(state.demands(false)[2].detailed);
}

#[test]
fn frequency_display_keeps_sampling_without_enabling_cpu_process_details() {
    use crate::resident::preference_schema::DisplayMetricId;
    let state = test_state();
    state.panel_open.store(false, Ordering::Relaxed);
    assert!(!state.demands(false)[0].frequency);
    for id in [
        DisplayMetricId::CpuFrequency,
        DisplayMetricId::CpuEfficiencyFrequency,
        DisplayMetricId::CpuPerformanceFrequency,
        DisplayMetricId::CpuSuperFrequency,
    ] {
        {
            let mut preferences = state.preferences.lock().unwrap();
            for item in &mut preferences.metrics {
                if item.id.frequency_class().is_some() {
                    item.enabled = item.id == id;
                }
            }
        }
        let demand = state.demands(false)[0].clone();
        assert_eq!(demand.frequency, id.supported_on_platform());
        assert!(!demand.detailed);
        assert!(!state.cpu_processes_visible());
        state.panel_open.store(true, Ordering::Relaxed);
        *state.panel_page.lock().unwrap() = MetricId::Cpu.into();
        assert!(state.demands(false)[0].frequency);
        assert!(state.cpu_processes_visible());
        state.panel_open.store(false, Ordering::Relaxed);
        assert_eq!(
            state.demands(false)[0].frequency,
            id.supported_on_platform()
        );
    }
    state.preferences.lock().unwrap().enabled = false;
    assert!(!state.demands(false)[0].frequency);
}
