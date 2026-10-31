use crate::filesystem::atomic_replace::replace_file;

use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::{
    history::{
        DeepCleanupOperationDetails, FileCleanupHistoryItem, FileCleanupHistoryItemStatus,
        FileCleanupOperationDetails, OperationCategory, OperationDetails, OperationOutcome,
        OperationRecord, OPERATION_RECORD_SCHEMA_VERSION,
    },
    shared::application_paths,
    CoreError, CoreResult,
};

static HISTORY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const HISTORY_DOCUMENT_SCHEMA_VERSION: u32 = 3;
const FILE_CLEANUP_HISTORY_ITEM_LIMIT: usize = 200;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HistoryDocument {
    schema_version: u32,
    records: Vec<OperationRecord>,
}

pub struct HistoryService;

#[derive(Debug, Clone, Copy)]
pub(crate) enum FileCleanupHistoryCategory {
    LargeFiles,
    DuplicateFiles,
}

impl HistoryService {
    pub fn list() -> CoreResult<Vec<OperationRecord>> {
        let _guard = history_lock()
            .lock()
            .map_err(|_| CoreError::persistence("cleanup history lock is poisoned"))?;
        let _process_guard = acquire_history_process_lock()?;
        read_records()
    }

    pub fn append(record: OperationRecord) -> CoreResult<()> {
        if record.schema_version != OPERATION_RECORD_SCHEMA_VERSION {
            return Err(CoreError::persistence(
                "new history records must use the current operation schema",
            ));
        }
        validate_operation_record(&record)?;
        let _guard = history_lock()
            .lock()
            .map_err(|_| CoreError::persistence("cleanup history lock is poisoned"))?;
        let _process_guard = acquire_history_process_lock()?;
        let mut records = read_records()?;
        records.insert(0, record);
        records.truncate(100);
        let content = serde_json::to_vec_pretty(&HistoryDocument {
            schema_version: HISTORY_DOCUMENT_SCHEMA_VERSION,
            records,
        })
        .map_err(|error| {
            CoreError::persistence(format!("failed to serialize cleanup history: {error}"))
        })?;
        write_atomic(&history_path()?, &content)
    }

    /// Saves one deep-cleanup step and merges it with an earlier step from the
    /// same user confirmation. Cleanup rules and application leftovers execute
    /// through separate safety boundaries, but history must remain aligned with
    /// the single product operation the user initiated.
    pub fn upsert_deep_cleanup(record: OperationRecord) -> CoreResult<()> {
        if record.schema_version != OPERATION_RECORD_SCHEMA_VERSION {
            return Err(CoreError::persistence(
                "new history records must use the current operation schema",
            ));
        }
        validate_operation_record(&record)?;
        if record.category != OperationCategory::DeepCleanup {
            return Err(CoreError::persistence(
                "only deep-cleanup records can be merged",
            ));
        }

        let _guard = history_lock()
            .lock()
            .map_err(|_| CoreError::persistence("cleanup history lock is poisoned"))?;
        let _process_guard = acquire_history_process_lock()?;
        let mut records = read_records()?;
        if let Some(index) = records
            .iter()
            .position(|current| current.operation_id == record.operation_id)
        {
            let current = records.remove(index);
            records.insert(0, merge_deep_cleanup_records(current, record)?);
        } else {
            records.insert(0, record);
        }
        records.truncate(100);
        save_records(records)
    }

    pub fn clear() -> CoreResult<()> {
        let _guard = history_lock()
            .lock()
            .map_err(|_| CoreError::persistence("cleanup history lock is poisoned"))?;
        let _process_guard = acquire_history_process_lock()?;
        let path = history_path()?;
        if path.exists() {
            fs::remove_file(path).map_err(|error| {
                CoreError::persistence(format!("failed to clear cleanup history: {error}"))
            })?;
        }
        Ok(())
    }
}

fn history_lock() -> &'static Mutex<()> {
    HISTORY_LOCK.get_or_init(|| Mutex::new(()))
}

