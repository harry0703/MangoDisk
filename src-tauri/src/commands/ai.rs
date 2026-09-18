use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use mangodisk_core::ai::{
    discover_local_models, explain, AiConfiguration, AiConfigurationUpdate, AiDelta, AiError,
    AiPreferences, AiRequest, AiSettings, AiUsage, InstalledLocalModel,
};
use tauri::{ipc::Channel, State};
use tokio::sync::watch;

struct ActiveRequest {
    cancel: watch::Sender<bool>,
    running: bool,
    created: Instant,
}

#[derive(Default, Clone)]
pub(crate) struct AiRuntime(Arc<Mutex<AiRuntimeState>>);

#[derive(Default)]
struct AiRuntimeState {
    preferences: Option<AiPreferences>,
    active: HashMap<String, ActiveRequest>,
    // Each disable invalidates existing quota reads even after a quick re-enable.
    disabled_revision: watch::Sender<u64>,
}

impl AiRuntimeState {
    fn require_enabled(&self) -> Result<(), AiError> {
        if self.preferences.is_some_and(|value| value.enabled) {
            Ok(())
        } else {
            Err(AiError::Disabled)
        }
    }

    fn apply_preferences(&mut self, preferences: AiPreferences) {
        self.preferences = Some(preferences);
        if !preferences.enabled {
            self.disabled_revision
                .send_modify(|revision| *revision += 1);
            for request in self.active.values() {
                request.cancel.send_replace(true);
            }
            self.active.retain(|_, request| request.running);
        }
    }
}

// Five module streams and one connection test can coexist. Keep reservations
// bounded so abandoned IPC calls cannot consume unbounded client resources.
const MAX_ACTIVE_REQUESTS: usize = 6;

impl AiRuntime {
    fn preferences(&self) -> Result<AiPreferences, AiError> {
        let mut state = self.0.lock().map_err(|_| AiError::Busy)?;
        if let Some(preferences) = state.preferences {
            return Ok(preferences);
        }
        let preferences = AiPreferences::load()?;
        state.apply_preferences(preferences);
        Ok(preferences)
    }

    fn save_preferences(&self, enabled: bool) -> Result<AiPreferences, AiError> {
        let mut state = self.0.lock().map_err(|_| AiError::Busy)?;
        // Serialize persistence with request admission. A failed atomic write
        // leaves the last confirmed setting intact; success cancels all modules
        // and connection tests before acknowledging the toggle to the UI.
        let preferences = AiPreferences::save(enabled)?;
        let active_count = state.active.len();
        state.apply_preferences(preferences);
        log::info!("ai_feature_preference_saved enabled={enabled} active_count={active_count}");
        Ok(preferences)
    }

    fn quota_permit(&self) -> Result<watch::Receiver<u64>, AiError> {
        let state = self.0.lock().map_err(|_| AiError::Busy)?;
        state.require_enabled()?;
        Ok(state.disabled_revision.subscribe())
    }

    fn begin(&self) -> Result<String, AiError> {
        let mut state = self.0.lock().map_err(|_| AiError::Busy)?;
        state.require_enabled()?;
        let active = &mut state.active;
        let before = active.len();
        active.retain(|_, value| value.running || value.created.elapsed().as_secs() < 60);
        if before != active.len() {
            log::warn!("ai_reservations_expired count={}", before - active.len());
        }
        if active.len() >= MAX_ACTIVE_REQUESTS {
            log::warn!("ai_request_capacity_reached active_count={}", active.len());
            return Err(AiError::Busy);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (cancel, _) = watch::channel(false);
        active.insert(
            id.clone(),
            ActiveRequest {
                cancel,
                running: false,
                created: Instant::now(),
            },
        );
        log::debug!(
            "ai_request_reserved operation_id={id} active_count={}",
            active.len()
        );
        Ok(id)
    }

    fn start(&self, id: &str) -> Result<watch::Receiver<bool>, AiError> {
        let mut state = self.0.lock().map_err(|_| AiError::Busy)?;
        state.require_enabled()?;
        let active = &mut state.active;
        let value = active
            .get_mut(id)
            .filter(|value| !value.running)
            .ok_or(AiError::Cancelled)?;
        value.running = true;
        Ok(value.cancel.subscribe())
    }

    fn cancel(&self, id: &str) -> Result<(), AiError> {
        let mut state = self.0.lock().map_err(|_| AiError::Busy)?;
        let active = &mut state.active;
        if let Some(value) = active.get_mut(id) {
            value.cancel.send_replace(true);
            log::debug!(
                "ai_request_cancelled operation_id={id} running={}",
                value.running
            );
            // Running requests own their slot until transport cleanup completes.
            if !value.running {
                active.remove(id);
            }
        }
        Ok(())
    }

    fn finish(&self, id: &str) {
        if let Ok(mut active) = self.0.lock() {
            active.active.remove(id);
        }
    }
}

#[tauri::command]
pub(crate) async fn ai_get_preferences(
    state: State<'_, AiRuntime>,
) -> Result<AiPreferences, AiError> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || runtime.preferences())
        .await
        .map_err(|_| AiError::ConfigurationUnavailable)?
}

