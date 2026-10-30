mod coordinator;
mod demand;
mod observation;

use mangodisk_core::system_resources::readings::ResourceReadings;
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    time::Instant,
};
use tauri::{Listener, Manager};

use super::{preferences::ResidentPreferences, sampling_workers::SamplingEvent, TRAY_ID};

pub const READING_EVENT: &str = "resident-reading";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidentReading {
    pub revision: u64,
    #[serde(flatten)]
    pub resources: ResourceReadings,
}

pub struct ResidentState {
    // Serialize native preference transactions and periodic display refreshes.
    // Sampling only reads the short-lived committed-preferences lock below.
    pub preference_update: Mutex<()>,
    pub network_traffic:
        Mutex<Option<Arc<mangodisk_core::system_resources::network_traffic::NetworkTraffic>>>,
    pub preferences: Mutex<ResidentPreferences>,
    pub display_initialized: AtomicBool,
    pub panel_open: AtomicBool,
    pub panel_ready: AtomicBool,
    pub panel_creation: Mutex<()>,
    pub panel_action: Mutex<()>,
    pub panel_requested_at: Mutex<Option<Instant>>,
    #[cfg(windows)]
    pub panel_hide_blurs: std::sync::atomic::AtomicU32,
    pub panel_page: Mutex<super::panel_page::PanelPage>,
    pub panel_source: Mutex<String>,
    pub reading: Mutex<ResidentReading>,
    pub published_reading: Mutex<ResidentReading>,
    catalogue: AtomicU8,
    process_refresh: AtomicBool,
    wake: SyncSender<SamplingEvent>,
}

impl ResidentState {
    pub fn enabled(&self) -> bool {
        self.preferences
            .lock()
            .map(|value| value.enabled)
            .unwrap_or(false)
    }

    pub fn wake(&self) {
        let _ = self.wake.try_send(SamplingEvent::Wake);
    }

    pub fn request_catalogue(&self) {
        self.catalogue.store(7, Ordering::Relaxed);
        self.wake();
    }

    pub fn refresh_application_processes(&self) {
        self.process_refresh.store(true, Ordering::Relaxed);
        self.wake();
    }
}

pub fn start(app: &tauri::AppHandle, preferences: ResidentPreferences) -> Arc<ResidentState> {
    let (sender, events) = mpsc::sync_channel(8);
    let state = Arc::new(ResidentState {
        preference_update: Mutex::new(()),
        network_traffic: Mutex::new(None),
        preferences: Mutex::new(preferences),
        display_initialized: AtomicBool::new(false),
        panel_open: AtomicBool::new(false),
        panel_ready: AtomicBool::new(false),
        panel_creation: Mutex::new(()),
        panel_action: Mutex::new(()),
        panel_requested_at: Mutex::new(None),
        #[cfg(windows)]
        panel_hide_blurs: std::sync::atomic::AtomicU32::new(0),
        panel_page: Mutex::new(super::panel_page::PanelPage::Cpu),
        panel_source: Mutex::new(TRAY_ID.into()),
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
        wake: sender.clone(),
    });
    // Publish state before any worker can call a desktop adapter that looks it up.
    app.manage(state.clone());
    let locale_state = state.clone();
    // Store notifications immediately refresh native labels after locale changes.
    // The listener only queues work: reading the store inside its own mutation
    // callback would contend with the plugin's cache lock.
    app.listen("store://change", move |event| {
        if serde_json::from_str::<serde_json::Value>(event.payload())
            .ok()
            .is_some_and(|value| value.get("key").and_then(|key| key.as_str()) == Some("settings"))
        {
            locale_state.wake();
        }
    });
    let worker = state.clone();
    let app = app.clone();
    std::thread::spawn(move || coordinator::run(app, worker, sender, events));
    state
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod overview_tests;
