//! On-demand public egress discovery and memory-only cache.
use mangodisk_core::system_resources::network_addresses::{
    self, AddressError, AddressFamily, NetworkAddress,
};
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use tokio::time::Instant;

const EVENT: &str = "resident-public-network";
const CACHE_AGE: Duration = Duration::from_secs(3600);
const FAMILIES: [AddressFamily; 2] = [AddressFamily::Ipv4, AddressFamily::Ipv6];
type ContextFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
type ProbeFuture = Pin<Box<dyn Future<Output = Result<NetworkAddress, AddressError>> + Send>>;

trait Source: Send + Sync {
    fn context(&self) -> ContextFuture;
    fn probe(&self, family: AddressFamily) -> ProbeFuture;
    fn publish(&self, snapshot: Snapshot);
}
struct NativeSource(tauri::AppHandle);
impl Source for NativeSource {
    fn context(&self) -> ContextFuture {
        Box::pin(async {
            tokio::task::spawn_blocking(network_addresses::network_context_key)
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())
        })
    }
    fn probe(&self, family: AddressFamily) -> ProbeFuture {
        let metadata = crate::services::client_request_metadata::collect(&self.0);
        Box::pin(async move {
            crate::services::client_request_metadata::log_request("public_ip", &metadata);
            network_addresses::probe(family, &metadata).await
        })
    }
    fn publish(&self, snapshot: Snapshot) {
        // Hidden WebViews do not receive cache events.
        let visible = self
            .0
            .try_state::<Arc<super::runtime::ResidentState>>()
            .is_some_and(|state| {
                state.panel_open.load(std::sync::atomic::Ordering::Relaxed)
                    && state
                        .panel_page
                        .lock()
                        .is_ok_and(|page| *page == super::panel_page::PanelPage::Network)
            });
        if visible {
            let _ = self.0.emit_to(super::PANEL_LABEL, EVENT, snapshot);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    address: Option<NetworkAddress>,
    loading: bool,
    failed: bool,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct Probes {
    ipv4: Probe,
    ipv6: Probe,
}
impl Probes {
    fn get_mut(&mut self, family: AddressFamily) -> &mut Probe {
        match family {
            AddressFamily::Ipv4 => &mut self.ipv4,
            AddressFamily::Ipv6 => &mut self.ipv6,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    schema_version: u8,
    revision: u64,
    probes: Probes,
    context_failed: bool,
    updated_at_ms: u64,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            probes: Probes::default(),
            context_failed: false,
            updated_at_ms: 0,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Request {
    Observe,
    Open,
    Refresh,
}
#[derive(Clone, Copy, Debug)]
enum Trigger {
    Open,
    Manual,
    NetworkChanged,
}

#[derive(Default)]
struct Cache {
    snapshot: Snapshot,
    enabled: bool,
    generation: u64,
    context: Option<String>,
    cached_at: Option<Instant>,
    flights: [Option<u64>; 2],
    wanted: [bool; 2],
}
impl Cache {
    fn changed(&mut self) {
        self.snapshot.revision = self.snapshot.revision.saturating_add(1);
    }
    fn invalidate(&mut self) {
        self.generation += 1;
        self.context = None;
        self.cached_at = None;
        self.snapshot.probes = Probes::default();
        self.snapshot.updated_at_ms = 0;
        self.snapshot.context_failed = false;
        self.wanted = [false; 2];
        self.changed();
    }
    fn expired(&self) -> bool {
        self.cached_at
            .is_none_or(|cached_at| cached_at.elapsed() >= CACHE_AGE)
            || super::sampling_workers::timestamp_ms()
                .checked_sub(self.snapshot.updated_at_ms)
                .is_none_or(|age| age >= CACHE_AGE.as_millis() as u64)
            || (self.snapshot.probes.ipv4.failed && self.snapshot.probes.ipv6.failed)
    }
}

pub struct PublicNetwork {
    cache: Mutex<Cache>,
    context_gate: tokio::sync::Mutex<()>,
    source: Arc<dyn Source>,
}
impl PublicNetwork {
    fn new(source: Arc<dyn Source>) -> Arc<Self> {
        Arc::new(Self {
            cache: Mutex::new(Cache::default()),
            context_gate: tokio::sync::Mutex::new(()),
            source,
        })
    }
    pub fn configure(&self, preferences: &super::preferences::ResidentPreferences) {
        self.configure_policy(preferences.enabled && preferences.public_network.enabled);
    }
    fn configure_policy(&self, enabled: bool) {
        let snapshot = {
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            if cache.enabled != enabled {
                cache.enabled = enabled;
                cache.invalidate();
            }
            cache.snapshot.clone()
        };
        self.source.publish(snapshot);
    }
    pub fn snapshot(&self) -> Snapshot {
        self.cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot
            .clone()
    }
    pub async fn request(self: &Arc<Self>, request: Request) -> Snapshot {
        self.inspect(match request {
            Request::Observe => None,
            Request::Open => Some(Trigger::Open),
            Request::Refresh => Some(Trigger::Manual),
        })
        .await;
        self.snapshot()
    }
    async fn inspect(self: &Arc<Self>, trigger: Option<Trigger>) {
        let _gate = self.context_gate.lock().await;
        let generation = {
            let cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            if !cache.enabled {
                return;
            }
            cache.generation
        };
        let context = self.source.context().await;
        let refresh = {
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            if !cache.enabled || generation != cache.generation {
                return;
            }
            let mut changed = false;
            match context {
                Ok(key) => {
                    if cache.context.as_ref() != Some(&key) {
                        cache.invalidate();
                        cache.context = Some(key);
                        changed = true;
                    }
                    cache.snapshot.context_failed = false;
                }
                error => {
                    if !cache.snapshot.context_failed {
                        log::warn!(
                            "public_network_context_failed error={}",
                            mangodisk_platform::diagnostics::text(&format!("{error:?}"))
                        );
                    }
                    cache.snapshot.context_failed = true;
                }
            }
            cache.changed();
            if changed {
                Some(Trigger::NetworkChanged)
            } else {
                trigger.filter(|trigger| !matches!(trigger, Trigger::Open) || cache.expired())
            }
        };
        self.source.publish(self.snapshot());
        if let Some(trigger) = refresh {
            self.begin(trigger, None);
        }
    }
    fn begin(self: &Arc<Self>, trigger: Trigger, only: Option<usize>) {
        let jobs = {
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            if !cache.enabled {
                return;
            }
            let mut jobs = Vec::new();
            for (index, family) in FAMILIES.into_iter().enumerate() {
                if only.is_some_and(|only| only != index) {
                    continue;
                }
                if let Some(generation) = cache.flights[index] {
                    cache.wanted[index] = generation != cache.generation;
                } else {
                    cache.flights[index] = Some(cache.generation);
                    cache.wanted[index] = false;
                    jobs.push((index, family, cache.generation));
                }
                let probe = cache.snapshot.probes.get_mut(family);
                probe.loading = true;
                probe.failed = false;
            }
            // Waiting for obsolete flights changes loading state without starting a new job.
            cache.changed();
            jobs
        };
        self.source.publish(self.snapshot());
        if !jobs.is_empty() {
            log::info!(
                "public_network_refresh_started trigger={trigger:?} probes={}",
                jobs.len()
            );
        }
        for (index, family, generation) in jobs {
            let worker = self.clone();
            tokio::spawn(async move {
                let result = worker.source.probe(family).await;
                worker.complete(index, family, generation, result);
            });
        }
    }
    fn complete(
        self: &Arc<Self>,
        index: usize,
        family: AddressFamily,
        generation: u64,
        result: Result<NetworkAddress, AddressError>,
    ) {
        let restart = {
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            cache.flights[index] = None;
            if generation == cache.generation && cache.enabled {
                let ready = result.is_ok();
                let probe = cache.snapshot.probes.get_mut(family);
                probe.loading = false;
                match result {
                    Ok(address) => {
                        probe.address = Some(address);
                        probe.failed = false;
                    }
                    Err(error) => {
                        probe.failed = true;
                        log::warn!("public_network_probe_failed family={family:?} error={error}");
                    }
                }
                if ready {
                    cache.cached_at = Some(Instant::now());
                    cache.snapshot.updated_at_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                }
                cache.changed();
                log::info!("public_network_probe_completed family={family:?} ready={ready}");
            }
            cache.enabled && cache.wanted[index]
        };
        self.source.publish(self.snapshot());
        if restart {
            self.begin(Trigger::NetworkChanged, Some(index));
        }
    }
}

pub fn install(app: &tauri::AppHandle, preferences: &super::preferences::ResidentPreferences) {
    let worker = PublicNetwork::new(Arc::new(NativeSource(app.clone())));
    worker.configure(preferences);
    app.manage(worker);
}

#[cfg(test)]
#[path = "public_network_tests.rs"]
mod tests;
