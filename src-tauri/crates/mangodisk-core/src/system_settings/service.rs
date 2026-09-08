use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::{Mutex, OnceLock},
    time::Instant,
};

use mangodisk_platform::{
    current_platform, PlatformCancellation, PlatformErrorCode, PlatformMutationState,
    PlatformSystemSettingChangeRequest, PlatformSystemSettingDiagnosticCode,
    PlatformSystemSettingSnapshot, PlatformSystemSettingValue, SystemSettingsPlatform as _,
};
use serde::{Deserialize, Serialize};

use crate::{
    filesystem::metadata::now_ms,
    history::{
        HistoryService, OperationCategory, OperationDetails, OperationOutcome, OperationRecord,
        SystemOptimizationHistoryItem, SystemOptimizationHistoryItemStatus,
        SystemOptimizationOperationDetails, OPERATION_RECORD_SCHEMA_VERSION,
    },
    shared::{
        application_paths,
        operation::{CoordinatedOperationKind, OperationCancellationToken, OperationGuard},
        CoreError, CoreResult,
    },
};

use super::{
    catalog::{definitions, SettingDefinition},
    CatalogNativeItem, CatalogSession, PendingChangeItem, PendingChangePlan,
    SystemSettingChangeFailureReason, SystemSettingChangeItemResult,
    SystemSettingChangeOutcomeStatus, SystemSettingChangePlanItem, SystemSettingChangeSkipReason,
    SystemSettingChangeSkippedItem, SystemSettingItem, SystemSettingSelectionKind,
    SystemSettingStatus, SystemSettingTargetState, SystemSettingsCatalog,
    SystemSettingsCatalogSummary, SystemSettingsChangePlan, SystemSettingsChangeResult,
    SystemSettingsChangeSelection, SystemSettingsPlatform, SYSTEM_SETTINGS_CATALOG_SCHEMA_VERSION,
    SYSTEM_SETTINGS_CHANGE_PLAN_SCHEMA_VERSION,
};

const CHANGE_PLAN_TTL_MS: u64 = 5 * 60 * 1_000;
const MAX_CHANGE_ITEMS: usize = 256;
const MAX_RECOVERY_ITEMS: usize = 1024;
const RECOVERY_SCHEMA_VERSION: u32 = 2;
const MIN_SUPPORTED_RECOVERY_SCHEMA_VERSION: u32 = 1;
const RECOVERY_MAX_BYTES: u64 = 1024 * 1024;
const RECOVERY_MAX_TEXT_BYTES: usize = 1024;
const RECOVERY_FILE_NAME: &str = "system-settings-recovery.json";
const INVALID_RECOVERY_FILE_NAME: &str = "system-settings-recovery.invalid.json";