fn acquire_history_process_lock() -> CoreResult<fs::File> {
    let directory = application_paths()?.runtime_directory();
    fs::create_dir_all(directory).map_err(|error| {
        CoreError::persistence(format!(
            "failed to create the application runtime directory: {error}"
        ))
    })?;
    let path = directory.join("history.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            CoreError::persistence(format!("failed to open the history lock: {error}"))
        })?;
    file.lock_exclusive().map_err(|error| {
        CoreError::persistence(format!("failed to acquire the history lock: {error}"))
    })?;
    Ok(file)
}

fn read_records() -> CoreResult<Vec<OperationRecord>> {
    let path = history_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path).map_err(|error| {
        CoreError::persistence(format!(
            "failed to read cleanup history path={}: {error}",
            crate::diagnostic_path(&path)
        ))
    })?;
    let document = match serde_json::from_str::<HistoryDocument>(&content) {
        Ok(document) => document,
        Err(error) => {
            log::warn!(
                "history_quarantine reason=invalid_format path={} error={}",
                crate::diagnostic_path(&path),
                mangodisk_platform::diagnostics::text(&error)
            );
            quarantine_invalid_history(&path)?;
            return Ok(Vec::new());
        }
    };
    match validate_history_document(document) {
        Ok(records) => Ok(records),
        Err(error) => {
            log::warn!(
                "history_quarantine reason=unsupported_schema path={} error={}",
                crate::diagnostic_path(&path),
                mangodisk_platform::diagnostics::text(&error)
            );
            quarantine_invalid_history(&path)?;
            Ok(Vec::new())
        }
    }
}

fn validate_history_document(document: HistoryDocument) -> CoreResult<Vec<OperationRecord>> {
    if document.schema_version != HISTORY_DOCUMENT_SCHEMA_VERSION {
        return Err(CoreError::persistence(format!(
            "unsupported cleanup history schema version: {}",
            document.schema_version
        )));
    }
    for record in &document.records {
        validate_operation_record(record)?;
    }
    Ok(document.records)
}