#[tauri::command]
pub(crate) async fn ai_set_enabled(
    enabled: bool,
    state: State<'_, AiRuntime>,
) -> Result<AiPreferences, AiError> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || runtime.save_preferences(enabled))
        .await
        .map_err(|_| AiError::ConfigurationUnavailable)?
}

#[tauri::command]
pub(crate) async fn ai_get_settings() -> Result<Option<AiSettings>, AiError> {
    let result = tauri::async_runtime::spawn_blocking(|| {
        AiConfiguration::load()
            .map(|value| Some(value.unwrap_or_else(AiConfiguration::initial).settings()))
    })
    .await
    .map_err(|_| AiError::ConfigurationUnavailable)?;
    if let Err(reason) = &result {
        log::warn!("ai_configuration_read_failed reason={reason:?}");
    }
    result
}

#[tauri::command]
pub(crate) async fn ai_get_configuration() -> Result<AiEditorState, AiError> {
    let started = Instant::now();
    // Read once for the editor rather than loading the same file through both
    // configuration and public-settings commands. The capability is build-only.
    let result = tauri::async_runtime::spawn_blocking(|| {
        AiConfiguration::load().map(|configuration| AiEditorState {
            configuration,
            free_available: AiConfiguration::initial().settings().free_available,
        })
    })
    .await
    .map_err(|_| AiError::ConfigurationUnavailable)
    .and_then(|result| result);
    log::info!(
        "ai_editor_configuration_loaded duration_ms={} success={} reason={:?}",
        started.elapsed().as_millis(),
        result.is_ok(),
        result.as_ref().err()
    );
    result
}

// This IPC snapshot is not persisted. Never derive Debug: it carries a secret.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiEditorState {
    configuration: Option<AiConfiguration>,
    free_available: bool,
}

#[tauri::command]
pub(crate) async fn ai_save_settings(update: AiConfigurationUpdate) -> Result<AiSettings, AiError> {
    let result = tauri::async_runtime::spawn_blocking(|| AiConfiguration::save(update))
        .await
        .map_err(|_| AiError::ConfigurationUnavailable)?;
    log::info!(
        "ai_configuration_saved success={} reason={:?}",
        result.is_ok(),
        result.as_ref().err()
    );
    result
}

#[tauri::command]
pub(crate) async fn ai_delete_settings() -> Result<(), AiError> {
    let result = tauri::async_runtime::spawn_blocking(AiConfiguration::delete)
        .await
        .map_err(|_| AiError::ConfigurationUnavailable)?;
    log::info!(
        "ai_configuration_deleted success={} reason={:?}",
        result.is_ok(),
        result.as_ref().err()
    );
    result
}

/// Reserving before streaming lets the UI cancel even while settings are
/// being read. Only backend-generated IDs are accepted or written to logs.
#[tauri::command]
pub(crate) fn ai_begin(state: State<'_, AiRuntime>) -> Result<String, AiError> {
    state.begin()
}

#[tauri::command]
pub(crate) fn ai_cancel(id: String, state: State<'_, AiRuntime>) -> Result<(), AiError> {
    state.cancel(&id)
}