static CATALOG_SESSION: OnceLock<Mutex<Option<CatalogSession>>> = OnceLock::new();
static CHANGE_PLAN: OnceLock<Mutex<Option<PendingChangePlan>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryDocument {
    schema_version: u32,
    recovery_id: String,
    created_at_ms: u64,
    items: Vec<RecoveryItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryItem {
    setting_id: String,
    original_value: PlatformSystemSettingValue,
    optimized_value: PlatformSystemSettingValue,
}

pub struct SystemSettingsService;

impl SystemSettingsService {
    pub fn cancel_scan() {
        OperationCancellationToken::system_settings_scan().cancel();
    }

    pub fn scan() -> CoreResult<SystemSettingsCatalog> {
        let operation = OperationGuard::start(CoordinatedOperationKind::SystemSettingsScan)?;
        let session = capture_catalog(&operation)?;
        replace_catalog_session(session.clone())?;
        log_catalog(operation.id(), &session.public);
        operation.complete();
        Ok(session.public)
    }

    pub fn prepare_change(
        selection: SystemSettingsChangeSelection,
    ) -> CoreResult<SystemSettingsChangePlan> {
        validate_selection(&selection)?;
        let operation = OperationGuard::start(CoordinatedOperationKind::SystemSettingsChange)?;
        let session = current_catalog_session()?;
        if session.public.scan_id != selection.scan_id {
            return Err(CoreError::invalid_input(
                "system settings catalog session has expired",
            ));
        }

        let recovery = load_recovery()?;
        let pending = build_change_plan(selection, session, recovery.as_ref(), now_ms());
        let public = pending.public.clone();
        replace_change_plan(pending)?;
        log::info!(
            "system_settings_change_prepared operation_id={} target_count={} enable_count={} restore_count={} skipped_count={} restart_required={}",
            operation.id(),
            public.items.len(),
            public
                .items
                .iter()
                .filter(|item| item.target == SystemSettingTargetState::Optimized)
                .count(),
            public
                .items
                .iter()
                .filter(|item| item.target == SystemSettingTargetState::Default)
                .count(),
            public.skipped_items.len(),
            public.requires_restart
        );
        operation.complete();
        Ok(public)
    }

    pub fn execute_change(plan_id: String) -> CoreResult<SystemSettingsChangeResult> {
        validate_plan_id(&plan_id)?;
        let operation = OperationGuard::start(CoordinatedOperationKind::SystemSettingsChange)?;
        let started_at_ms = now_ms();
        let pending = take_change_plan(&plan_id)?;
        if now_ms() > pending.public.expires_at_ms {
            return Err(CoreError::invalid_input(
                "system settings change plan has expired",
            ));
        }

        // Cancellation must be observed before replacing the durable recovery record. Once the
        // preflight intent is persisted there is no further cancellable Core step before the
        // platform batch, otherwise an early return could discard recovery for a prior operation.
        operation.ensure_not_cancelled()?;
        let platform = current_platform();
        let mut results = Vec::with_capacity(pending.items.len());
        let mut possibly_changed = BTreeSet::new();
        let previous_recovery = load_recovery()?;
        let preflight_recovery = recovery_intent(previous_recovery.as_ref(), &pending.items);
        // Persist the merged per-setting baseline before the first operating-system mutation.
        // This keeps earlier settings reversible when a later batch changes unrelated items.
        if !preflight_recovery.is_empty() {
            save_recovery(&plan_id, preflight_recovery.clone(), operation.id())?;
        }
        let requests = pending
            .items
            .iter()
            .map(|item| PlatformSystemSettingChangeRequest {
                setting_id: item.public.setting_id.clone(),
                expected_value: item.expected_value.clone(),
                desired_value: item.desired_value.clone(),
            })
            .collect::<Vec<_>>();
        let platform_results = match platform.change_system_settings(&requests) {
            Ok(platform_results) if platform_results.len() == pending.items.len() => {
                Some(platform_results)
            }
            Ok(platform_results) => {
                log::error!(
                    "system_settings_change_batch_count_invalid operation_id={} expected_count={} actual_count={}",
                    operation.id(),
                    pending.items.len(),
                    platform_results.len()
                );
                for item in &pending.items {
                    possibly_changed.insert(item.public.setting_id.clone());
                    results.push(change_result(
                        item.public.setting_id.clone(),
                        SystemSettingChangeOutcomeStatus::Failed,
                        false,
                        Some(SystemSettingChangeFailureReason::PlatformFailure),
                    ));
                }
                None
            }
            Err(error) => {
                log::warn!(
                    "system_settings_change_batch_failed operation_id={} code={:?} error_digest={}",
                    operation.id(),
                    error.code(),
                    blake3::hash(error.as_bytes()).to_hex()
                );
                for item in &pending.items {
                    if error.mutation_state() == PlatformMutationState::MayHaveChanged {
                        possibly_changed.insert(item.public.setting_id.clone());
                    }
                    results.push(change_result(
                        item.public.setting_id.clone(),
                        SystemSettingChangeOutcomeStatus::Failed,
                        false,
                        Some(failure_reason(error.code())),
                    ));
                }
                None
            }
        };
        for (item, platform_result) in pending
            .items
            .iter()
            .zip(platform_results.into_iter().flatten())
        {
            match platform_result {
                Ok(result) if result.verified => {
                    let status = if result.changed {
                        SystemSettingChangeOutcomeStatus::Changed
                    } else {
                        SystemSettingChangeOutcomeStatus::Unchanged
                    };
                    log::info!(
                        "system_setting_change_completed operation_id={} setting_id={} target={:?} changed={} verified=true restart_required={}",
                        operation.id(), item.public.setting_id, item.public.target, result.changed,
                        item.public.requires_restart
                    );
                    results.push(change_result(
                        item.public.setting_id.clone(),
                        status,
                        true,
                        None,
                    ));
                }
                Ok(_) => {
                    possibly_changed.insert(item.public.setting_id.clone());
                    log::warn!(
                        "system_setting_change_verification_failed operation_id={} setting_id={} mutation_state=may_have_changed",
                        operation.id(),
                        item.public.setting_id
                    );
                    results.push(change_result(
                        item.public.setting_id.clone(),
                        SystemSettingChangeOutcomeStatus::Failed,
                        false,
                        Some(SystemSettingChangeFailureReason::VerificationFailed),
                    ));
                }
                Err(error) => {
                    let mutation_state = error.mutation_state();
                    if mutation_state == PlatformMutationState::MayHaveChanged {
                        possibly_changed.insert(item.public.setting_id.clone());
                    }
                    log::warn!(
                        "system_setting_change_failed operation_id={} setting_id={} code={:?} mutation_state={:?} error_digest={}",
                        operation.id(),
                        item.public.setting_id,
                        error.code(),
                        mutation_state,
                        blake3::hash(error.as_bytes()).to_hex()
                    );
                    results.push(change_result(
                        item.public.setting_id.clone(),
                        SystemSettingChangeOutcomeStatus::Failed,
                        false,
                        Some(failure_reason(error.code())),
                    ));
                }
            }
        }

        let persisted_recovery_available = reconcile_recovery(
            previous_recovery.as_ref(),
            &plan_id,
            preflight_recovery,
            &pending.items,
            &results,
            &possibly_changed,
            operation.id(),
        );
        let changed_count = count_status(&results, SystemSettingChangeOutcomeStatus::Changed);
        let failed_count = count_status(&results, SystemSettingChangeOutcomeStatus::Failed);
        let requires_restart = changed_items_require_restart(&pending.items, &results);
        let catalog = capture_catalog(&operation)
            .and_then(|session| {
                replace_catalog_session(session.clone())?;
                Ok(session.public)
            })
            .ok();
        let recovery_available = catalog
            .as_ref()
            .map(|catalog| catalog.recovery_available)
            .unwrap_or(persisted_recovery_available);
        log::info!(
            "system_settings_change_finished operation_id={} changed_count={} failed_count={} uncertain_mutation_count={} recovery_available={}",
            operation.id(),
            changed_count,
            failed_count,
            possibly_changed.len(),
            recovery_available
        );
        append_history(
            &plan_id,
            started_at_ms,
            &pending.items,
            &results,
            changed_count,
            failed_count,
        );
        operation.complete();
        Ok(SystemSettingsChangeResult {
            plan_id,
            changed_count,
            failed_count,
            requires_restart,
            recovery_available,
            items: results,
            catalog,
        })
    }
}

/// Builds a deterministic, side-effect-free change plan from one authoritative catalog.
/// Keeping selection decisions in one function prevents GUI and CLI adapters from diverging and
/// lets recovery-sensitive skip behavior be tested without reading or changing real settings.
fn build_change_plan(
    selection: SystemSettingsChangeSelection,
    session: CatalogSession,
    recovery: Option<&RecoveryDocument>,
    created_at_ms: u64,
) -> PendingChangePlan {
    let recovery_items = recovery_items_by_id(recovery);
    let mut items = Vec::new();
    let mut skipped_items = Vec::new();
    for change in selection.items {
        let setting_id = change.setting_id;
        let Some(item) = session
            .native_items
            .iter()
            .find(|item| item.public.setting_id == setting_id)
        else {
            skipped_items.push(skipped(
                setting_id,
                SystemSettingChangeSkipReason::SettingMissing,
            ));
            continue;
        };
        if item.public.status == SystemSettingStatus::Unavailable {
            skipped_items.push(skipped(
                setting_id,
                skip_reason_for_diagnostic(item.public.diagnostic),
            ));
            continue;
        }
        if change.target == SystemSettingTargetState::Optimized
            && item.public.status == SystemSettingStatus::Optimized
        {
            skipped_items.push(skipped(
                setting_id,
                SystemSettingChangeSkipReason::AlreadyOptimized,
            ));
            continue;
        }
        if change.target == SystemSettingTargetState::Default
            && item.public.status != SystemSettingStatus::Optimized
            && !item.public.restore_available
        {
            skipped_items.push(skipped(
                setting_id,
                SystemSettingChangeSkipReason::AlreadyDefault,
            ));
            continue;
        }
        let Some(desired_value) =
            desired_value_for_target(item, change.target, recovery_items.get(&setting_id))
        else {
            skipped_items.push(skipped(
                setting_id,
                SystemSettingChangeSkipReason::SettingChanged,
            ));
            continue;
        };
        let public = SystemSettingChangePlanItem {
            setting_id: setting_id.clone(),
            category: item.public.category,
            target: change.target,
            requires_restart: item.public.requires_restart,
            requires_elevation: item.public.requires_elevation,
        };
        items.push(PendingChangeItem {
            public,
            expected_value: item.current_value.clone(),
            desired_value,
        });
    }

    let expires_at_ms = created_at_ms.saturating_add(CHANGE_PLAN_TTL_MS);
    let plan_id = stable_id(
        "system-settings-plan",
        &[
            session.public.catalog_revision.as_str(),
            created_at_ms.to_string().as_str(),
            items
                .iter()
                .map(|item| {
                    format!(
                        "{}:{}",
                        item.public.setting_id,
                        target_state_name(item.public.target)
                    )
                })
                .collect::<Vec<_>>()
                .join("|")
                .as_str(),
        ],
    );
    let public = SystemSettingsChangePlan {
        schema_version: SYSTEM_SETTINGS_CHANGE_PLAN_SCHEMA_VERSION,
        plan_id,
        scan_id: session.public.scan_id,
        catalog_revision: session.public.catalog_revision,
        created_at_ms,
        expires_at_ms,
        items: items.iter().map(|item| item.public.clone()).collect(),
        skipped_items,
        requires_confirmation: !items.is_empty(),
        requires_restart: items.iter().any(|item| item.public.requires_restart),
    };
    PendingChangePlan { public, items }
}

fn capture_catalog(operation: &OperationGuard) -> CoreResult<CatalogSession> {
    let started = Instant::now();
    let platform_kind = current_platform_kind();
    let definitions = definitions(platform_kind);
    let setting_ids = definitions.iter().map(|item| item.id).collect::<Vec<_>>();
    let cancellation = PlatformCancellation::new({
        let cancelled = operation.cancellation_flag();
        move || cancelled.load(std::sync::atomic::Ordering::Relaxed)
    });
    let states = current_platform().scan_system_settings(&setting_ids, &cancellation)?;
    operation.ensure_not_cancelled()?;
    let native_items = catalog_native_items(platform_kind, states)?;
    let recovery = load_recovery()?;
    Ok(assemble_catalog_session(
        platform_kind,
        started,
        native_items,
        recovery.as_ref(),
    ))
}

/// Validates the adapter inventory before applying declarative setting definitions. Missing
/// states remain visible as unavailable settings, while duplicate or unknown identifiers fail
/// closed because silently overwriting them could authorize a change from ambiguous evidence.
fn catalog_native_items(
    platform_kind: SystemSettingsPlatform,
    states: Vec<mangodisk_platform::PlatformSystemSettingState>,
) -> CoreResult<Vec<CatalogNativeItem>> {
    let definitions = definitions(platform_kind);
    let known_ids = definitions
        .iter()
        .map(|definition| definition.id)
        .collect::<BTreeSet<_>>();
    let mut indexed_states = BTreeMap::new();
    for state in states {
        if !known_ids.contains(state.setting_id.as_str()) {
            return Err(CoreError::new(
                crate::shared::CoreErrorCode::Platform,
                "system settings adapter returned an unknown setting",
            ));
        }
        if indexed_states
            .insert(state.setting_id.clone(), state)
            .is_some()
        {
            return Err(CoreError::new(
                crate::shared::CoreErrorCode::Platform,
                "system settings adapter returned a duplicate setting",
            ));
        }
    }
    Ok(definitions
        .iter()
        .map(|definition| native_item(definition, indexed_states.get(definition.id)))
        .collect())
}

fn assemble_catalog_session(
    platform_kind: SystemSettingsPlatform,
    started: Instant,
    mut native_items: Vec<CatalogNativeItem>,
    recovery: Option<&RecoveryDocument>,
) -> CatalogSession {
    let recovery_available = mark_restore_availability(&mut native_items, recovery);
    let scanned_at_ms = now_ms();
    let catalog_revision = catalog_revision(&native_items);
    let scan_id = stable_id(
        "system-settings-scan",
        &[
            catalog_revision.as_str(),
            scanned_at_ms.to_string().as_str(),
        ],
    );
    let items = native_items
        .iter()
        .map(|item| item.public.clone())
        .collect::<Vec<_>>();
    let public = SystemSettingsCatalog {
        schema_version: SYSTEM_SETTINGS_CATALOG_SCHEMA_VERSION,
        scan_id,
        catalog_revision,
        platform: platform_kind,
        scanned_at_ms,
        summary: summary(&items),
        items,
        elapsed_ms: started.elapsed().as_millis() as u64,
        recovery_available,
    };
    CatalogSession {
        public,
        native_items,
    }
}

fn native_item(
    definition: &SettingDefinition,
    state: Option<&mangodisk_platform::PlatformSystemSettingState>,
) -> CatalogNativeItem {
    let elevation_mismatch =
        state.is_some_and(|state| state.requires_elevation != definition.requires_elevation);
    let diagnostic = if elevation_mismatch {
        Some(PlatformSystemSettingDiagnosticCode::InvalidData)
    } else {
        state.and_then(|state| state.diagnostic).or_else(|| {
            state
                .is_none()
                .then_some(PlatformSystemSettingDiagnosticCode::StateUnavailable)
        })
    };
    let current_value = state
        .map(|state| state.value.clone())
        .unwrap_or(PlatformSystemSettingValue::Missing);
    let effective_value = if current_value == PlatformSystemSettingValue::Missing {
        definition.default_value.owned()
    } else {
        state
            .map(|state| state.effective_value.clone())
            .unwrap_or_else(|| current_value.clone())
    };
    let recommended_value = definition.recommended_value.owned();
    let disabled_value = definition
        .disabled_value
        .unwrap_or(definition.default_value)
        .owned();
    let status = if diagnostic.is_some() {
        SystemSettingStatus::Unavailable
    } else if effective_value == recommended_value {
        SystemSettingStatus::Optimized
    } else {
        SystemSettingStatus::Recommended
    };
    CatalogNativeItem {
        public: SystemSettingItem {
            setting_id: definition.id.to_string(),
            category: definition.category,
            selection_kind: definition.selection_kind,
            risk_level: definition.risk_level,
            status,
            selected_by_default: status == SystemSettingStatus::Recommended
                && definition.selection_kind == SystemSettingSelectionKind::OneClick,
            restore_available: false,
            requires_restart: definition.requires_restart,
            requires_elevation: state
                .map(|state| state.requires_elevation)
                .unwrap_or(definition.requires_elevation),
            diagnostic,
        },
        current_value,
        effective_value,
        disabled_value,
        recommended_value,
    }
}

fn summary(items: &[SystemSettingItem]) -> SystemSettingsCatalogSummary {
    SystemSettingsCatalogSummary {
        item_count: items.len() as u64,
        recommended_count: items
            .iter()
            .filter(|item| item.status == SystemSettingStatus::Recommended)
            .count() as u64,
        optimized_count: items
            .iter()
            .filter(|item| item.status == SystemSettingStatus::Optimized)
            .count() as u64,
        selected_count: items.iter().filter(|item| item.selected_by_default).count() as u64,
        unavailable_count: items
            .iter()
            .filter(|item| item.status == SystemSettingStatus::Unavailable)
            .count() as u64,
    }
}

fn catalog_revision(items: &[CatalogNativeItem]) -> String {
    let serialized = serde_json::to_vec(
        &items
            .iter()
            .map(|item| {
                (
                    &item.public.setting_id,
                    &item.current_value,
                    item.public.diagnostic,
                )
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    blake3::hash(&serialized).to_hex().to_string()
}

fn current_platform_kind() -> SystemSettingsPlatform {
    #[cfg(target_os = "macos")]
    {
        SystemSettingsPlatform::Macos
    }
    #[cfg(windows)]
    {
        SystemSettingsPlatform::Windows
    }
}

fn validate_selection(selection: &SystemSettingsChangeSelection) -> CoreResult<()> {
    if selection.scan_id.is_empty() || selection.scan_id.len() > 128 {
        return Err(CoreError::invalid_input(
            "system settings scan identifier is invalid",
        ));
    }
    if selection.items.is_empty() || selection.items.len() > MAX_CHANGE_ITEMS {
        return Err(CoreError::invalid_input(
            "system settings selection is invalid",
        ));
    }
    if selection
        .items
        .iter()
        .any(|item| item.setting_id.is_empty() || item.setting_id.len() > 128)
    {
        return Err(CoreError::invalid_input(
            "system setting identifier is invalid",
        ));
    }
    let unique_ids = selection
        .items
        .iter()
        .map(|item| item.setting_id.as_str())
        .collect::<BTreeSet<_>>();
    if unique_ids.len() != selection.items.len() {
        return Err(CoreError::invalid_input(
            "system settings selection contains duplicate identifiers",
        ));
    }
    Ok(())
}

fn validate_plan_id(plan_id: &str) -> CoreResult<()> {
    if plan_id.is_empty() || plan_id.len() > 128 {
        Err(CoreError::invalid_input(
            "system settings plan identifier is invalid",
        ))
    } else {
        Ok(())
    }
}

fn stable_id(namespace: &str, parts: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(namespace.as_bytes());
    for part in parts {
        hasher.update(&[0]);
        hasher.update(part.as_bytes());
    }
    format!("{namespace}-{}", &hasher.finalize().to_hex()[..24])
}

fn target_state_name(target: SystemSettingTargetState) -> &'static str {
    match target {
        SystemSettingTargetState::Optimized => "optimized",
        SystemSettingTargetState::Default => "default",
    }
}

fn skipped(
    setting_id: String,
    reason: SystemSettingChangeSkipReason,
) -> SystemSettingChangeSkippedItem {
    SystemSettingChangeSkippedItem { setting_id, reason }
}

fn skip_reason_for_diagnostic(
    diagnostic: Option<PlatformSystemSettingDiagnosticCode>,
) -> SystemSettingChangeSkipReason {
    match diagnostic {
        Some(PlatformSystemSettingDiagnosticCode::Unsupported) => {
            SystemSettingChangeSkipReason::Unsupported
        }
        _ => SystemSettingChangeSkipReason::StateUnavailable,
    }
}

fn failure_reason(code: PlatformErrorCode) -> SystemSettingChangeFailureReason {
    match code {
        PlatformErrorCode::AccessDenied => SystemSettingChangeFailureReason::PermissionDenied,
        PlatformErrorCode::UserCancelled => SystemSettingChangeFailureReason::UserCancelled,
        PlatformErrorCode::ItemChanged => SystemSettingChangeFailureReason::SettingChanged,
        PlatformErrorCode::Unsupported => SystemSettingChangeFailureReason::Unsupported,
        _ => SystemSettingChangeFailureReason::PlatformFailure,
    }
}

fn append_history(
    plan_id: &str,
    started_at_ms: u64,
    pending_items: &[PendingChangeItem],
    results: &[SystemSettingChangeItemResult],
    changed_count: u64,
    failed_count: u64,
) {
    if results.is_empty() {
        return;
    }
    let cancelled = results
        .iter()
        .any(|item| item.failure_reason == Some(SystemSettingChangeFailureReason::UserCancelled));
    let items = results
        .iter()
        .map(|item| SystemOptimizationHistoryItem {
            setting_id: item.setting_id.clone(),
            status: match item.status {
                SystemSettingChangeOutcomeStatus::Changed => {
                    SystemOptimizationHistoryItemStatus::Changed
                }
                SystemSettingChangeOutcomeStatus::Unchanged => {
                    SystemOptimizationHistoryItemStatus::Unchanged
                }
                SystemSettingChangeOutcomeStatus::Failed => {
                    SystemOptimizationHistoryItemStatus::Failed
                }
            },
            failure_reason: item.failure_reason,
            desired_optimized: pending_items
                .iter()
                .find(|pending| pending.public.setting_id == item.setting_id)
                .map(|pending| pending.public.target == SystemSettingTargetState::Optimized),
        })
        .collect::<Vec<_>>();
    let time_component = started_at_ms.to_string();
    let record = OperationRecord {
        schema_version: OPERATION_RECORD_SCHEMA_VERSION,
        operation_id: stable_id("system-settings-change", &[plan_id, &time_component]),
        category: OperationCategory::SystemOptimization,
        started_at_ms,
        finished_at_ms: now_ms(),
        outcome: if cancelled {
            OperationOutcome::Cancelled
        } else if failed_count > 0 {
            OperationOutcome::CompletedWithWarnings
        } else {
            OperationOutcome::Completed
        },
        dry_run: false,
        selected_item_count: items.len() as u64,
        affected_item_count: changed_count,
        expected_bytes: 0,
        released_bytes: None,
        released_bytes_is_estimate: false,
        failed_item_count: failed_count,
        details: OperationDetails::SystemOptimization(SystemOptimizationOperationDetails {
            plan_id: plan_id.to_string(),
            restoration: false,
            items,
        }),
    };
    let history_operation_id = record.operation_id.clone();
    if let Err(error) = HistoryService::append(record) {
        log::warn!(
            "system_settings_history_save_failed history_operation_id={} error_digest={}",
            history_operation_id,
            blake3::hash(error.diagnostic().as_bytes()).to_hex()
        );
    }
}

fn change_result(
    setting_id: String,
    status: SystemSettingChangeOutcomeStatus,
    verified: bool,
    failure_reason: Option<SystemSettingChangeFailureReason>,
) -> SystemSettingChangeItemResult {
    SystemSettingChangeItemResult {
        setting_id,
        status,
        verified,
        failure_reason,
    }
}

fn count_status(
    items: &[SystemSettingChangeItemResult],
    status: SystemSettingChangeOutcomeStatus,
) -> u64 {
    items.iter().filter(|item| item.status == status).count() as u64
}

fn changed_items_require_restart(
    pending_items: &[PendingChangeItem],
    results: &[SystemSettingChangeItemResult],
) -> bool {
    pending_items.iter().any(|pending| {
        pending.public.requires_restart
            && results.iter().any(|result| {
                result.setting_id == pending.public.setting_id
                    && result.status == SystemSettingChangeOutcomeStatus::Changed
            })
    })
}

fn catalog_session_lock() -> &'static Mutex<Option<CatalogSession>> {
    CATALOG_SESSION.get_or_init(|| Mutex::new(None))
}

fn change_plan_lock() -> &'static Mutex<Option<PendingChangePlan>> {
    CHANGE_PLAN.get_or_init(|| Mutex::new(None))
}

fn replace_catalog_session(session: CatalogSession) -> CoreResult<()> {
    *catalog_session_lock()
        .lock()
        .map_err(|_| CoreError::operation_failed("system settings catalog lock is poisoned"))? =
        Some(session);
    Ok(())
}

fn current_catalog_session() -> CoreResult<CatalogSession> {
    catalog_session_lock()
        .lock()
        .map_err(|_| CoreError::operation_failed("system settings catalog lock is poisoned"))?
        .clone()
        .ok_or_else(|| CoreError::invalid_input("scan system settings before preparing changes"))
}

fn replace_change_plan(plan: PendingChangePlan) -> CoreResult<()> {
    *change_plan_lock()
        .lock()
        .map_err(|_| CoreError::operation_failed("system settings plan lock is poisoned"))? =
        Some(plan);
    Ok(())
}

fn take_change_plan(plan_id: &str) -> CoreResult<PendingChangePlan> {
    let mut guard = change_plan_lock()
        .lock()
        .map_err(|_| CoreError::operation_failed("system settings plan lock is poisoned"))?;
    if guard
        .as_ref()
        .is_none_or(|plan| plan.public.plan_id != plan_id)
    {
        return Err(CoreError::invalid_input(
            "system settings change plan is unavailable",
        ));
    }
    guard
        .take()
        .ok_or_else(|| CoreError::invalid_input("system settings change plan is unavailable"))
}

fn recovery_path() -> CoreResult<std::path::PathBuf> {
    Ok(application_paths()?
        .data_directory()
        .join(RECOVERY_FILE_NAME))
}

fn recovery_exists() -> bool {
    recovery_path().is_ok_and(|path| path.is_file())
}

fn save_recovery(recovery_id: &str, items: Vec<RecoveryItem>, operation_id: u64) -> CoreResult<()> {
    if items.len() > MAX_RECOVERY_ITEMS {
        log::error!(
            "system_settings_recovery_save_rejected operation_id={} item_count={} max_item_count={}",
            operation_id,
            items.len(),
            MAX_RECOVERY_ITEMS
        );
        return Err(CoreError::persistence(
            "system settings recovery record exceeds its item limit",
        ));
    }
    let path = recovery_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::persistence("recovery path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        CoreError::persistence(format!("failed to create recovery directory: {error}"))
    })?;
    let document = RecoveryDocument {
        schema_version: RECOVERY_SCHEMA_VERSION,
        recovery_id: recovery_id.to_string(),
        created_at_ms: now_ms(),
        items,
    };
    let content = serde_json::to_vec_pretty(&document).map_err(|error| {
        CoreError::persistence(format!("failed to serialize recovery record: {error}"))
    })?;
    write_atomic(&path, &content)?;
    log::info!(
        "system_settings_recovery_saved operation_id={} schema_version={} item_count={}",
        operation_id,
        RECOVERY_SCHEMA_VERSION,
        document.items.len()
    );
    Ok(())
}