fn validate_operation_record(record: &OperationRecord) -> CoreResult<()> {
    if record.schema_version != OPERATION_RECORD_SCHEMA_VERSION {
        return Err(CoreError::persistence(
            "cleanup history contains an unsupported operation record",
        ));
    }
    let consistent = matches!(
        (record.category, &record.details),
        (
            OperationCategory::DeepCleanup,
            OperationDetails::DeepCleanup(_),
        ) | (
            OperationCategory::LargeFileCleanup,
            OperationDetails::LargeFileCleanup(_),
        ) | (
            OperationCategory::DuplicateFileCleanup,
            OperationDetails::DuplicateFileCleanup(_),
        ) | (
            OperationCategory::ApplicationUninstall,
            OperationDetails::ApplicationUninstall(_),
        ) | (
            OperationCategory::StartupManagement,
            OperationDetails::StartupManagement(_),
        ) | (
            OperationCategory::SystemOptimization,
            OperationDetails::SystemOptimization(_),
        ) | (
            OperationCategory::PrivacyCleanup,
            OperationDetails::PrivacyCleanup(_),
        )
    );
    if !consistent {
        return Err(CoreError::persistence(
            "cleanup history contains inconsistent operation metadata",
        ));
    }
    if record.released_bytes_is_estimate && record.released_bytes.is_some() {
        return Err(CoreError::persistence(
            "estimated history records must not claim measured released bytes",
        ));
    }
    if let OperationDetails::LargeFileCleanup(details)
    | OperationDetails::DuplicateFileCleanup(details) = &record.details
    {
        if details.items.len() > FILE_CLEANUP_HISTORY_ITEM_LIMIT {
            return Err(CoreError::persistence(
                "file cleanup history exceeds the detail item limit",
            ));
        }
        if record.selected_item_count
            != (details.items.len() as u64).saturating_add(details.omitted_item_count)
        {
            return Err(CoreError::persistence(
                "file cleanup history item counts are inconsistent",
            ));
        }
    }
    if let OperationDetails::ApplicationUninstall(details) = &record.details {
        let application_ids = details
            .applications
            .iter()
            .map(|application| application.application_id.as_str())
            .collect::<HashSet<_>>();
        if details.applications.is_empty()
            || record.selected_item_count != details.applications.len() as u64
            || application_ids.len() != details.applications.len()
        {
            return Err(CoreError::persistence(
                "application uninstall history batch is inconsistent",
            ));
        }
    }
    if let OperationDetails::StartupManagement(details) = &record.details {
        let item_ids = details
            .items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<HashSet<_>>();
        if details.items.is_empty()
            || record.selected_item_count != details.items.len() as u64
            || item_ids.len() != details.items.len()
            || record.expected_bytes != 0
            || record.released_bytes.is_some()
        {
            return Err(CoreError::persistence(
                "startup management history is inconsistent",
            ));
        }
    }
    if let OperationDetails::SystemOptimization(details) = &record.details {
        let setting_ids = details
            .items
            .iter()
            .map(|item| item.setting_id.as_str())
            .collect::<HashSet<_>>();
        if details.items.is_empty()
            || record.selected_item_count != details.items.len() as u64
            || setting_ids.len() != details.items.len()
            || record.expected_bytes != 0
            || record.released_bytes.is_some()
        {
            return Err(CoreError::persistence(
                "system optimization history is inconsistent",
            ));
        }
    }
    if let OperationDetails::PrivacyCleanup(details) = &record.details {
        if details.items.is_empty()
            || record.selected_item_count != details.items.len() as u64
            || details.plan_id.is_empty()
            || record.expected_bytes != 0
            || record.released_bytes.is_some()
        {
            return Err(CoreError::persistence(
                "privacy cleanup history is inconsistent",
            ));
        }
    }
    Ok(())
}

fn merge_deep_cleanup_records(
    current: OperationRecord,
    incoming: OperationRecord,
) -> CoreResult<OperationRecord> {
    let (
        OperationDetails::DeepCleanup(current_details),
        OperationDetails::DeepCleanup(incoming_details),
    ) = (current.details, incoming.details)
    else {
        return Err(CoreError::persistence(
            "deep-cleanup history merge received incompatible details",
        ));
    };
    if current.dry_run != incoming.dry_run {
        return Err(CoreError::persistence(
            "deep-cleanup history steps disagree on dry-run mode",
        ));
    }
    let details = DeepCleanupOperationDetails {
        cleanup: incoming_details.cleanup.or(current_details.cleanup),
        application_leftovers: incoming_details
            .application_leftovers
            .or(current_details.application_leftovers),
    };
    let mut merged = summarize_deep_cleanup(
        incoming.operation_id,
        current.started_at_ms.min(incoming.started_at_ms),
        current.finished_at_ms.max(incoming.finished_at_ms),
        incoming.dry_run,
        details,
    );
    merged.schema_version = incoming.schema_version;
    Ok(merged)
}