#[tauri::command]
pub(crate) async fn ai_explain(
    id: String,
    request: AiRequest,
    metadata: Option<mangodisk_core::ai::AiClientMetadata>,
    expected_mode: mangodisk_core::ai::AiServiceMode,
    on_delta: Channel<AiDelta>,
    state: State<'_, AiRuntime>,
) -> Result<AiUsage, AiError> {
    let cancel = state.start(&id)?;
    let started = Instant::now();
    log::info!(
        "ai_request_started operation_id={id} module={} context_schema={:?} connection_test={}",
        request
            .context
            .as_ref()
            .map_or("connectionTest", |context| context.subject.module_name()),
        request
            .context
            .as_ref()
            .map(|context| context.schema_version),
        request.context.is_none()
    );
    let result = async {
        let config = tauri::async_runtime::spawn_blocking(AiConfiguration::load)
            .await
            .map_err(|_| AiError::ConfigurationUnavailable)??
            .unwrap_or_else(AiConfiguration::initial);
        if *cancel.borrow() {
            return Err(AiError::Cancelled);
        }
        if config.mode != expected_mode {
            return Err(AiError::InvalidConfiguration);
        }
        if config.mode == mangodisk_core::ai::AiServiceMode::Free {
            return mangodisk_core::ai::official_explain(
                config,
                request,
                metadata.ok_or(AiError::InvalidContext)?,
                &id,
                cancel,
                |text| on_delta.send(text).is_ok(),
            )
            .await;
        }
        explain(config, request, &id, cancel, |text| {
            on_delta.send(text).is_ok()
        })
        .await
    }
    .await;
    state.finish(&id);
    match &result {
        Ok(usage) => log::info!("ai_request_completed operation_id={id} elapsed_ms={} prompt_tokens={:?} completion_tokens={:?}", started.elapsed().as_millis(), usage.prompt_tokens, usage.completion_tokens),
        Err(AiError::Cancelled) => log::info!("ai_request_cancelled operation_id={id} elapsed_ms={}", started.elapsed().as_millis()),
        Err(reason) => log::warn!("ai_request_failed operation_id={id} elapsed_ms={} reason={reason:?}", started.elapsed().as_millis()),
    }
    result
}

#[tauri::command]
pub(crate) async fn ai_get_quota(
    metadata: mangodisk_core::ai::AiClientMetadata,
    state: State<'_, AiRuntime>,
) -> Result<mangodisk_core::ai::AiQuota, AiError> {
    let mut disabled = state.quota_permit()?;
    tokio::select! {
        biased;
        _ = disabled.changed() => Err(AiError::Cancelled),
        result = mangodisk_core::ai::official_quota(metadata) => result,
    }
}