fn recovery_items_by_id(document: Option<&RecoveryDocument>) -> BTreeMap<String, RecoveryItem> {
    document
        .into_iter()
        .flat_map(|document| document.items.iter().cloned())
        .map(|item| (item.setting_id.clone(), item))
        .collect()
}

fn valid_recovery_baseline(recovery: &RecoveryItem, item: &CatalogNativeItem) -> bool {
    recovery.optimized_value == item.effective_value
        && (matches!(recovery.original_value, PlatformSystemSettingValue::Missing)
            || same_value_kind(&recovery.original_value, &item.disabled_value)
            || same_value_kind(&recovery.original_value, &item.recommended_value)
            || matches!(
                (&recovery.original_value, &item.current_value),
                (
                    PlatformSystemSettingValue::Snapshot(_),
                    PlatformSystemSettingValue::Snapshot(_)
                )
            ))
}

/// Resolves the requested target without weakening an explicit recovery operation. Manual
/// switches may still use the catalog's disabled value, while a setting advertised as restorable
/// must retain its exact durable baseline until the change plan is prepared.
fn desired_value_for_target(
    item: &CatalogNativeItem,
    target: SystemSettingTargetState,
    recovery: Option<&RecoveryItem>,
) -> Option<PlatformSystemSettingValue> {
    match target {
        SystemSettingTargetState::Optimized => Some(item.recommended_value.clone()),
        SystemSettingTargetState::Default => recovery
            .filter(|recovery| valid_recovery_baseline(recovery, item))
            .map(|recovery| recovery.original_value.clone())
            .or_else(|| (!item.public.restore_available).then(|| item.disabled_value.clone())),
    }
}