pub(crate) fn summarize_deep_cleanup(
    operation_id: String,
    started_at_ms: u64,
    finished_at_ms: u64,
    dry_run: bool,
    details: DeepCleanupOperationDetails,
) -> OperationRecord {
    let cleanup_selected = details
        .cleanup
        .as_ref()
        .map_or(0, |step| step.selected_rule_ids.len() as u64);
    let leftover_selected = details
        .application_leftovers
        .as_ref()
        .map_or(0, |step| step.candidate_ids.len() as u64);
    let cleanup_expected = details
        .cleanup
        .as_ref()
        .map_or(0, |step| step.expected_bytes);
    let leftover_expected = details
        .application_leftovers
        .as_ref()
        .map_or(0, |step| step.expected_bytes);
    let cleanup_released = details.cleanup.as_ref().map_or(0, |step| {
        step.actions
            .iter()
            .map(|action| action.released_bytes)
            .sum()
    });
    let leftover_released = details.application_leftovers.as_ref().map_or(0, |step| {
        step.actions
            .iter()
            .map(|action| action.released_bytes)
            .sum()
    });
    let cleanup_affected = details.cleanup.as_ref().map_or(0, |step| {
        step.actions
            .iter()
            .map(|action| action.affected_item_count)
            .sum()
    });
    let leftover_affected = details.application_leftovers.as_ref().map_or(0, |step| {
        step.actions
            .iter()
            .filter(|action| action.status == crate::ApplicationLeftoverActionStatus::Completed)
            .count() as u64
    });
    let cleanup_failed = details.cleanup.as_ref().map_or(0, |step| {
        step.actions
            .iter()
            .map(|action| action.failed_item_count)
            .sum()
    });
    let leftover_failed = details.application_leftovers.as_ref().map_or(0, |step| {
        step.actions
            .iter()
            .filter(|action| action.status == crate::ApplicationLeftoverActionStatus::Failed)
            .count() as u64
    });
    let cleanup_cancelled = details.cleanup.as_ref().is_some_and(|step| {
        step.actions
            .iter()
            .any(|action| action.reason_code == Some(crate::CleanupActionReason::Cancelled))
    });
    let leftover_cancelled = details.application_leftovers.as_ref().is_some_and(|step| {
        step.actions
            .iter()
            .any(|action| action.status == crate::ApplicationLeftoverActionStatus::Cancelled)
    });
    let cancelled = cleanup_cancelled || leftover_cancelled;
    let failed_item_count = cleanup_failed + leftover_failed;
    OperationRecord {
        schema_version: OPERATION_RECORD_SCHEMA_VERSION,
        operation_id,
        category: OperationCategory::DeepCleanup,
        started_at_ms,
        finished_at_ms,
        outcome: if cancelled {
            OperationOutcome::Cancelled
        } else if failed_item_count > 0 {
            OperationOutcome::CompletedWithWarnings
        } else {
            OperationOutcome::Completed
        },
        dry_run,
        selected_item_count: cleanup_selected + leftover_selected,
        affected_item_count: cleanup_affected + leftover_affected,
        expected_bytes: cleanup_expected.saturating_add(leftover_expected),
        released_bytes: Some(cleanup_released.saturating_add(leftover_released)),
        released_bytes_is_estimate: false,
        failed_item_count,
        details: OperationDetails::DeepCleanup(details),
    }
}

pub(crate) fn file_cleanup_record(
    operation_id: String,
    category: FileCleanupHistoryCategory,
    started_at_ms: u64,
    finished_at_ms: u64,
    selected_paths: Vec<String>,
    expected_bytes: u64,
    result: &crate::PermanentDeleteBatchResult,
) -> OperationRecord {
    let selected_item_count = selected_paths.len() as u64;
    let items = result
        .removed_paths
        .iter()
        .take(FILE_CLEANUP_HISTORY_ITEM_LIMIT)
        .map(|path| FileCleanupHistoryItem {
            path: path.clone(),
            status: FileCleanupHistoryItemStatus::Deleted,
        })
        .collect::<Vec<_>>();
    let details = FileCleanupOperationDetails {
        omitted_item_count: selected_item_count.saturating_sub(items.len() as u64),
        items,
    };
    let (category, details) = match category {
        FileCleanupHistoryCategory::LargeFiles => (
            OperationCategory::LargeFileCleanup,
            OperationDetails::LargeFileCleanup(details),
        ),
        FileCleanupHistoryCategory::DuplicateFiles => (
            OperationCategory::DuplicateFileCleanup,
            OperationDetails::DuplicateFileCleanup(details),
        ),
    };
    OperationRecord {
        schema_version: OPERATION_RECORD_SCHEMA_VERSION,
        operation_id,
        category,
        started_at_ms,
        finished_at_ms,
        outcome: if result.failed.is_empty() {
            OperationOutcome::Completed
        } else {
            OperationOutcome::CompletedWithWarnings
        },
        dry_run: false,
        selected_item_count,
        affected_item_count: result.removed_paths.len() as u64,
        // Existing estimate-only records display this field as projected release.
        // Failed and already absent selections must not inflate that estimate.
        expected_bytes: if result.released_bytes_is_estimate {
            result.released_bytes
        } else {
            expected_bytes
        },
        released_bytes: (!result.released_bytes_is_estimate).then_some(result.released_bytes),
        released_bytes_is_estimate: result.released_bytes_is_estimate,
        failed_item_count: result.failed.len() as u64,
        details,
    }
}