#[tauri::command]
pub(crate) async fn ai_list_local_models() -> super::error::CommandResult<Vec<InstalledLocalModel>>
{
    super::error::run_blocking("ai_list_local_models", discover_local_models).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn enabled_runtime() -> AiRuntime {
        let runtime = AiRuntime::default();
        runtime
            .0
            .lock()
            .unwrap()
            .apply_preferences(AiPreferences::default());
        runtime
    }

    #[test]
    fn unloaded_and_disabled_preferences_block_all_network_admission() {
        let runtime = AiRuntime::default();
        assert!(matches!(runtime.begin(), Err(AiError::Disabled)));
        assert!(matches!(runtime.quota_permit(), Err(AiError::Disabled)));
        runtime.0.lock().unwrap().apply_preferences(AiPreferences {
            enabled: false,
            ..AiPreferences::default()
        });
        assert!(matches!(runtime.begin(), Err(AiError::Disabled)));
        assert!(matches!(runtime.start("unknown"), Err(AiError::Disabled)));
        assert!(matches!(runtime.quota_permit(), Err(AiError::Disabled)));
    }

    #[test]
    fn disabling_cancels_every_stream_reservation_and_quota_without_reviving_them() {
        let runtime = enabled_runtime();
        let reserved = runtime.begin().unwrap();
        let running = runtime.begin().unwrap();
        let cancellation = runtime.start(&running).unwrap();
        let quota = runtime.quota_permit().unwrap();
        runtime.0.lock().unwrap().apply_preferences(AiPreferences {
            enabled: false,
            ..AiPreferences::default()
        });
        assert!(*cancellation.borrow());
        assert!(quota.has_changed().unwrap());
        assert!(matches!(runtime.begin(), Err(AiError::Disabled)));
        runtime
            .0
            .lock()
            .unwrap()
            .apply_preferences(AiPreferences::default());
        assert!(*cancellation.borrow());
        assert!(quota.has_changed().unwrap());
        assert!(matches!(runtime.start(&reserved), Err(AiError::Cancelled)));
        runtime.finish(&running);
        let fresh = runtime.begin().unwrap();
        assert!(!*runtime.start(&fresh).unwrap().borrow());
        assert!(!runtime.quota_permit().unwrap().has_changed().unwrap());
    }

    #[test]
    fn editor_snapshot_uses_the_frontend_contract_without_changing_persisted_configuration() {
        let state = AiEditorState {
            configuration: None,
            free_available: true,
        };
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!({"configuration": null, "freeAvailable": true})
        );
        let state = AiEditorState {
            configuration: Some(AiConfiguration::initial()),
            free_available: false,
        };
        let value = serde_json::to_value(state).unwrap();
        assert_eq!(value["configuration"]["schemaVersion"], 2);
        assert!(value["configuration"].get("freeAvailable").is_none());
    }

    #[test]
    fn parallel_requests_cancel_and_finish_independently() {
        let runtime = enabled_runtime();
        let first = runtime.begin().unwrap();
        let second = runtime.begin().unwrap();
        let first_cancel = runtime.start(&first).unwrap();
        let second_cancel = runtime.start(&second).unwrap();
        runtime.cancel(&first).unwrap();
        assert!(*first_cancel.borrow());
        assert!(!*second_cancel.borrow());
        runtime.finish(&first);
        assert!(runtime.0.lock().unwrap().active.contains_key(&second));
        runtime.cancel(&first).unwrap();
        assert!(!*second_cancel.borrow());
        runtime.finish(&second);
        assert!(runtime.0.lock().unwrap().active.is_empty());
    }

    #[test]
    fn five_modules_and_connection_test_fit_with_bounded_capacity() {
        let runtime = enabled_runtime();
        let ids: Vec<_> = (0..MAX_ACTIVE_REQUESTS)
            .map(|_| runtime.begin().unwrap())
            .collect();
        for id in &ids {
            runtime.start(id).unwrap();
        }
        assert!(matches!(runtime.begin(), Err(AiError::Busy)));
        runtime.cancel(&ids[0]).unwrap();
        assert!(matches!(runtime.begin(), Err(AiError::Busy)));
        runtime.finish(&ids[0]);
        assert!(runtime.begin().is_ok());
    }

    #[test]
    fn reservations_expire_without_evicting_running_streams() {
        let runtime = enabled_runtime();
        let reserved = runtime.begin().unwrap();
        let running = runtime.begin().unwrap();
        runtime.start(&running).unwrap();
        for request in runtime.0.lock().unwrap().active.values_mut() {
            request.created = Instant::now() - Duration::from_secs(61);
        }
        runtime.begin().unwrap();
        let state = runtime.0.lock().unwrap();
        let active = &state.active;
        assert!(!active.contains_key(&reserved));
        assert!(active.contains_key(&running));
    }

    #[test]
    fn cancelled_unknown_and_replayed_reservations_cannot_start() {
        let runtime = enabled_runtime();
        let cancelled = runtime.begin().unwrap();
        runtime.cancel(&cancelled).unwrap();
        assert!(matches!(runtime.start(&cancelled), Err(AiError::Cancelled)));
        assert!(matches!(runtime.start("unknown"), Err(AiError::Cancelled)));
        let running = runtime.begin().unwrap();
        runtime.start(&running).unwrap();
        assert!(matches!(runtime.start(&running), Err(AiError::Cancelled)));
        runtime.finish(&running);
        assert!(matches!(runtime.start(&running), Err(AiError::Cancelled)));
    }
}