/// Marks only available settings whose current native value still matches MangoDisk's durable
/// baseline. The catalog recommendation may change between releases, so recovery is intentionally
/// independent of the current optimized status. A value changed by another tool still fails the
/// baseline match and cannot appear in the bulk restore action.
fn mark_restore_availability(
    items: &mut [CatalogNativeItem],
    recovery: Option<&RecoveryDocument>,
) -> bool {
    let recovery_items = recovery_items_by_id(recovery);
    let mut available = false;
    for item in items {
        item.public.restore_available = item.public.status != SystemSettingStatus::Unavailable
            && recovery_items
                .get(&item.public.setting_id)
                .is_some_and(|recovery| valid_recovery_baseline(recovery, item));
        available |= item.public.restore_available;
    }
    available
}

fn same_value_kind(left: &PlatformSystemSettingValue, right: &PlatformSystemSettingValue) -> bool {
    matches!(
        (left, right),
        (
            PlatformSystemSettingValue::Boolean(_),
            PlatformSystemSettingValue::Boolean(_)
        ) | (
            PlatformSystemSettingValue::Integer(_),
            PlatformSystemSettingValue::Integer(_)
        ) | (
            PlatformSystemSettingValue::Text(_),
            PlatformSystemSettingValue::Text(_)
        )
    )
}