fn save_records(records: Vec<OperationRecord>) -> CoreResult<()> {
    let content = serde_json::to_vec_pretty(&HistoryDocument {
        schema_version: HISTORY_DOCUMENT_SCHEMA_VERSION,
        records,
    })
    .map_err(|error| {
        CoreError::persistence(format!("failed to serialize cleanup history: {error}"))
    })?;
    write_atomic(&history_path()?, &content)
}

fn write_atomic(path: &Path, content: &[u8]) -> CoreResult<()> {
    let temporary = path.with_file_name("history.json.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temporary)
        .map_err(|error| {
            CoreError::persistence(format!(
                "failed to create cleanup history temporary file path={}: {error}",
                crate::diagnostic_path(&temporary)
            ))
        })?;
    file.write_all(content)
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            CoreError::persistence(format!(
                "failed to write cleanup history temporary file path={}: {error}",
                crate::diagnostic_path(&temporary)
            ))
        })?;
    replace_file(&temporary, path).map_err(|error| {
        CoreError::persistence(format!(
            "failed to save cleanup history source={} target={}: {error}",
            crate::diagnostic_path(&temporary),
            crate::diagnostic_path(path)
        ))
    })
}

fn quarantine_invalid_history(path: &Path) -> CoreResult<()> {
    let quarantine = path.with_file_name("history.invalid.json");
    replace_file(path, &quarantine).map_err(|error| {
        CoreError::persistence(format!(
            "failed to quarantine unsupported cleanup history: {error}"
        ))
    })
}

