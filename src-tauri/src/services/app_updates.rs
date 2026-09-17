//! Update discovery belongs to the process, not a lazily created WebView.
//! Installation remains an explicit action in the existing updater interface.
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};
use tauri::Manager;
use tauri_plugin_store::StoreExt;
use tauri_plugin_updater::UpdaterExt;
use tokio::sync::Notify;

const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const START_DELAY: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_secs(30);
const TIMEOUT: Duration = Duration::from_secs(15);
const EVENT: &str = "app-update-notice";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Notice {
    schema_version: u32,
    revision: u64,
    version: Option<String>,
    checked: bool,
}

struct Schedule {
    due: SystemTime,
    failures: usize,
    notice: Notice,
}
impl Schedule {
    fn new(now: SystemTime) -> Self {
        Self {
            due: now + START_DELAY,
            failures: 0,
            notice: Notice {
                schema_version: 1,
                revision: 0,
                version: None,
                checked: false,
            },
        }
    }
    fn ready(&self, now: SystemTime) -> bool {
        // A clock correction must not postpone checks indefinitely. Wall time
        // also counts sleep on platforms whose monotonic clock pauses in sleep.
        now >= self.due
            || self
                .due
                .duration_since(now)
                .is_ok_and(|wait| wait > INTERVAL)
    }
    fn finish(&mut self, now: SystemTime, result: Result<Option<String>, ()>) -> bool {
        match result {
            Ok(version) => {
                self.failures = 0;
                self.due = now + INTERVAL;
                let changed = !self.notice.checked || self.notice.version != version;
                self.notice.version = version;
                self.notice.checked = true;
                // Refresh revisions even for the same version: notes may change,
                // and a successful no-update result must clear every window.
                self.notice.revision += 1;
                changed
            }
            Err(()) => {
                let delays = [60, 300, 1800, 3600];
                self.due = now + Duration::from_secs(delays[self.failures.min(delays.len() - 1)]);
                self.failures = self.failures.saturating_add(1);
                // Network failures must not erase a previously discovered release.
                false
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckSource {
    Background,
    Manual,
    Cached,
}

type CheckResult = Result<Option<tauri_plugin_updater::Update>, String>;

struct Cache {
    schedule: Schedule,
    update: Option<tauri_plugin_updater::Update>,
    generation: u64,
    last_result: CheckResult,
}

pub(crate) struct AppUpdates {
    cache: Mutex<Cache>,
    check_gate: tokio::sync::Mutex<()>,
    wake: Notify,
}

impl AppUpdates {
    fn new(now: SystemTime) -> Self {
        Self {
            cache: Mutex::new(Cache {
                schedule: Schedule::new(now),
                update: None,
                generation: 0,
                last_result: Ok(None),
            }),
            check_gate: tokio::sync::Mutex::new(()),
            wake: Notify::new(),
        }
    }

    pub(crate) fn snapshot(&self) -> Notice {
        self.wake.notify_one();
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .schedule
            .notice
            .clone()
    }

    // The generation includes failures, so callers waiting on the same request
    // receive the same result instead of accidentally starting a retry storm.
    async fn check_with<F, Fut>(
        &self,
        source: CheckSource,
        discover: F,
    ) -> Result<(Option<tauri_plugin_updater::Update>, Option<Notice>), String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = CheckResult>,
    {
        let generation = {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if source == CheckSource::Cached && cache.schedule.notice.checked {
                return Ok((cache.update.clone(), None));
            }
            cache.generation
        };
        let _gate = self.check_gate.lock().await;
        {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if cache.generation != generation {
                log::info!(
                    "app_update_check_shared source={source:?} request_id={}",
                    cache.generation
                );
                return cache.last_result.clone().map(|update| (update, None));
            }
            if source == CheckSource::Background && !cache.schedule.ready(SystemTime::now()) {
                return Ok((cache.update.clone(), None));
            }
        }
        let started = std::time::Instant::now();
        let request_id = generation + 1;
        log::info!("app_update_check_started source={source:?} request_id={request_id}");
        let result = discover().await;
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let now = SystemTime::now();
        let changed = cache.schedule.finish(
            now,
            result
                .as_ref()
                .map(|update| update.as_ref().map(|u| u.version.clone()))
                .map_err(|_| ()),
        );
        cache.generation = request_id;
        cache.last_result = result.clone();
        if let Ok(update) = &result {
            cache.update = update.clone();
        }
        let next = cache
            .schedule
            .due
            .duration_since(now)
            .unwrap_or_default()
            .as_secs();
        match &result {
            Ok(update) => log::info!("app_update_check_completed source={source:?} request_id={request_id} available={} version={} changed={changed} next_check_secs={next} elapsed_ms={}", update.is_some(), update.as_ref().map_or("none", |u| u.version.as_str()), started.elapsed().as_millis()),
            Err(error) => log::warn!("app_update_check_failed source={source:?} request_id={request_id} error={} retained_notice={} failures={} retry_secs={next} elapsed_ms={}", mangodisk_platform::diagnostics::text(error), cache.update.is_some(), cache.schedule.failures, started.elapsed().as_millis()),
        }
        result.map(|update| (update, Some(cache.schedule.notice.clone())))
    }

    pub(crate) async fn check(&self, app: &tauri::AppHandle, source: CheckSource) -> CheckResult {
        let (update, notice) = self
            .check_with(source, || async {
                match tokio::time::timeout(TIMEOUT + Duration::from_secs(5), discover(app)).await {
                    Ok(result) => {
                        result.map_err(|error| mangodisk_platform::diagnostics::text(&error))
                    }
                    Err(_) => Err("update_check_timeout".into()),
                }
            })
            .await?;
        if let Some(notice) = notice {
            crate::events::emit(app, EVENT, notice);
        }
        Ok(update)
    }
}

async fn discover(
    app: &tauri::AppHandle,
) -> tauri_plugin_updater::Result<Option<tauri_plugin_updater::Update>> {
    let locale = super::native_labels::NativeLabels::load(app).locale;
    let mut builder = app
        .updater_builder()
        .timeout(TIMEOUT)
        .header("Accept-Language", &locale)?
        .header("x-mangodisk-locale", &locale)?
        .header(
            "x-mangodisk-os-version",
            tauri_plugin_os::version().to_string(),
        )?
        .header(
            "x-mangodisk-distribution",
            crate::commands::app_distribution::current().diagnostic_name(),
        )?;
    // Reuse an existing identity without racing the frontend's first-time
    // identity creation. Missing telemetry must never block signed updates.
    if let Ok(store) = app
        .store_builder("installation.json")
        .disable_auto_save()
        .build()
    {
        if let Some(identity) = store.get("identity") {
            if identity.get("schemaVersion").and_then(|v| v.as_u64()) == Some(1) {
                if let Some(id) = identity
                    .get("installId")
                    .and_then(|v| v.as_str())
                    .and_then(|v| uuid::Uuid::parse_str(v).ok())
                {
                    builder = builder.header("x-mangodisk-install-id", id.to_string())?;
                }
            }
        }
    }
    builder.build()?.check().await
}

pub(crate) fn start(app: &tauri::AppHandle) {
    let state = Arc::new(AppUpdates::new(SystemTime::now()));
    app.manage(state.clone());
    let app = app.clone();
    log::info!(
        "app_update_scheduler_started initial_delay_secs={} interval_secs={}",
        START_DELAY.as_secs(),
        INTERVAL.as_secs()
    );
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(START_DELAY).await;
        state.wake.notify_one();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(POLL) => {},
                _ = state.wake.notified() => {},
            }
            // Errors are logged with backoff by the shared check operation.
            let _ = state.check(&app, CheckSource::Background).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_reads_and_periodic_checks_reuse_a_completed_result() {
        tauri::async_runtime::block_on(async {
            let state = AppUpdates::new(SystemTime::now());
            let (_, notice) = state
                .check_with(CheckSource::Manual, || async { Ok(None) })
                .await
                .unwrap();
            assert!(notice.unwrap().checked);
            for source in [CheckSource::Cached, CheckSource::Background] {
                let (_, notice) = state
                    .check_with(source, || async {
                        panic!("cached result must not access the network")
                    })
                    .await
                    .unwrap();
                assert!(notice.is_none());
            }
            let (_, notice) = state
                .check_with(CheckSource::Manual, || async { Ok(None) })
                .await
                .unwrap();
            assert_eq!(notice.unwrap().revision, 2);
            assert_eq!(state.cache.lock().unwrap().generation, 2);
        });
    }

    #[test]
    fn concurrent_callers_share_success_and_failure_without_duplicate_requests() {
        for fail in [false, true] {
            tauri::async_runtime::block_on(async {
                let state = Arc::new(AppUpdates::new(SystemTime::now()));
                let (started, waiting) = tokio::sync::oneshot::channel();
                let (release, ready) = tokio::sync::oneshot::channel();
                let first_state = state.clone();
                let first = tauri::async_runtime::spawn(async move {
                    first_state
                        .check_with(CheckSource::Manual, || async {
                            started.send(()).unwrap();
                            ready.await.unwrap();
                            if fail {
                                Err("offline".into())
                            } else {
                                Ok(None)
                            }
                        })
                        .await
                });
                waiting.await.unwrap();
                let second = state.check_with(CheckSource::Manual, || async {
                    panic!("concurrent caller must share the active request")
                });
                let mut second = std::pin::pin!(second);
                std::future::poll_fn(|cx| {
                    assert!(std::future::Future::poll(second.as_mut(), cx).is_pending());
                    std::task::Poll::Ready(())
                })
                .await;
                release.send(()).unwrap();
                assert_eq!(first.await.unwrap().is_err(), fail);
                assert_eq!(second.await.is_err(), fail);
                assert_eq!(state.cache.lock().unwrap().generation, 1);
            });
        }
    }

    #[test]
    fn startup_periodic_sleep_and_clock_correction() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100_000);
        let mut s = Schedule::new(now);
        assert!(!s.ready(now));
        assert!(s.ready(now + START_DELAY));
        s.finish(now + START_DELAY, Ok(None));
        assert!(!s.ready(now + Duration::from_secs(60)));
        assert!(s.ready(now + Duration::from_secs(7 * 3600)));
        assert!(s.ready(now - Duration::from_secs(3600)));
    }
    #[test]
    fn failures_back_off_and_retain_notice_then_success_resets() {
        let now = SystemTime::UNIX_EPOCH;
        let mut s = Schedule::new(now);
        assert!(s.finish(now, Ok(Some("2.0.0".into()))));
        assert!(!s.finish(now, Ok(Some("2.0.0".into()))));
        for delay in [60, 300, 1800, 3600, 3600] {
            assert!(!s.finish(now, Err(())));
            assert_eq!(s.due, now + Duration::from_secs(delay));
            assert_eq!(s.notice.version.as_deref(), Some("2.0.0"));
            assert_eq!(s.notice.revision, 2);
        }
        assert!(s.finish(now, Ok(None)));
        assert_eq!(s.failures, 0);
        assert_eq!(s.due, now + INTERVAL);
        assert_eq!(s.notice.revision, 3);
    }
}