/// Builds the durable baseline ledger written before native changes begin.
/// Existing baselines are never replaced, so applying a second profile cannot make settings from
/// the first profile irreversible. Only the optimized target is refreshed when policy evolves.
fn recovery_intent(
    previous: Option<&RecoveryDocument>,
    pending_items: &[PendingChangeItem],
) -> Vec<RecoveryItem> {
    let mut items = recovery_items_by_id(previous);
    for pending in pending_items
        .iter()
        .filter(|item| item.public.target == SystemSettingTargetState::Optimized)
    {
        if let Some(existing) = items.get_mut(&pending.public.setting_id) {
            existing.optimized_value = pending.desired_value.clone();
        } else {
            items.insert(
                pending.public.setting_id.clone(),
                RecoveryItem {
                    setting_id: pending.public.setting_id.clone(),
                    original_value: pending.expected_value.clone(),
                    optimized_value: pending.desired_value.clone(),
                },
            );
        }
    }
    items.into_values().collect()
}

fn reconcile_recovery(
    previous: Option<&RecoveryDocument>,
    recovery_id: &str,
    preflight_items: Vec<RecoveryItem>,
    pending_items: &[PendingChangeItem],
    results: &[SystemSettingChangeItemResult],
    possibly_changed: &BTreeSet<String>,
    operation_id: u64,
) -> bool {
    let items = reconciled_recovery_items(
        previous,
        preflight_items,
        pending_items,
        results,
        possibly_changed,
    );
    log::info!(
        "system_settings_recovery_reconciled operation_id={} previous_count={} retained_count={} uncertain_mutation_count={}",
        operation_id,
        previous.map_or(0, |document| document.items.len()),
        items.len(),
        possibly_changed.len()
    );
    let result = if items.is_empty() {
        remove_recovery().map(|_| false)
    } else {
        save_recovery(recovery_id, items, operation_id).map(|_| true)
    };
    match result {
        Ok(available) => available,
        Err(error) => {
            // The preflight document is already durable. A failed compaction must not turn a
            // successfully changed operation into an error or hide the conservative recovery.
            log::warn!(
                "system_settings_recovery_reconcile_failed operation_id={} error_digest={}",
                operation_id,
                blake3::hash(error.diagnostic().as_bytes()).to_hex()
            );
            recovery_exists()
        }
    }
}

/// Resolves the post-batch recovery ledger without filesystem side effects so that mixed enable
/// and restore batches have one deterministic source of truth that can be regression tested.
fn reconciled_recovery_items(
    previous: Option<&RecoveryDocument>,
    preflight_items: Vec<RecoveryItem>,
    pending_items: &[PendingChangeItem],
    results: &[SystemSettingChangeItemResult],
    possibly_changed: &BTreeSet<String>,
) -> Vec<RecoveryItem> {
    let previous_items = recovery_items_by_id(previous);
    let mut items = preflight_items
        .into_iter()
        .map(|item| (item.setting_id.clone(), item))
        .collect::<BTreeMap<_, _>>();
    for pending in pending_items {
        let result = results
            .iter()
            .find(|result| result.setting_id == pending.public.setting_id);
        let succeeded = result.is_some_and(|result| {
            result.verified && result.status != SystemSettingChangeOutcomeStatus::Failed
        });
        match pending.public.target {
            SystemSettingTargetState::Default if succeeded => {
                items.remove(&pending.public.setting_id);
            }
            SystemSettingTargetState::Optimized
                if (!succeeded && !possibly_changed.contains(&pending.public.setting_id))
                    || (result.is_some_and(|result| {
                        result.status == SystemSettingChangeOutcomeStatus::Unchanged
                    }) && !previous_items.contains_key(&pending.public.setting_id)) =>
            {
                if let Some(previous) = previous_items.get(&pending.public.setting_id) {
                    items.insert(pending.public.setting_id.clone(), previous.clone());
                } else {
                    items.remove(&pending.public.setting_id);
                }
            }
            _ => {}
        }
    }
    items.into_values().collect()
}

fn write_atomic(path: &Path, content: &[u8]) -> CoreResult<()> {
    let temporary = path.with_file_name("system-settings-recovery.json.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temporary)
        .map_err(|error| {
            CoreError::persistence(format!(
                "failed to create system settings recovery temporary file: {error}"
            ))
        })?;
    file.write_all(content)
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            CoreError::persistence(format!(
                "failed to write system settings recovery temporary file: {error}"
            ))
        })?;
    replace_file(&temporary, path).map_err(|error| {
        CoreError::persistence(format!("failed to save system settings recovery: {error}"))
    })
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let succeeded = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if succeeded == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn load_recovery() -> CoreResult<Option<RecoveryDocument>> {
    let path = recovery_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let metadata = fs::metadata(&path).map_err(|error| {
        CoreError::persistence(format!("failed to inspect recovery record: {error}"))
    })?;
    if metadata.len() > RECOVERY_MAX_BYTES {
        log::warn!(
            "system_settings_recovery_quarantined reason=oversized byte_count={}",
            metadata.len()
        );
        quarantine_invalid_recovery(&path)?;
        return Ok(None);
    }
    let content = fs::read(&path).map_err(|error| {
        CoreError::persistence(format!("failed to read recovery record: {error}"))
    })?;
    let document = match serde_json::from_slice::<RecoveryDocument>(&content) {
        Ok(document) => document,
        Err(error) => {
            log::warn!(
                "system_settings_recovery_quarantined reason=invalid_format error_digest={}",
                blake3::hash(error.to_string().as_bytes()).to_hex()
            );
            quarantine_invalid_recovery(&path)?;
            return Ok(None);
        }
    };
    let unique_ids = document
        .items
        .iter()
        .map(|item| item.setting_id.as_str())
        .collect::<BTreeSet<_>>();
    let invalid_item = document.items.iter().any(|item| {
        item.setting_id.is_empty()
            || item.setting_id.len() > 128
            || !(item.setting_id.starts_with("macos.") || item.setting_id.starts_with("windows."))
            || recovery_value_is_too_large(&item.original_value)
            || recovery_value_is_too_large(&item.optimized_value)
    });
    if !(MIN_SUPPORTED_RECOVERY_SCHEMA_VERSION..=RECOVERY_SCHEMA_VERSION)
        .contains(&document.schema_version)
        || document.items.len() > MAX_RECOVERY_ITEMS
        || unique_ids.len() != document.items.len()
        || invalid_item
    {
        log::warn!(
            "system_settings_recovery_quarantined reason=unsupported_schema schema_version={} item_count={}",
            document.schema_version,
            document.items.len()
        );
        quarantine_invalid_recovery(&path)?;
        return Ok(None);
    }
    Ok(Some(document))
}

fn recovery_value_is_too_large(value: &PlatformSystemSettingValue) -> bool {
    match value {
        PlatformSystemSettingValue::Text(value)
        | PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::Text(value)) => {
            value.len() > RECOVERY_MAX_TEXT_BYTES
        }
        PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::IntegerMap(values)) => {
            values.len() > MAX_RECOVERY_ITEMS
                || values.iter().any(|(key, value)| {
                    key.is_empty()
                        || key.len() > 256
                        || value.is_some_and(|value| !(0..=i64::from(u32::MAX)).contains(&value))
                })
        }
        _ => false,
    }
}

fn quarantine_invalid_recovery(path: &Path) -> CoreResult<()> {
    let quarantine = path.with_file_name(INVALID_RECOVERY_FILE_NAME);
    replace_file(path, &quarantine).map_err(|error| {
        CoreError::persistence(format!(
            "failed to quarantine unsupported system settings recovery: {error}"
        ))
    })
}

fn remove_recovery() -> CoreResult<()> {
    let path = recovery_path()?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| {
            CoreError::persistence(format!("failed to remove recovery record: {error}"))
        })?;
    }
    Ok(())
}