fn history_path() -> CoreResult<PathBuf> {
    let directory = application_paths()?.data_directory();
    fs::create_dir_all(directory).map_err(|error| {
        CoreError::persistence(format!(
            "failed to create the application data directory: {error}"
        ))
    })?;
    Ok(directory.join("history.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        history::{
            ApplicationLeftoverOperationDetails, CleanupOperationDetails,
            DeepCleanupOperationDetails,
        },
        ApplicationLeftoverActionResult, ApplicationLeftoverActionStatus, CleanupActionKind,
        CleanupActionResult, CleanupActionStatus,
    };

    #[test]
    fn history_write_failure_identifies_the_actual_target_and_native_error() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-diagnostic-history-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let blocker = root.join("blocked-parent");
        fs::write(&blocker, b"fixture").unwrap();
        let result = write_atomic(&blocker.join("history.json"), b"content-must-not-be-logged");
        fs::remove_dir_all(&root).unwrap();
        let error = result.unwrap_err();
        assert!(error.diagnostic().contains("blocked-parent"));
        assert!(error.diagnostic().contains("history.json.tmp"));
        assert!(error.diagnostic().contains("os error"));
        assert!(!error.diagnostic().contains("content-must-not-be-logged"));
        println!(
            "history_write_fixture_failed error={}",
            mangodisk_platform::diagnostics::text(&error)
        );
    }

    fn cleanup_step(rule_id: &str, expected_bytes: u64) -> CleanupOperationDetails {
        CleanupOperationDetails {
            selected_rule_ids: vec![rule_id.to_string()],
            expected_bytes,
            actions: vec![CleanupActionResult {
                rule_id: rule_id.to_string(),
                action_kind: CleanupActionKind::Delete,
                status: CleanupActionStatus::Completed,
                reason_code: None,
                bytes_expected: expected_bytes,
                released_bytes: expected_bytes,
                affected_item_count: 1,
                failed_item_count: 0,
                running_processes: Vec::new(),
            }],
        }
    }

    #[test]
    fn current_history_uses_product_categories() {
        let record = summarize_deep_cleanup(
            "deep-cleanup-1".to_string(),
            1,
            2,
            false,
            DeepCleanupOperationDetails {
                cleanup: Some(cleanup_step("system.cache", 10)),
                application_leftovers: None,
            },
        );
        let json = serde_json::to_string(&HistoryDocument {
            schema_version: HISTORY_DOCUMENT_SCHEMA_VERSION,
            records: vec![record],
        })
        .expect("current history must serialize");
        let document =
            serde_json::from_str::<HistoryDocument>(&json).expect("current history must load");

        assert_eq!(document.records[0].category, OperationCategory::DeepCleanup);
        assert!(validate_history_document(document).is_ok());
    }

    #[test]
    fn previous_history_schema_is_rejected_during_development() {
        let record = summarize_deep_cleanup(
            "previous-schema".to_string(),
            1,
            2,
            false,
            DeepCleanupOperationDetails {
                cleanup: Some(cleanup_step("system.cache", 10)),
                application_leftovers: None,
            },
        );

        assert!(validate_history_document(HistoryDocument {
            schema_version: HISTORY_DOCUMENT_SCHEMA_VERSION - 1,
            records: vec![record],
        })
        .is_err());
    }

    #[test]
    fn deep_cleanup_steps_merge_into_one_record() {
        let current = summarize_deep_cleanup(
            "deep-cleanup-1".to_string(),
            10,
            20,
            false,
            DeepCleanupOperationDetails {
                cleanup: Some(cleanup_step("system.cache", 10)),
                application_leftovers: None,
            },
        );
        let incoming = summarize_deep_cleanup(
            "deep-cleanup-1".to_string(),
            21,
            30,
            false,
            DeepCleanupOperationDetails {
                cleanup: None,
                application_leftovers: Some(ApplicationLeftoverOperationDetails {
                    candidate_ids: vec!["leftover-1".to_string()],
                    expected_bytes: 5,
                    actions: Vec::new(),
                }),
            },
        );

        let merged =
            merge_deep_cleanup_records(current, incoming).expect("steps must merge successfully");

        assert_eq!(merged.selected_item_count, 2);
        assert_eq!(merged.expected_bytes, 15);
        assert_eq!(merged.started_at_ms, 10);
        assert_eq!(merged.finished_at_ms, 30);
        let OperationDetails::DeepCleanup(details) = merged.details else {
            panic!("merged details must remain deep cleanup");
        };
        assert!(details.cleanup.is_some());
        assert!(details.application_leftovers.is_some());
    }

    #[test]
    fn cancelled_leftover_actions_mark_the_deep_cleanup_record_cancelled() {
        let record = summarize_deep_cleanup(
            "deep-cleanup-cancelled".to_string(),
            10,
            20,
            false,
            DeepCleanupOperationDetails {
                cleanup: None,
                application_leftovers: Some(ApplicationLeftoverOperationDetails {
                    candidate_ids: vec!["leftover-1".to_string()],
                    expected_bytes: 5,
                    actions: vec![ApplicationLeftoverActionResult {
                        candidate_id: "leftover-1".to_string(),
                        application_identifier: "fixture.application".to_string(),
                        application_name: "Fixture".to_string(),
                        status: ApplicationLeftoverActionStatus::Cancelled,
                        reason: None,
                        expected_bytes: 5,
                        released_bytes: 0,
                    }],
                }),
            },
        );

        assert_eq!(record.outcome, OperationOutcome::Cancelled);
        assert_eq!(record.failed_item_count, 0);
        assert_eq!(record.affected_item_count, 0);
    }

    #[test]
    fn category_and_details_must_match() {
        let mut record = summarize_deep_cleanup(
            "invalid".to_string(),
            1,
            2,
            false,
            DeepCleanupOperationDetails {
                cleanup: Some(cleanup_step("system.cache", 10)),
                application_leftovers: None,
            },
        );
        record.category = OperationCategory::LargeFileCleanup;

        assert!(validate_operation_record(&record).is_err());
    }

    #[test]
    fn file_cleanup_record_uses_the_requested_product_category() {
        let result = crate::PermanentDeleteBatchResult {
            released_bytes_is_estimate: false,
            already_absent_paths: Vec::new(),
            removed_paths: vec!["/fixture/removed.bin".to_string()],
            failed: vec![crate::PermanentDeleteFailure {
                path: "/fixture/failed.bin".to_string(),
                details: crate::filesystem::PermanentDeleteFailureDetails {
                    message: "fixture failure".to_string(),
                    ..Default::default()
                },
            }],
            released_bytes: 10,
        };

        let record = file_cleanup_record(
            "large-files-1".to_string(),
            FileCleanupHistoryCategory::LargeFiles,
            1,
            2,
            vec![
                "/fixture/removed.bin".to_string(),
                "/fixture/failed.bin".to_string(),
            ],
            20,
            &result,
        );

        assert_eq!(record.category, OperationCategory::LargeFileCleanup);
        assert_eq!(record.selected_item_count, 2);
        assert_eq!(record.affected_item_count, 1);
        assert_eq!(record.failed_item_count, 1);
        assert_eq!(record.outcome, OperationOutcome::CompletedWithWarnings);
        let OperationDetails::LargeFileCleanup(details) = &record.details else {
            panic!("large-file cleanup must keep file history details");
        };
        assert_eq!(details.items.len(), 1);
        assert_eq!(details.items[0].path, "/fixture/removed.bin");
        assert_eq!(
            details.items[0].status,
            FileCleanupHistoryItemStatus::Deleted
        );
        assert_eq!(details.omitted_item_count, 1);
        assert!(validate_operation_record(&record).is_ok());
    }

    #[test]
    fn file_cleanup_history_keeps_large_failures_as_totals_only() {
        let failed = (0..1000)
            .map(|index| crate::PermanentDeleteFailure {
                path: format!("/fixture/denied-{index}"),
                details: crate::filesystem::PermanentDeleteFailureDetails {
                    message: "native error".repeat(100),
                    ..Default::default()
                },
            })
            .collect::<Vec<_>>();
        let result = crate::PermanentDeleteBatchResult {
            released_bytes_is_estimate: false,
            already_absent_paths: Vec::new(),
            removed_paths: vec![],
            failed,
            released_bytes: 0,
        };
        let record = file_cleanup_record(
            "failure-1".into(),
            FileCleanupHistoryCategory::DuplicateFiles,
            1,
            2,
            result
                .failed
                .iter()
                .map(|failure| failure.path.clone())
                .collect(),
            4,
            &result,
        );
        validate_operation_record(&record).unwrap();
        let serialized = serde_json::to_string(&record).unwrap();
        assert!(serialized.len() < 1000);
        assert!(!serialized.contains("denied-"));
        assert!(!serialized.contains("native error"));
        assert_eq!(record.failed_item_count, 1000);
        let OperationDetails::DuplicateFileCleanup(details) = record.details else {
            panic!("duplicate details")
        };
        assert!(details.items.is_empty());
        assert_eq!(details.omitted_item_count, 1000);
    }

    #[test]
    fn file_cleanup_history_bounds_successful_details() {
        let selected_paths = (0..FILE_CLEANUP_HISTORY_ITEM_LIMIT + 5)
            .map(|index| format!("/fixture/{index}.bin"))
            .collect::<Vec<_>>();
        let failed_path = selected_paths
            .last()
            .expect("the fixture must contain a failed path")
            .clone();
        let result = crate::PermanentDeleteBatchResult {
            released_bytes_is_estimate: false,
            already_absent_paths: Vec::new(),
            removed_paths: selected_paths[..selected_paths.len() - 1].to_vec(),
            failed: vec![crate::PermanentDeleteFailure {
                path: failed_path.clone(),
                details: crate::filesystem::PermanentDeleteFailureDetails {
                    message: "fixture failure".to_string(),
                    ..Default::default()
                },
            }],
            released_bytes: 10,
        };

        let record = file_cleanup_record(
            "duplicate-files-1".to_string(),
            FileCleanupHistoryCategory::DuplicateFiles,
            1,
            2,
            selected_paths,
            20,
            &result,
        );

        let OperationDetails::DuplicateFileCleanup(details) = &record.details else {
            panic!("duplicate cleanup must keep file history details");
        };
        assert_eq!(details.items.len(), FILE_CLEANUP_HISTORY_ITEM_LIMIT);
        assert_eq!(details.omitted_item_count, 5);
        assert!(details
            .items
            .iter()
            .all(|item| item.status == FileCleanupHistoryItemStatus::Deleted));
        assert!(!details.items.iter().any(|item| item.path == failed_path));
        assert!(validate_operation_record(&record).is_ok());

        let mut oversized = record.clone();
        oversized.selected_item_count += 1;
        let OperationDetails::DuplicateFileCleanup(details) = &mut oversized.details else {
            panic!("duplicate cleanup must keep file history details");
        };
        details.items.push(FileCleanupHistoryItem {
            path: "/fixture/overflow.bin".to_string(),
            status: FileCleanupHistoryItemStatus::Deleted,
        });
        assert!(validate_operation_record(&oversized).is_err());
    }

    #[test]
    fn invalid_history_is_quarantined_instead_of_deleted() {
        let directory = std::env::temp_dir().join(format!(
            "mangodisk-history-quarantine-{}-{}",
            std::process::id(),
            crate::shared::clock::unix_time_ms()
        ));
        fs::create_dir_all(&directory).expect("the history fixture directory must be created");
        let history = directory.join("history.json");
        let quarantine = directory.join("history.invalid.json");
        fs::write(&history, b"invalid history")
            .expect("the invalid history fixture must be written");

        quarantine_invalid_history(&history).expect("invalid history must be quarantined");

        assert!(!history.exists());
        assert_eq!(
            fs::read(&quarantine).expect("the quarantine must remain readable"),
            b"invalid history"
        );
        fs::remove_dir_all(directory).expect("the history fixture directory must be removed");
    }

    #[test]
    fn old_operation_shape_is_not_deserialized() {
        let json = r#"{
            "schemaVersion": 4,
            "operationId": "legacy",
            "domain": "cleanup",
            "operationKind": "cleanup"
        }"#;

        assert!(serde_json::from_str::<OperationRecord>(json).is_err());
    }
    #[test]
    fn estimated_directory_cleanup_does_not_claim_measured_released_bytes() {
        let result = crate::PermanentDeleteBatchResult {
            removed_paths: vec!["/fixture/directory".into()],
            already_absent_paths: vec!["/fixture/missing".into()],
            failed: vec![],
            released_bytes: 4096,
            released_bytes_is_estimate: true,
        };
        let record = file_cleanup_record(
            "directory-estimate".into(),
            FileCleanupHistoryCategory::DuplicateFiles,
            1,
            2,
            vec!["/fixture/directory".into(), "/fixture/missing".into()],
            12288,
            &result,
        );
        assert!(record.released_bytes_is_estimate);
        assert_eq!(record.released_bytes, None);
        assert_eq!(record.expected_bytes, 4096);
        assert_eq!(record.affected_item_count, 1);
        assert_eq!(record.failed_item_count, 0);
        validate_operation_record(&record).unwrap();
    }
}