fn log_catalog(operation_id: u64, catalog: &SystemSettingsCatalog) {
    let restorable_count = catalog
        .items
        .iter()
        .filter(|item| item.restore_available)
        .count();
    log::info!(
        "system_settings_catalog_scanned operation_id={} platform={:?} item_count={} recommended_count={} selected_count={} restorable_count={} unavailable_count={} elapsed_ms={}",
        operation_id,
        catalog.platform,
        catalog.summary.item_count,
        catalog.summary.recommended_count,
        catalog.summary.selected_count,
        restorable_count,
        catalog.summary.unavailable_count,
        catalog.elapsed_ms
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_item(
        setting_id: &str,
        target: SystemSettingTargetState,
        expected: i64,
        desired: i64,
    ) -> PendingChangeItem {
        PendingChangeItem {
            public: SystemSettingChangePlanItem {
                setting_id: setting_id.to_string(),
                category: super::super::SystemSettingCategory::Performance,
                target,
                requires_restart: false,
                requires_elevation: false,
            },
            expected_value: PlatformSystemSettingValue::Integer(expected),
            desired_value: PlatformSystemSettingValue::Integer(desired),
        }
    }

    fn recovery_document(items: Vec<RecoveryItem>) -> RecoveryDocument {
        RecoveryDocument {
            schema_version: RECOVERY_SCHEMA_VERSION,
            recovery_id: "recovery".to_string(),
            created_at_ms: 1,
            items,
        }
    }

    fn recovery_item(setting_id: &str, original: i64, optimized: i64) -> RecoveryItem {
        RecoveryItem {
            setting_id: setting_id.to_string(),
            original_value: PlatformSystemSettingValue::Integer(original),
            optimized_value: PlatformSystemSettingValue::Integer(optimized),
        }
    }

    fn native_fixture(
        setting_id: &str,
        status: SystemSettingStatus,
        restore_available: bool,
        diagnostic: Option<PlatformSystemSettingDiagnosticCode>,
        requires_restart: bool,
    ) -> CatalogNativeItem {
        let optimized = status == SystemSettingStatus::Optimized;
        let value = i64::from(optimized);
        CatalogNativeItem {
            public: SystemSettingItem {
                setting_id: setting_id.to_string(),
                category: super::super::SystemSettingCategory::Performance,
                selection_kind: SystemSettingSelectionKind::Custom,
                risk_level: super::super::SystemSettingRiskLevel::Standard,
                status,
                selected_by_default: false,
                restore_available,
                requires_restart,
                requires_elevation: false,
                diagnostic,
            },
            current_value: PlatformSystemSettingValue::Integer(value),
            effective_value: PlatformSystemSettingValue::Integer(value),
            disabled_value: PlatformSystemSettingValue::Integer(0),
            recommended_value: PlatformSystemSettingValue::Integer(1),
        }
    }

    fn catalog_session_fixture(native_items: Vec<CatalogNativeItem>) -> CatalogSession {
        let items = native_items
            .iter()
            .map(|item| item.public.clone())
            .collect::<Vec<_>>();
        CatalogSession {
            public: SystemSettingsCatalog {
                schema_version: SYSTEM_SETTINGS_CATALOG_SCHEMA_VERSION,
                scan_id: "scan-plan-fixture".to_string(),
                catalog_revision: catalog_revision(&native_items),
                platform: current_platform_kind(),
                scanned_at_ms: 1,
                summary: summary(&items),
                items,
                elapsed_ms: 1,
                recovery_available: false,
            },
            native_items,
        }
    }

    #[test]
    fn shortcut_overlay_restore_uses_the_durable_native_baseline() {
        let id = "windows.explorer.hide-shortcut-arrows";
        let mut item = native_fixture(id, SystemSettingStatus::Optimized, true, None, false);
        item.current_value = PlatformSystemSettingValue::Snapshot(
            PlatformSystemSettingSnapshot::Text("owned-icon".into()),
        );
        item.effective_value = PlatformSystemSettingValue::Boolean(true);
        item.recommended_value = PlatformSystemSettingValue::Boolean(true);
        item.disabled_value = PlatformSystemSettingValue::Boolean(false);
        let recovery = RecoveryItem {
            setting_id: id.into(),
            original_value: PlatformSystemSettingValue::Missing,
            optimized_value: PlatformSystemSettingValue::Boolean(true),
        };
        assert_eq!(
            desired_value_for_target(&item, SystemSettingTargetState::Default, Some(&recovery)),
            Some(PlatformSystemSettingValue::Missing)
        );
        assert_eq!(
            desired_value_for_target(&item, SystemSettingTargetState::Default, None),
            None
        );
    }

    #[test]
    fn platform_inventory_rejects_ambiguous_identifiers_and_marks_missing_states() {
        let platform = current_platform_kind();
        let definition = definitions(platform)[0];
        let state = mangodisk_platform::PlatformSystemSettingState {
            setting_id: definition.id.to_string(),
            value: definition.default_value.owned(),
            effective_value: definition.default_value.owned(),
            requires_elevation: definition.requires_elevation,
            diagnostic: None,
        };

        let items = catalog_native_items(platform, vec![state.clone()])
            .expect("a partial native inventory must remain inspectable");
        assert_eq!(items.len(), definitions(platform).len());
        assert_eq!(items[0].public.setting_id, definition.id);
        assert!(items
            .iter()
            .skip(1)
            .all(|item| item.public.status == SystemSettingStatus::Unavailable));

        assert_eq!(
            catalog_native_items(platform, vec![state.clone(), state])
                .err()
                .expect("duplicate states must fail closed")
                .code(),
            crate::shared::CoreErrorCode::Platform
        );
        let unknown = mangodisk_platform::PlatformSystemSettingState {
            setting_id: "system-setting.unknown".to_string(),
            value: PlatformSystemSettingValue::Missing,
            effective_value: PlatformSystemSettingValue::Missing,
            requires_elevation: false,
            diagnostic: None,
        };
        assert_eq!(
            catalog_native_items(platform, vec![unknown])
                .err()
                .expect("unknown states must fail closed")
                .code(),
            crate::shared::CoreErrorCode::Platform
        );
    }

    #[test]
    fn missing_key_repeat_preferences_remain_system_owned_and_recommended() {
        let definition = definitions(SystemSettingsPlatform::Macos)
            .iter()
            .find(|definition| definition.id == "macos.keyboard.fast-key-repeat")
            .expect("the key repeat preference should exist");
        let state = mangodisk_platform::PlatformSystemSettingState {
            setting_id: definition.id.to_string(),
            value: PlatformSystemSettingValue::Missing,
            effective_value: PlatformSystemSettingValue::Missing,
            requires_elevation: false,
            diagnostic: None,
        };

        let item = native_item(definition, Some(&state));

        assert_eq!(item.public.status, SystemSettingStatus::Recommended);
        assert_eq!(item.current_value, PlatformSystemSettingValue::Missing);
        assert_eq!(item.effective_value, PlatformSystemSettingValue::Missing);
        assert_eq!(item.disabled_value, PlatformSystemSettingValue::Missing);
        assert_eq!(
            item.recommended_value,
            PlatformSystemSettingValue::Integer(2)
        );
    }

    #[test]
    fn legacy_recommendation_remains_restorable_after_catalog_changes() {
        let recovery = recovery_document(vec![RecoveryItem {
            setting_id: "macos.keyboard.fast-key-repeat".to_string(),
            original_value: PlatformSystemSettingValue::Missing,
            optimized_value: PlatformSystemSettingValue::Integer(1),
        }]);
        let mut native_items = vec![CatalogNativeItem {
            public: SystemSettingItem {
                setting_id: "macos.keyboard.fast-key-repeat".to_string(),
                category: super::super::SystemSettingCategory::Performance,
                selection_kind: SystemSettingSelectionKind::Custom,
                risk_level: super::super::SystemSettingRiskLevel::Standard,
                status: SystemSettingStatus::Recommended,
                selected_by_default: false,
                restore_available: false,
                requires_restart: false,
                requires_elevation: false,
                diagnostic: None,
            },
            current_value: PlatformSystemSettingValue::Integer(1),
            effective_value: PlatformSystemSettingValue::Integer(1),
            disabled_value: PlatformSystemSettingValue::Missing,
            recommended_value: PlatformSystemSettingValue::Integer(2),
        }];
        assert!(mark_restore_availability(
            &mut native_items,
            Some(&recovery)
        ));
        let session = catalog_session_fixture(native_items);
        let selection = SystemSettingsChangeSelection {
            scan_id: session.public.scan_id.clone(),
            items: vec![super::super::SystemSettingChangeSelectionItem {
                setting_id: "macos.keyboard.fast-key-repeat".to_string(),
                target: SystemSettingTargetState::Default,
            }],
        };

        let pending = build_change_plan(selection, session, Some(&recovery), 5_000);

        assert!(pending.public.skipped_items.is_empty());
        assert_eq!(pending.items.len(), 1);
        assert_eq!(
            pending.items[0].desired_value,
            PlatformSystemSettingValue::Missing
        );
    }

    #[test]
    fn mixed_change_plan_preserves_recovery_and_reports_every_skip_reason() {
        let session = catalog_session_fixture(vec![
            native_fixture(
                "enable",
                SystemSettingStatus::Recommended,
                false,
                None,
                true,
            ),
            native_fixture("restore", SystemSettingStatus::Optimized, true, None, false),
            native_fixture(
                "unsupported",
                SystemSettingStatus::Unavailable,
                false,
                Some(PlatformSystemSettingDiagnosticCode::Unsupported),
                false,
            ),
            native_fixture(
                "already-default",
                SystemSettingStatus::Recommended,
                false,
                None,
                false,
            ),
            native_fixture(
                "already-optimized",
                SystemSettingStatus::Optimized,
                false,
                None,
                false,
            ),
            native_fixture(
                "stale-restore",
                SystemSettingStatus::Optimized,
                true,
                None,
                false,
            ),
        ]);
        let selection_item =
            |setting_id: &str, target| super::super::SystemSettingChangeSelectionItem {
                setting_id: setting_id.to_string(),
                target,
            };
        let selection = SystemSettingsChangeSelection {
            scan_id: session.public.scan_id.clone(),
            items: vec![
                selection_item("enable", SystemSettingTargetState::Optimized),
                selection_item("restore", SystemSettingTargetState::Default),
                selection_item("unsupported", SystemSettingTargetState::Optimized),
                selection_item("already-default", SystemSettingTargetState::Default),
                selection_item("already-optimized", SystemSettingTargetState::Optimized),
                selection_item("stale-restore", SystemSettingTargetState::Default),
                selection_item("missing", SystemSettingTargetState::Optimized),
            ],
        };
        let recovery = recovery_document(vec![recovery_item("restore", 0, 1)]);

        let pending = build_change_plan(selection, session, Some(&recovery), 5_000);

        assert_eq!(pending.public.created_at_ms, 5_000);
        assert_eq!(pending.public.expires_at_ms, 5_000 + CHANGE_PLAN_TTL_MS);
        assert!(pending.public.requires_confirmation);
        assert!(pending.public.requires_restart);
        assert_eq!(pending.items.len(), 2);
        assert_eq!(pending.items[0].public.setting_id, "enable");
        assert_eq!(
            pending.items[0].desired_value,
            PlatformSystemSettingValue::Integer(1)
        );
        assert_eq!(pending.items[1].public.setting_id, "restore");
        assert_eq!(
            pending.items[1].desired_value,
            PlatformSystemSettingValue::Integer(0)
        );
        assert_eq!(
            pending
                .public
                .skipped_items
                .iter()
                .map(|item| (item.setting_id.as_str(), item.reason))
                .collect::<Vec<_>>(),
            vec![
                ("unsupported", SystemSettingChangeSkipReason::Unsupported),
                (
                    "already-default",
                    SystemSettingChangeSkipReason::AlreadyDefault
                ),
                (
                    "already-optimized",
                    SystemSettingChangeSkipReason::AlreadyOptimized
                ),
                (
                    "stale-restore",
                    SystemSettingChangeSkipReason::SettingChanged
                ),
                ("missing", SystemSettingChangeSkipReason::SettingMissing),
            ]
        );
    }

    #[test]
    fn summary_separates_recommended_and_default_selected_items() {
        let items = vec![
            SystemSettingItem {
                setting_id: "one".to_string(),
                category: super::super::SystemSettingCategory::Privacy,
                selection_kind: SystemSettingSelectionKind::OneClick,
                risk_level: super::super::SystemSettingRiskLevel::Standard,
                status: SystemSettingStatus::Recommended,
                selected_by_default: true,
                restore_available: false,
                requires_restart: false,
                requires_elevation: false,
                diagnostic: None,
            },
            SystemSettingItem {
                setting_id: "two".to_string(),
                category: super::super::SystemSettingCategory::Appearance,
                selection_kind: SystemSettingSelectionKind::Custom,
                risk_level: super::super::SystemSettingRiskLevel::Standard,
                status: SystemSettingStatus::Recommended,
                selected_by_default: false,
                restore_available: false,
                requires_restart: false,
                requires_elevation: false,
                diagnostic: None,
            },
        ];
        let summary = summary(&items);
        assert_eq!(summary.recommended_count, 2);
        assert_eq!(summary.selected_count, 1);
    }

    #[test]
    fn native_capability_mismatches_fail_closed() {
        let definition = SettingDefinition {
            id: "windows.test.setting",
            category: super::super::SystemSettingCategory::Privacy,
            selection_kind: SystemSettingSelectionKind::Custom,
            risk_level: super::super::SystemSettingRiskLevel::Standard,
            default_value: super::super::catalog::DefinitionValue::Integer(0),
            disabled_value: None,
            recommended_value: super::super::catalog::DefinitionValue::Integer(1),
            requires_restart: false,
            requires_elevation: false,
        };
        let state = mangodisk_platform::PlatformSystemSettingState {
            setting_id: definition.id.to_string(),
            value: PlatformSystemSettingValue::Integer(0),
            effective_value: PlatformSystemSettingValue::Integer(0),
            requires_elevation: true,
            diagnostic: None,
        };

        let item = native_item(&definition, Some(&state));

        assert_eq!(item.public.status, SystemSettingStatus::Unavailable);
        assert_eq!(
            item.public.diagnostic,
            Some(PlatformSystemSettingDiagnosticCode::InvalidData)
        );
        assert!(item.public.requires_elevation);
    }

    #[test]
    fn restart_notice_requires_a_changed_item_that_needs_restart() {
        let pending_items = vec![
            PendingChangeItem {
                public: SystemSettingChangePlanItem {
                    setting_id: "restart".to_string(),
                    category: super::super::SystemSettingCategory::Productivity,
                    target: SystemSettingTargetState::Optimized,
                    requires_restart: true,
                    requires_elevation: false,
                },
                expected_value: PlatformSystemSettingValue::Integer(0),
                desired_value: PlatformSystemSettingValue::Integer(1),
            },
            PendingChangeItem {
                public: SystemSettingChangePlanItem {
                    setting_id: "immediate".to_string(),
                    category: super::super::SystemSettingCategory::Appearance,
                    target: SystemSettingTargetState::Optimized,
                    requires_restart: false,
                    requires_elevation: false,
                },
                expected_value: PlatformSystemSettingValue::Integer(0),
                desired_value: PlatformSystemSettingValue::Integer(1),
            },
        ];
        let failed_restart = vec![
            change_result(
                "restart".to_string(),
                SystemSettingChangeOutcomeStatus::Failed,
                false,
                Some(SystemSettingChangeFailureReason::PlatformFailure),
            ),
            change_result(
                "immediate".to_string(),
                SystemSettingChangeOutcomeStatus::Changed,
                true,
                None,
            ),
        ];
        assert!(!changed_items_require_restart(
            &pending_items,
            &failed_restart
        ));

        let changed_restart = vec![change_result(
            "restart".to_string(),
            SystemSettingChangeOutcomeStatus::Changed,
            true,
            None,
        )];
        assert!(changed_items_require_restart(
            &pending_items,
            &changed_restart
        ));
    }

    #[test]
    fn user_cancelled_platform_errors_preserve_cancellation_semantics() {
        assert_eq!(
            failure_reason(PlatformErrorCode::UserCancelled),
            SystemSettingChangeFailureReason::UserCancelled
        );
    }

    #[test]
    fn recovery_intent_preserves_prior_baselines_across_batches() {
        let previous = recovery_document(vec![recovery_item("existing", 0, 1)]);
        let pending = vec![pending_item(
            "new",
            SystemSettingTargetState::Optimized,
            4,
            8,
        )];

        assert_eq!(
            recovery_intent(Some(&previous), &pending),
            vec![recovery_item("existing", 0, 1), recovery_item("new", 4, 8)]
        );
    }

    #[test]
    fn recovery_baseline_must_match_the_current_native_value_and_value_kind() {
        let item = CatalogNativeItem {
            public: SystemSettingItem {
                setting_id: "windows.test.setting".to_string(),
                category: super::super::SystemSettingCategory::Privacy,
                selection_kind: SystemSettingSelectionKind::Custom,
                risk_level: super::super::SystemSettingRiskLevel::Standard,
                status: SystemSettingStatus::Optimized,
                selected_by_default: false,
                restore_available: false,
                requires_restart: false,
                requires_elevation: false,
                diagnostic: None,
            },
            current_value: PlatformSystemSettingValue::Integer(1),
            effective_value: PlatformSystemSettingValue::Integer(1),
            disabled_value: PlatformSystemSettingValue::Integer(0),
            recommended_value: PlatformSystemSettingValue::Integer(1),
        };

        assert!(valid_recovery_baseline(
            &recovery_item(item.public.setting_id.as_str(), 0, 1),
            &item
        ));
        assert!(!valid_recovery_baseline(
            &recovery_item(item.public.setting_id.as_str(), 0, 2),
            &item
        ));
        assert!(!valid_recovery_baseline(
            &RecoveryItem {
                setting_id: item.public.setting_id.clone(),
                original_value: PlatformSystemSettingValue::Text("invalid".to_string()),
                optimized_value: PlatformSystemSettingValue::Integer(1),
            },
            &item
        ));

        let snapshot =
            PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::IntegerMap(
                BTreeMap::from([("native".to_string(), Some(0))]),
            ));
        let snapshot_recovery = RecoveryItem {
            setting_id: item.public.setting_id.clone(),
            original_value: snapshot.clone(),
            optimized_value: PlatformSystemSettingValue::Integer(1),
        };
        assert!(!valid_recovery_baseline(&snapshot_recovery, &item));

        let snapshot_item = CatalogNativeItem {
            current_value: snapshot,
            ..item
        };
        assert!(valid_recovery_baseline(&snapshot_recovery, &snapshot_item));
    }

    #[test]
    fn recovery_availability_requires_a_matching_durable_baseline() {
        let mut items = vec![CatalogNativeItem {
            public: SystemSettingItem {
                setting_id: "windows.test.setting".to_string(),
                category: super::super::SystemSettingCategory::Performance,
                selection_kind: SystemSettingSelectionKind::Custom,
                risk_level: super::super::SystemSettingRiskLevel::Standard,
                status: SystemSettingStatus::Optimized,
                selected_by_default: false,
                restore_available: false,
                requires_restart: false,
                requires_elevation: false,
                diagnostic: None,
            },
            current_value: PlatformSystemSettingValue::Integer(1),
            effective_value: PlatformSystemSettingValue::Integer(1),
            disabled_value: PlatformSystemSettingValue::Integer(0),
            recommended_value: PlatformSystemSettingValue::Integer(1),
        }];
        let matching = recovery_document(vec![recovery_item("windows.test.setting", 0, 1)]);

        assert!(mark_restore_availability(&mut items, Some(&matching)));
        assert!(items[0].public.restore_available);

        items[0].public.status = SystemSettingStatus::Recommended;
        items[0].recommended_value = PlatformSystemSettingValue::Integer(2);
        assert!(mark_restore_availability(&mut items, Some(&matching)));
        assert!(items[0].public.restore_available);

        let stale = recovery_document(vec![recovery_item("windows.test.setting", 0, 2)]);
        assert!(!mark_restore_availability(&mut items, Some(&stale)));
        assert!(!items[0].public.restore_available);

        items[0].public.status = SystemSettingStatus::Unavailable;
        assert!(!mark_restore_availability(&mut items, Some(&matching)));
        assert!(!items[0].public.restore_available);
    }

    #[test]
    fn explicit_recovery_never_falls_back_to_the_catalog_default() {
        let mut item = CatalogNativeItem {
            public: SystemSettingItem {
                setting_id: "windows.test.setting".to_string(),
                category: super::super::SystemSettingCategory::Performance,
                selection_kind: SystemSettingSelectionKind::Custom,
                risk_level: super::super::SystemSettingRiskLevel::Standard,
                status: SystemSettingStatus::Optimized,
                selected_by_default: false,
                restore_available: true,
                requires_restart: false,
                requires_elevation: false,
                diagnostic: None,
            },
            current_value: PlatformSystemSettingValue::Integer(1),
            effective_value: PlatformSystemSettingValue::Integer(1),
            disabled_value: PlatformSystemSettingValue::Integer(0),
            recommended_value: PlatformSystemSettingValue::Integer(1),
        };

        assert_eq!(
            desired_value_for_target(&item, SystemSettingTargetState::Default, None),
            None
        );

        item.public.restore_available = false;
        assert_eq!(
            desired_value_for_target(&item, SystemSettingTargetState::Default, None),
            Some(PlatformSystemSettingValue::Integer(0))
        );
    }

    #[test]
    fn successful_restore_removes_only_its_baseline() {
        let previous = recovery_document(vec![
            recovery_item("keep", 0, 1),
            recovery_item("restore", 2, 3),
        ]);
        let pending = vec![pending_item(
            "restore",
            SystemSettingTargetState::Default,
            3,
            2,
        )];
        let results = vec![change_result(
            "restore".to_string(),
            SystemSettingChangeOutcomeStatus::Changed,
            true,
            None,
        )];
        let preflight = recovery_intent(Some(&previous), &pending);

        assert_eq!(
            reconciled_recovery_items(
                Some(&previous),
                preflight,
                &pending,
                &results,
                &BTreeSet::new(),
            ),
            vec![recovery_item("keep", 0, 1)]
        );
    }

    #[test]
    fn failed_enable_does_not_create_a_false_recovery_baseline() {
        let pending = vec![pending_item(
            "failed",
            SystemSettingTargetState::Optimized,
            0,
            1,
        )];
        let results = vec![change_result(
            "failed".to_string(),
            SystemSettingChangeOutcomeStatus::Failed,
            false,
            Some(SystemSettingChangeFailureReason::PlatformFailure),
        )];
        let preflight = recovery_intent(None, &pending);

        assert!(
            reconciled_recovery_items(None, preflight, &pending, &results, &BTreeSet::new(),)
                .is_empty()
        );
    }

    #[test]
    fn uncertain_enable_failure_retains_the_preflight_recovery_baseline() {
        let pending = vec![pending_item(
            "uncertain",
            SystemSettingTargetState::Optimized,
            0,
            1,
        )];
        let results = vec![change_result(
            "uncertain".to_string(),
            SystemSettingChangeOutcomeStatus::Failed,
            false,
            Some(SystemSettingChangeFailureReason::VerificationFailed),
        )];
        let preflight = recovery_intent(None, &pending);
        let possibly_changed = BTreeSet::from(["uncertain".to_string()]);

        assert_eq!(
            reconciled_recovery_items(None, preflight, &pending, &results, &possibly_changed,),
            vec![recovery_item("uncertain", 0, 1)]
        );
    }

    #[test]
    fn selection_rejects_duplicate_setting_identifiers() {
        let selection = SystemSettingsChangeSelection {
            scan_id: "scan".to_string(),
            items: vec![
                super::super::SystemSettingChangeSelectionItem {
                    setting_id: "duplicate".to_string(),
                    target: SystemSettingTargetState::Optimized,
                },
                super::super::SystemSettingChangeSelectionItem {
                    setting_id: "duplicate".to_string(),
                    target: SystemSettingTargetState::Default,
                },
            ],
        };

        assert!(validate_selection(&selection).is_err());
    }

    #[test]
    fn selection_accepts_the_complete_current_windows_catalog_size() {
        let item_count = definitions(SystemSettingsPlatform::Windows).len();
        let selection = SystemSettingsChangeSelection {
            scan_id: "scan".to_string(),
            items: (0..item_count)
                .map(|index| super::super::SystemSettingChangeSelectionItem {
                    setting_id: format!("windows.test.{index}"),
                    target: SystemSettingTargetState::Optimized,
                })
                .collect(),
        };

        assert!(item_count > 128, "the regression requires a large catalog");
        assert!(MAX_CHANGE_ITEMS >= item_count);
        assert!(MAX_RECOVERY_ITEMS >= item_count);
        assert!(validate_selection(&selection).is_ok());
    }

    #[test]
    fn invalid_recovery_is_quarantined_instead_of_deleted() {
        let directory = std::env::temp_dir().join(format!(
            "mangodisk-system-settings-recovery-quarantine-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&directory).expect("the recovery fixture directory must be created");
        let recovery = directory.join(RECOVERY_FILE_NAME);
        let quarantine = directory.join(INVALID_RECOVERY_FILE_NAME);
        fs::write(&recovery, b"invalid recovery")
            .expect("the invalid recovery fixture must be written");

        quarantine_invalid_recovery(&recovery).expect("invalid recovery must be quarantined");

        assert!(!recovery.exists());
        assert_eq!(
            fs::read(&quarantine).expect("the quarantine must remain readable"),
            b"invalid recovery"
        );
        fs::remove_dir_all(directory).expect("the recovery fixture directory must be removed");
    }
}
