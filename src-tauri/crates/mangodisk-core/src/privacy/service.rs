use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Instant, UNIX_EPOCH},
};

use mangodisk_platform::{
    current_platform, Platform, PlatformCancellation, PlatformMutationState,
    PlatformPrivacyApplicationNativeTraceKind, PlatformPrivacyApplicationTraceAvailability,
    PlatformPrivacyApplicationTraceKind, PlatformPrivacyBrowserKind,
    PlatformPrivacySystemTraceKind, PrivacyPlatform,
};

use crate::{
    applications::{
        catalog::ProcessSnapshot,
        process_control::{close_resolved_applications, ResolvedApplicationCloseTarget},
    },
    filesystem::metadata::now_ms,
    history::{
        HistoryService, OperationCategory, OperationDetails, OperationOutcome, OperationRecord,
        PrivacyCleanupHistoryItem, PrivacyCleanupHistoryItemStatus, PrivacyCleanupOperationDetails,
        OPERATION_RECORD_SCHEMA_VERSION,
    },
    shared::operation::{CoordinatedOperationKind, OperationCancellationToken, OperationGuard},
    ApplicationCloseBatchResult, CoreError, CoreErrorCode, CoreResult,
};

use super::{
    browser_database, PrivacyBrowserCloseRequest, PrivacyBrowserCloseRequirement,
    PrivacyBrowserStatusRequest, PrivacyBrowserStatusResult, PrivacyBrowserStatusTarget,
    PrivacyCapabilityState, PrivacyCategory, PrivacyDataKind, PrivacyDetailsPage,
    PrivacyDetailsPresentation, PrivacyDetailsRequest, PrivacyExecutionItemResult,
    PrivacyExecutionItemStatus, PrivacyExecutionPlan, PrivacyExecutionPlanItem,
    PrivacyExecutionProgress, PrivacyExecutionProgressItem, PrivacyExecutionRequest,
    PrivacyExecutionResult, PrivacyExecutionRunRequest, PrivacyExecutionStage, PrivacyImpact,
    PrivacyItem, PrivacyRecommendation, PrivacyScanProgress, PrivacyScanRequest, PrivacyScanResult,
    PrivacyScanStage, PrivacySensitivity, PrivacySourceCoverage, PrivacyTimeRange,
    PRIVACY_DETAILS_SCHEMA_VERSION, PRIVACY_PLAN_SCHEMA_VERSION, PRIVACY_SCAN_SCHEMA_VERSION,
};

const PLAN_TTL_MS: u64 = 5 * 60 * 1_000;
const MAX_SELECTIONS: usize = 256;
const MAX_DIRECTORY_SCAN_ENTRIES: u64 = 50_000;
const MAX_DETAIL_PAGE_SIZE: u32 = 200;
const MAX_DETAIL_OFFSET: u64 = 100_000;
const MAX_DETAIL_LABEL_CHARS: usize = 2_048;
const MAX_DETAIL_LIST_ENTRIES: usize =
    MAX_DETAIL_OFFSET as usize + MAX_DETAIL_PAGE_SIZE as usize + 1;

static SCAN_SESSION: OnceLock<Mutex<Option<PrivacyScanSession>>> = OnceLock::new();
static EXECUTION_PLAN: OnceLock<Mutex<Option<PendingPrivacyPlan>>> = OnceLock::new();

#[derive(Clone)]
struct PrivacyScanSession {
    public_result: PrivacyScanResult,
    candidates: BTreeMap<String, NativePrivacyCandidate>,
}

#[derive(Clone)]
struct PendingPrivacyPlan {
    public_plan: PrivacyExecutionPlan,
    candidates: Vec<NativePrivacyCandidate>,
    time_range: PrivacyTimeRange,
}

#[derive(Clone)]
struct NativePrivacyCandidate {
    token: String,
    item: PrivacyItem,
    fingerprint: String,
    action: NativePrivacyAction,
    browser_process_names: Vec<String>,
}

#[derive(Debug)]
struct PrivacyMutationFailure {
    error: CoreError,
    confirmed_affected_item_count: u64,
}

impl PrivacyMutationFailure {
    fn new(error: CoreError, confirmed_affected_item_count: u64) -> Self {
        Self {
            error,
            confirmed_affected_item_count,
        }
    }

    fn with_possible_side_effects(mut self) -> Self {
        self.error = self.error.with_possible_side_effects();
        self
    }
}

impl From<CoreError> for PrivacyMutationFailure {
    fn from(error: CoreError) -> Self {
        Self::new(error, 0)
    }
}

impl From<mangodisk_platform::PlatformError> for PrivacyMutationFailure {
    fn from(error: mangodisk_platform::PlatformError) -> Self {
        Self::from(CoreError::from(error))
    }
}

struct PrivacyItemInput {
    token: String,
    source_id: String,
    source_name: String,
    profile_id: Option<String>,
    profile_name: Option<String>,
    kind: PrivacyDataKind,
    capability: PrivacyCapabilityState,
    item_count: u64,
    estimated_bytes: u64,
    requires_browser_close: bool,
}

#[derive(Clone)]
enum NativePrivacyAction {
    Database {
        path: PathBuf,
        browser: PlatformPrivacyBrowserKind,
        kind: PrivacyDataKind,
        range: PrivacyTimeRange,
        scan_now_ms: u64,
    },
    Directories {
        roots: Vec<PathBuf>,
    },
    /// Aggregate evidence that is intentionally excluded from every execution plan.
    ReviewOnlyFile {
        path: PathBuf,
        browser: PlatformPrivacyBrowserKind,
        kind: PrivacyDataKind,
        range: PrivacyTimeRange,
        scan_now_ms: u64,
    },
    System {
        kind: PlatformPrivacySystemTraceKind,
        roots: Vec<PathBuf>,
        /// Whether this file-backed item also clears an operating-system owned mirror.
        has_native_revision: bool,
    },
    ApplicationNative {
        kind: PlatformPrivacyApplicationNativeTraceKind,
    },
}

pub struct PrivacyService;

impl PrivacyService {
    pub fn cancel_scan() {
        OperationCancellationToken::privacy_scan().cancel();
    }

    pub fn cancel_execution() {
        OperationCancellationToken::privacy_execution().cancel();
    }

    pub fn scan(request: PrivacyScanRequest) -> CoreResult<PrivacyScanResult> {
        Self::scan_with_progress(request, |_| {})
    }

    pub fn scan_with_progress(
        request: PrivacyScanRequest,
        progress: impl Fn(PrivacyScanProgress),
    ) -> CoreResult<PrivacyScanResult> {
        let operation = OperationGuard::start(CoordinatedOperationKind::PrivacyScan)?;
        // Starting a new scan invalidates every previous token immediately, including when the new
        // scan is cancelled or fails. This prevents adapters from executing a stale result after a
        // failed refresh.
        clear_scan_session()?;
        clear_pending_plan()?;
        let started = Instant::now();
        let observed_at_ms = now_ms();
        let cancellation = platform_cancellation(&operation);
        let platform = current_platform();
        progress(PrivacyScanProgress {
            stage: PrivacyScanStage::Discovering,
            source_name: None,
            completed_sources: 0,
            total_sources: 0,
        });
        let discovery = platform.discover_privacy_sources(&cancellation)?;
        let running_names = platform
            .running_process_names_with_cancellation(&cancellation)?
            .into_iter()
            .map(|name| process_leaf(&name).to_lowercase())
            .collect::<BTreeSet<_>>();
        operation.ensure_not_cancelled()?;

        let scan_nonce = format!("{}:{}", operation.id(), observed_at_ms);
        let mut candidates = BTreeMap::new();
        let mut items = Vec::new();
        let mut coverage = Vec::new();
        let total_sources = (discovery.browsers.len()
            + discovery.applications.len()
            + discovery.system_traces.len()) as u64;
        let mut completed_sources = 0_u64;

        for browser in discovery.browsers {
            progress(PrivacyScanProgress {
                stage: PrivacyScanStage::Browser,
                source_name: Some(browser.display_name.clone()),
                completed_sources,
                total_sources,
            });
            let browser_icon_path = browser
                .application_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned());
            let browser_running = browser
                .process_names
                .iter()
                .any(|name| running_names.contains(&name.to_lowercase()));
            let mut browser_item_count = 0_u64;
            for profile in browser.profiles {
                operation.ensure_not_cancelled()?;
                if let Some(path) = &profile.history_database {
                    add_database_candidate(
                        &scan_nonce,
                        &browser.provider_key,
                        &browser.display_name,
                        &profile.provider_key,
                        &profile.display_name,
                        path,
                        browser.kind,
                        PrivacyDataKind::BrowsingHistory,
                        request.time_range,
                        observed_at_ms,
                        browser_running,
                        &browser.process_names,
                        &mut candidates,
                        &mut items,
                    );
                    if browser.kind != PlatformPrivacyBrowserKind::Safari {
                        add_database_candidate(
                            &scan_nonce,
                            &browser.provider_key,
                            &browser.display_name,
                            &profile.provider_key,
                            &profile.display_name,
                            path,
                            browser.kind,
                            PrivacyDataKind::DownloadHistory,
                            request.time_range,
                            observed_at_ms,
                            browser_running,
                            &browser.process_names,
                            &mut candidates,
                            &mut items,
                        );
                    }
                    if browser.kind == PlatformPrivacyBrowserKind::Chromium {
                        add_database_candidate(
                            &scan_nonce,
                            &browser.provider_key,
                            &browser.display_name,
                            &profile.provider_key,
                            &profile.display_name,
                            path,
                            browser.kind,
                            PrivacyDataKind::SearchHistory,
                            request.time_range,
                            observed_at_ms,
                            browser_running,
                            &browser.process_names,
                            &mut candidates,
                            &mut items,
                        );
                    }
                }
                if let Some(path) = &profile.cookie_database {
                    add_database_candidate(
                        &scan_nonce,
                        &browser.provider_key,
                        &browser.display_name,
                        &profile.provider_key,
                        &profile.display_name,
                        path,
                        browser.kind,
                        PrivacyDataKind::Cookies,
                        request.time_range,
                        observed_at_ms,
                        browser_running,
                        &browser.process_names,
                        &mut candidates,
                        &mut items,
                    );
                }
                if let Some(path) = &profile.permission_database {
                    add_database_candidate(
                        &scan_nonce,
                        &browser.provider_key,
                        &browser.display_name,
                        &profile.provider_key,
                        &profile.display_name,
                        path,
                        browser.kind,
                        PrivacyDataKind::SitePermissions,
                        request.time_range,
                        observed_at_ms,
                        browser_running,
                        &browser.process_names,
                        &mut candidates,
                        &mut items,
                    );
                }
                if let Some(path) = &profile.saved_password_source {
                    add_database_candidate(
                        &scan_nonce,
                        &browser.provider_key,
                        &browser.display_name,
                        &profile.provider_key,
                        &profile.display_name,
                        path,
                        browser.kind,
                        PrivacyDataKind::SavedPasswords,
                        request.time_range,
                        observed_at_ms,
                        browser_running,
                        &browser.process_names,
                        &mut candidates,
                        &mut items,
                    );
                }
                if let Some(path) = &profile.autofill_database {
                    add_database_candidate(
                        &scan_nonce,
                        &browser.provider_key,
                        &browser.display_name,
                        &profile.provider_key,
                        &profile.display_name,
                        path,
                        browser.kind,
                        PrivacyDataKind::AutofillData,
                        request.time_range,
                        observed_at_ms,
                        browser_running,
                        &browser.process_names,
                        &mut candidates,
                        &mut items,
                    );
                }
                if let Some(path) = &profile.top_sites_database {
                    add_database_candidate(
                        &scan_nonce,
                        &browser.provider_key,
                        &browser.display_name,
                        &profile.provider_key,
                        &profile.display_name,
                        path,
                        browser.kind,
                        PrivacyDataKind::FrequentlyVisitedSites,
                        request.time_range,
                        observed_at_ms,
                        browser_running,
                        &browser.process_names,
                        &mut candidates,
                        &mut items,
                    );
                }
                if let Some(path) = &profile.shortcut_database {
                    add_database_candidate(
                        &scan_nonce,
                        &browser.provider_key,
                        &browser.display_name,
                        &profile.provider_key,
                        &profile.display_name,
                        path,
                        browser.kind,
                        PrivacyDataKind::AddressBarShortcuts,
                        request.time_range,
                        observed_at_ms,
                        browser_running,
                        &browser.process_names,
                        &mut candidates,
                        &mut items,
                    );
                }
                if let Some(path) = &profile.favicon_database {
                    add_database_candidate(
                        &scan_nonce,
                        &browser.provider_key,
                        &browser.display_name,
                        &profile.provider_key,
                        &profile.display_name,
                        path,
                        browser.kind,
                        PrivacyDataKind::WebsiteIcons,
                        request.time_range,
                        observed_at_ms,
                        browser_running,
                        &browser.process_names,
                        &mut candidates,
                        &mut items,
                    );
                }
                add_directory_candidate(
                    &scan_nonce,
                    &browser.provider_key,
                    &browser.display_name,
                    &profile.provider_key,
                    &profile.display_name,
                    PrivacyDataKind::Sessions,
                    profile.session_directories,
                    request.time_range,
                    browser_running,
                    &browser.process_names,
                    &cancellation,
                    &mut candidates,
                    &mut items,
                );
                add_directory_candidate(
                    &scan_nonce,
                    &browser.provider_key,
                    &browser.display_name,
                    &profile.provider_key,
                    &profile.display_name,
                    PrivacyDataKind::BrowserCache,
                    profile.cache_directories,
                    request.time_range,
                    browser_running,
                    &browser.process_names,
                    &cancellation,
                    &mut candidates,
                    &mut items,
                );
                add_directory_candidate(
                    &scan_nonce,
                    &browser.provider_key,
                    &browser.display_name,
                    &profile.provider_key,
                    &profile.display_name,
                    PrivacyDataKind::SiteStorage,
                    profile.site_storage_directories,
                    request.time_range,
                    browser_running,
                    &browser.process_names,
                    &cancellation,
                    &mut candidates,
                    &mut items,
                );
            }
            browser_item_count += items
                .iter()
                .filter(|item| item.source_id == browser.provider_key)
                .map(|item| item.item_count)
                .sum::<u64>();
            let browser_capabilities = items
                .iter()
                .filter(|item| item.source_id == browser.provider_key)
                .map(|item| item.capability)
                .collect::<Vec<_>>();
            coverage.push(PrivacySourceCoverage {
                source_id: browser.provider_key,
                source_name: browser.display_name,
                icon_path: browser_icon_path,
                capability: if browser_capabilities
                    .contains(&PrivacyCapabilityState::SchemaUnsupported)
                {
                    PrivacyCapabilityState::SchemaUnsupported
                } else if browser_capabilities.contains(&PrivacyCapabilityState::PermissionRequired)
                {
                    PrivacyCapabilityState::PermissionRequired
                } else if browser_capabilities.contains(&PrivacyCapabilityState::Unavailable) {
                    PrivacyCapabilityState::Unavailable
                } else if browser_capabilities.contains(&PrivacyCapabilityState::BrowserRunning) {
                    PrivacyCapabilityState::BrowserRunning
                } else if browser_item_count == 0 {
                    PrivacyCapabilityState::Empty
                } else {
                    PrivacyCapabilityState::Ready
                },
                item_count: browser_item_count,
            });
            completed_sources = completed_sources.saturating_add(1);
        }

        for application in discovery.applications {
            progress(PrivacyScanProgress {
                stage: PrivacyScanStage::Application,
                source_name: Some(application.display_name.clone()),
                completed_sources,
                total_sources,
            });
            let application_running = application
                .process_names
                .iter()
                .any(|name| running_names.contains(&process_leaf(name).to_lowercase()));
            let icon_path = application
                .application_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned());
            let source_id = application.provider_key;
            let source_name = application.display_name;
            let process_names = application.process_names;
            let mut source_capabilities = Vec::new();
            let mut source_item_count = 0_u64;

            for trace in application.traces {
                operation.ensure_not_cancelled()?;
                let kind = application_data_kind(trace.kind);
                let token = candidate_token(&scan_nonce, &source_id, None, kind);
                let range_supported =
                    !trace.all_time_only || request.time_range == PrivacyTimeRange::AllTime;
                let platform_capability = application_availability_capability(trace.availability);
                let (base_capability, item_count, estimated_bytes, fingerprint) =
                    if !range_supported {
                        (PrivacyCapabilityState::Unsupported, 0, 0, None)
                    } else if let Some(capability) = platform_capability {
                        (capability, 0, 0, None)
                    } else if !trace.roots.is_empty() {
                        match roots_summary_fingerprint(&trace.roots, &cancellation) {
                            Ok((fingerprint, count, bytes)) => (
                                if count == 0 {
                                    PrivacyCapabilityState::Empty
                                } else {
                                    PrivacyCapabilityState::Ready
                                },
                                count,
                                bytes,
                                Some(fingerprint),
                            ),
                            Err(error) if error.code() == CoreErrorCode::PermissionDenied => {
                                (PrivacyCapabilityState::PermissionRequired, 0, 0, None)
                            }
                            Err(_) => (PrivacyCapabilityState::Unavailable, 0, 0, None),
                        }
                    } else if trace.native_kind.is_some() {
                        (
                            if trace.item_count == 0 {
                                PrivacyCapabilityState::Empty
                            } else {
                                PrivacyCapabilityState::Ready
                            },
                            trace.item_count,
                            0,
                            Some(trace.revision.clone()),
                        )
                    } else {
                        (PrivacyCapabilityState::Empty, 0, 0, Some("empty".into()))
                    };
                let capability = if application_running
                    && item_count > 0
                    && matches!(
                        base_capability,
                        PrivacyCapabilityState::Ready | PrivacyCapabilityState::Empty
                    ) {
                    PrivacyCapabilityState::ApplicationRunning
                } else {
                    base_capability
                };
                let item = privacy_item(PrivacyItemInput {
                    token: token.clone(),
                    source_id: source_id.clone(),
                    source_name: source_name.clone(),
                    profile_id: None,
                    profile_name: None,
                    kind,
                    capability,
                    item_count,
                    estimated_bytes,
                    requires_browser_close: !process_names.is_empty(),
                });
                if let Some(fingerprint) = fingerprint {
                    let action = match trace.native_kind {
                        Some(kind) => NativePrivacyAction::ApplicationNative { kind },
                        None => NativePrivacyAction::Directories { roots: trace.roots },
                    };
                    candidates.insert(
                        token.clone(),
                        NativePrivacyCandidate {
                            token,
                            item: item.clone(),
                            fingerprint,
                            action,
                            browser_process_names: process_names.clone(),
                        },
                    );
                }
                source_capabilities.push(capability);
                source_item_count = source_item_count.saturating_add(item_count);
                items.push(item);
            }
            coverage.push(PrivacySourceCoverage {
                source_id,
                source_name,
                icon_path,
                capability: source_coverage_capability(&source_capabilities, source_item_count),
                item_count: source_item_count,
            });
            completed_sources = completed_sources.saturating_add(1);
        }

        for trace in discovery.system_traces {
            progress(PrivacyScanProgress {
                stage: PrivacyScanStage::System,
                source_name: Some(trace.display_name.clone()),
                completed_sources,
                total_sources,
            });
            let capability = if !trace.available
                || (trace.all_time_only && request.time_range != PrivacyTimeRange::AllTime)
            {
                PrivacyCapabilityState::Unsupported
            } else if trace.item_count == 0 {
                PrivacyCapabilityState::Empty
            } else {
                PrivacyCapabilityState::Ready
            };
            coverage.push(PrivacySourceCoverage {
                source_id: trace.provider_key.clone(),
                source_name: trace.display_name.clone(),
                icon_path: None,
                capability,
                item_count: trace.item_count,
            });
            if trace.available {
                let kind = match trace.kind {
                    PlatformPrivacySystemTraceKind::CurrentClipboard => {
                        PrivacyDataKind::CurrentClipboard
                    }
                    PlatformPrivacySystemTraceKind::ClipboardHistory => {
                        PrivacyDataKind::ClipboardHistory
                    }
                    PlatformPrivacySystemTraceKind::RecentItems => PrivacyDataKind::RecentItems,
                    PlatformPrivacySystemTraceKind::RecentApplications => {
                        PrivacyDataKind::RecentApplications
                    }
                    PlatformPrivacySystemTraceKind::RecentDocumentHistory => {
                        PrivacyDataKind::RecentItems
                    }
                    PlatformPrivacySystemTraceKind::ApplicationUsageHistory => {
                        PrivacyDataKind::ApplicationUsageHistory
                    }
                    PlatformPrivacySystemTraceKind::NetworkConnectionHistory => {
                        PrivacyDataKind::NetworkConnectionHistory
                    }
                    PlatformPrivacySystemTraceKind::FolderViewHistory => {
                        PrivacyDataKind::FolderViewHistory
                    }
                    PlatformPrivacySystemTraceKind::PrinterHistory => {
                        PrivacyDataKind::PrinterHistory
                    }
                    PlatformPrivacySystemTraceKind::ShellHistory => PrivacyDataKind::ShellHistory,
                    PlatformPrivacySystemTraceKind::JumpLists => PrivacyDataKind::JumpLists,
                    PlatformPrivacySystemTraceKind::RunDialogHistory => {
                        PrivacyDataKind::RunDialogHistory
                    }
                    PlatformPrivacySystemTraceKind::FileDialogHistory => {
                        PrivacyDataKind::FileDialogHistory
                    }
                    PlatformPrivacySystemTraceKind::ExplorerSearchHistory => {
                        PrivacyDataKind::SystemSearchHistory
                    }
                    PlatformPrivacySystemTraceKind::ExplorerPathHistory => {
                        PrivacyDataKind::ExplorerPathHistory
                    }
                };
                let token = candidate_token(&scan_nonce, &trace.provider_key, None, kind);
                let has_native_revision = !trace.roots.is_empty() && !trace.revision.is_empty();
                let (fingerprint, _, bytes) = if trace.roots.is_empty() {
                    (trace.revision.clone(), 0, 0)
                } else {
                    let (root_revision, count, bytes) =
                        roots_summary_fingerprint(&trace.roots, &cancellation)?;
                    (
                        system_trace_fingerprint(
                            &root_revision,
                            has_native_revision.then_some(trace.revision.as_str()),
                        ),
                        count,
                        bytes,
                    )
                };
                let item = privacy_item(PrivacyItemInput {
                    token: token.clone(),
                    source_id: trace.provider_key,
                    source_name: trace.display_name,
                    profile_id: None,
                    profile_name: None,
                    kind,
                    capability,
                    item_count: trace.item_count,
                    estimated_bytes: bytes,
                    requires_browser_close: false,
                });
                candidates.insert(
                    token.clone(),
                    NativePrivacyCandidate {
                        token,
                        item: item.clone(),
                        fingerprint,
                        action: NativePrivacyAction::System {
                            kind: trace.kind,
                            roots: trace.roots,
                            has_native_revision,
                        },
                        browser_process_names: Vec::new(),
                    },
                );
                items.push(item);
            }
            completed_sources = completed_sources.saturating_add(1);
        }

        progress(PrivacyScanProgress {
            stage: PrivacyScanStage::Finalizing,
            source_name: None,
            completed_sources: total_sources,
            total_sources,
        });

        // Several privacy kinds can share one SQLite database. A later count against that database
        // can update its WAL/SHM metadata and invalidate a fingerprint captured for an earlier kind.
        // Refresh all database fingerprints only after every read-only query has completed.
        refresh_database_fingerprints(&mut candidates)?;
        items.sort_by(|left, right| {
            (
                left.category,
                &left.source_name,
                &left.profile_name,
                left.kind as u8,
            )
                .cmp(&(
                    right.category,
                    &right.source_name,
                    &right.profile_name,
                    right.kind as u8,
                ))
        });
        let revision = revision_for(&candidates);
        let public_result = PrivacyScanResult {
            schema_version: PRIVACY_SCAN_SCHEMA_VERSION,
            scan_id: blake3::hash(format!("privacy-scan:{scan_nonce}:{revision}").as_bytes())
                .to_hex()
                .to_string(),
            revision,
            time_range: request.time_range,
            scanned_at_ms: observed_at_ms,
            elapsed_ms: started.elapsed().as_millis() as u64,
            items,
            coverage,
        };
        replace_scan_session(PrivacyScanSession {
            public_result: public_result.clone(),
            candidates,
        })?;
        log::info!(
            "privacy_scan_completed operation_id={} source_count={} candidate_count={} elapsed_ms={}",
            operation.id(), public_result.coverage.len(), public_result.items.len(), public_result.elapsed_ms
        );
        operation.complete();
        Ok(public_result)
    }

    pub fn details(request: PrivacyDetailsRequest) -> CoreResult<PrivacyDetailsPage> {
        if request.limit == 0
            || request.limit > MAX_DETAIL_PAGE_SIZE
            || request.offset > MAX_DETAIL_OFFSET
        {
            return Err(CoreError::invalid_input(
                "privacy detail page is outside the supported range",
            ));
        }
        let session = current_scan_session()?;
        if request.scan_id != session.public_result.scan_id {
            return Err(CoreError::invalid_input("privacy scan session has expired"));
        }
        let candidate = session
            .candidates
            .get(&request.token)
            .ok_or_else(|| CoreError::invalid_input("privacy detail token is unknown"))?;
        // Detail loading is read-only and the opaque token remains bound to one allowlisted source
        // in the active scan session. Windows recent-item registries and application databases can
        // legitimately change immediately after a scan, so enforcing the destructive-operation
        // fingerprint here made an ordinary detail click fail intermittently. Execution still
        // performs its complete fingerprint preflight before the first mutation.
        log::info!(
            "privacy_details_requested source_id={} kind={:?} offset={} limit={}",
            candidate.item.source_id,
            candidate.item.kind,
            request.offset,
            request.limit
        );
        let fetch_limit = request.limit.saturating_add(1);
        let (presentation, mut entries) = match &candidate.action {
            NativePrivacyAction::Database {
                path,
                browser,
                kind,
                range,
                scan_now_ms,
            }
            | NativePrivacyAction::ReviewOnlyFile {
                path,
                browser,
                kind,
                range,
                scan_now_ms,
            } => (
                PrivacyDetailsPresentation::List,
                browser_database::details(
                    path,
                    *browser,
                    *kind,
                    *range,
                    *scan_now_ms,
                    request.offset,
                    fetch_limit,
                )?,
            ),
            NativePrivacyAction::Directories { roots } => (
                PrivacyDetailsPresentation::List,
                root_detail_entries(roots, request.offset, fetch_limit)?,
            ),
            NativePrivacyAction::System { kind, roots, .. } if !roots.is_empty() => {
                let entries =
                    system_root_detail_entries(*kind, roots, request.offset, fetch_limit)?;
                (PrivacyDetailsPresentation::List, entries)
            }
            NativePrivacyAction::System { kind, .. } => {
                platform_detail_entries(current_platform().system_privacy_trace_details(
                    *kind,
                    request.offset,
                    fetch_limit,
                )?)
            }
            NativePrivacyAction::ApplicationNative { kind } => {
                platform_detail_entries(current_platform().application_privacy_trace_details(
                    *kind,
                    request.offset,
                    fetch_limit,
                )?)
            }
        };
        let has_more = entries.len() > request.limit as usize;
        if has_more {
            entries.truncate(request.limit as usize);
        }
        let next_offset = has_more.then(|| request.offset.saturating_add(entries.len() as u64));
        log::info!(
            "privacy_details_loaded source_id={} kind={:?} presentation={:?} entry_count={} has_more={}",
            candidate.item.source_id,
            candidate.item.kind,
            presentation,
            entries.len(),
            has_more
        );
        Ok(PrivacyDetailsPage {
            schema_version: PRIVACY_DETAILS_SCHEMA_VERSION,
            scan_id: request.scan_id,
            token: request.token,
            total_item_count: candidate.item.item_count,
            presentation,
            entries,
            next_offset,
        })
    }

    pub fn prepare(request: PrivacyExecutionRequest) -> CoreResult<PrivacyExecutionPlan> {
        let started_at_ms = now_ms();
        if request.tokens.is_empty() || request.tokens.len() > MAX_SELECTIONS {
            return Err(CoreError::invalid_input(
                "privacy selection count is invalid",
            ));
        }
        let unique = request.tokens.iter().collect::<BTreeSet<_>>();
        if unique.len() != request.tokens.len() {
            return Err(CoreError::invalid_input(
                "privacy selection contains duplicate tokens",
            ));
        }
        let operation = OperationGuard::start(CoordinatedOperationKind::PrivacyExecution)?;
        let session = current_scan_session()?;
        if request.scan_id != session.public_result.scan_id {
            return Err(CoreError::invalid_input("privacy scan session has expired"));
        }
        let mut selected = Vec::new();
        for token in &request.tokens {
            let candidate = session.candidates.get(token).ok_or_else(|| {
                CoreError::invalid_input("privacy selection contains an unknown token")
            })?;
            if !privacy_item_is_actionable(&candidate.item)
                || matches!(
                    candidate.item.recommendation,
                    PrivacyRecommendation::ReviewOnly | PrivacyRecommendation::Unsupported
                )
            {
                return Err(CoreError::invalid_input(
                    "privacy selection is not actionable",
                ));
            }
            // Preparation validates the opaque selection against the current scan session but does
            // not traverse every source again. Execution performs a complete fingerprint preflight
            // before its first mutation, so repeating it here only delays confirmation and can race
            // with live browser databases without adding a safety boundary.
            selected.push(candidate.clone());
        }
        selected.sort_by(|left, right| left.token.cmp(&right.token));
        let created_at_ms = now_ms();
        let expires_at_ms = created_at_ms.saturating_add(PLAN_TTL_MS);
        let plan_id = plan_id(&session.public_result, &selected, expires_at_ms);
        let browser_close_requirements = browser_close_requirements(&selected);
        let public_plan = PrivacyExecutionPlan {
            schema_version: PRIVACY_PLAN_SCHEMA_VERSION,
            plan_id,
            scan_id: session.public_result.scan_id,
            created_at_ms,
            expires_at_ms,
            browser_close_requirements: browser_close_requirements.clone(),
            requires_confirmation: true,
            requires_browser_close: !browser_close_requirements.is_empty(),
            items: selected
                .iter()
                .map(|candidate| PrivacyExecutionPlanItem {
                    token: candidate.token.clone(),
                    source_id: candidate.item.source_id.clone(),
                    source_name: candidate.item.source_name.clone(),
                    profile_name: candidate.item.profile_name.clone(),
                    kind: candidate.item.kind,
                    impact: candidate.item.impact,
                    item_count: candidate.item.item_count,
                    estimated_bytes: candidate.item.estimated_bytes,
                    requires_browser_close: candidate.item.requires_browser_close,
                    synchronization_may_propagate: candidate.item.synchronization_may_propagate,
                })
                .collect(),
        };
        replace_pending_plan(PendingPrivacyPlan {
            public_plan: public_plan.clone(),
            candidates: selected,
            time_range: session.public_result.time_range,
        })?;
        log::info!(
            "privacy_plan_prepared operation_id={} item_count={} expires_in_ms={} elapsed_ms={}",
            operation.id(),
            public_plan.items.len(),
            PLAN_TTL_MS,
            now_ms().saturating_sub(started_at_ms)
        );
        operation.complete();
        Ok(public_plan)
    }

    /// Closes only browser identities resolved from the still-pending privacy
    /// plan. The adapter supplies stable source IDs, never process names, so
    /// this command cannot become an arbitrary process-termination primitive.
    ///
    /// Closing a browser can flush structured privacy data. The pending plan remains available for
    /// a bounded force-close retry and for execution's candidate-scoped refresh. The adapter must
    /// not rebuild the complete privacy catalog between process close and execution.
    pub fn close_browsers(
        request: PrivacyBrowserCloseRequest,
    ) -> CoreResult<ApplicationCloseBatchResult> {
        validate_browser_process_request(&request.plan_id, &request.source_ids)?;
        let operation = OperationGuard::start(CoordinatedOperationKind::ApplicationClose)?;
        let targets = resolve_browser_process_targets(&request.plan_id, &request.source_ids)?;
        let result = close_resolved_applications(targets, request.mode)?;
        log::info!(
            "privacy_browser_close_finished operation_id={} source_count={} remaining_process_count={} failed_target_count={}",
            operation.id(),
            request.source_ids.len(),
            result.remaining_process_count,
            result.failed_target_count
        );
        operation.complete();
        Ok(result)
    }

    /// Refreshes only the process state authorized by the pending plan. It intentionally performs
    /// no close request and no privacy rescan, allowing the confirmation dialog to remove
    /// applications that finish shutting down after the bounded graceful-close wait.
    pub fn refresh_browser_status(
        request: PrivacyBrowserStatusRequest,
    ) -> CoreResult<PrivacyBrowserStatusResult> {
        validate_browser_process_request(&request.plan_id, &request.source_ids)?;
        let started = Instant::now();
        let targets = resolve_browser_process_targets(&request.plan_id, &request.source_ids)?;
        let snapshot = ProcessSnapshot::capture().map_err(CoreError::operation_failed)?;
        let targets = targets
            .into_iter()
            .map(|target| {
                let running_processes = snapshot.matching_processes(&target.executable_names);
                PrivacyBrowserStatusTarget {
                    source_id: target.target_id,
                    running_processes,
                }
            })
            .collect::<Vec<_>>();
        let running_process_count = targets
            .iter()
            .map(|target| target.running_processes.len() as u64)
            .sum();
        let elapsed_ms = started.elapsed().as_millis() as u64;
        if running_process_count == 0 {
            log::info!(
                "privacy_browser_status_cleared source_count={} elapsed_ms={elapsed_ms}",
                request.source_ids.len()
            );
        } else {
            log::debug!(
                "privacy_browser_status_refreshed source_count={} running_process_count={running_process_count} elapsed_ms={elapsed_ms}",
                request.source_ids.len()
            );
        }
        Ok(PrivacyBrowserStatusResult {
            running_process_count,
            targets,
            elapsed_ms,
        })
    }

    pub fn execute(request: PrivacyExecutionRunRequest) -> CoreResult<PrivacyExecutionResult> {
        Self::execute_with_progress(request, |_| {})
    }

    pub fn execute_with_progress<F>(
        request: PrivacyExecutionRunRequest,
        mut update: F,
    ) -> CoreResult<PrivacyExecutionResult>
    where
        F: FnMut(PrivacyExecutionProgress),
    {
        if request.plan_id.len() < 16 || request.plan_id.len() > 128 {
            return Err(CoreError::invalid_input(
                "privacy plan identifier is invalid",
            ));
        }
        let excluded_source_ids = request.excluded_source_ids.iter().collect::<BTreeSet<_>>();
        if excluded_source_ids.len() != request.excluded_source_ids.len()
            || excluded_source_ids.len() > MAX_SELECTIONS
        {
            return Err(CoreError::invalid_input(
                "privacy execution source exclusions are invalid",
            ));
        }
        let operation = OperationGuard::start(CoordinatedOperationKind::PrivacyExecution)?;
        let started_at_ms = now_ms();
        let mut pending = take_pending_plan(&request.plan_id)?;
        if now_ms() > pending.public_plan.expires_at_ms {
            return Err(CoreError::invalid_input(
                "privacy execution plan has expired",
            ));
        }
        let excludable_source_ids = pending
            .public_plan
            .browser_close_requirements
            .iter()
            .map(|requirement| requirement.source_id.as_str())
            .collect::<BTreeSet<_>>();
        if excluded_source_ids
            .iter()
            .any(|source_id| !excludable_source_ids.contains(source_id.as_str()))
        {
            return Err(CoreError::invalid_input(
                "privacy execution excludes an unauthorized source",
            ));
        }
        pending
            .candidates
            .retain(|candidate| !excluded_source_ids.contains(&candidate.item.source_id));
        let total_item_count = pending.candidates.len() as u64;
        update(PrivacyExecutionProgress {
            stage: PrivacyExecutionStage::Validating,
            current_token: None,
            current_source_name: None,
            current_kind: None,
            completed_item_count: 0,
            total_item_count,
            affected_item_count: 0,
            elapsed_ms: now_ms().saturating_sub(started_at_ms),
            completed_items: Vec::new(),
        });
        let running = current_platform()
            .running_process_names()?
            .into_iter()
            .map(|name| process_leaf(&name).to_lowercase())
            .collect::<BTreeSet<_>>();

        // Closing a browser can flush its final WAL or session state. Refresh only the already
        // authorized candidates whose owning process is now closed. This keeps destructive
        // preflight fail-closed without running another catalog scan before the result dialog.
        let mut refreshed_candidate_count = 0_u64;
        for candidate in &mut pending.candidates {
            let was_running = matches!(
                candidate.item.capability,
                PrivacyCapabilityState::BrowserRunning | PrivacyCapabilityState::ApplicationRunning
            );
            let is_running = candidate
                .browser_process_names
                .iter()
                .any(|name| running.contains(&name.to_lowercase()));
            if was_running && !is_running {
                candidate.fingerprint = current_fingerprint(candidate)?;
                candidate.item.capability = PrivacyCapabilityState::Ready;
                refreshed_candidate_count = refreshed_candidate_count.saturating_add(1);
            }
        }
        if refreshed_candidate_count > 0 || !excluded_source_ids.is_empty() {
            log::info!(
                "privacy_execution_selection_refreshed operation_id={} refreshed_candidate_count={} excluded_source_count={}",
                operation.id(),
                refreshed_candidate_count,
                excluded_source_ids.len()
            );
        }

        // Validate the complete plan before the first mutation. Without this pass, an early item
        // could be cleared before a later source is found to be running or changed since scan.
        // The second per-item fingerprint below remains necessary to cover drift during execution.
        let preflight_failures = pending
            .candidates
            .iter()
            .map(|candidate| {
                if operation.ensure_not_cancelled().is_err() {
                    Some("operationCancelled")
                } else if candidate
                    .browser_process_names
                    .iter()
                    .any(|name| running.contains(&name.to_lowercase()))
                {
                    Some("browserRunning")
                } else if !matches!(current_fingerprint(candidate), Ok(ref value) if value == &candidate.fingerprint)
                {
                    Some("sourceChanged")
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if preflight_failures.iter().any(Option::is_some) {
            for (candidate, failure) in pending.candidates.iter().zip(&preflight_failures) {
                if let Some(reason) = failure {
                    log_privacy_item_failure(
                        operation.id(),
                        candidate,
                        "preflight",
                        reason,
                        None,
                        0,
                    );
                }
            }
            let results = pending
                .candidates
                .iter()
                .zip(preflight_failures)
                .map(|(candidate, failure)| {
                    if let Some(reason) = failure {
                        execution_result(
                            candidate,
                            if reason == "operationCancelled" {
                                PrivacyExecutionItemStatus::Cancelled
                            } else {
                                PrivacyExecutionItemStatus::Failed
                            },
                            0,
                            false,
                            Some(reason),
                        )
                    } else {
                        execution_result(
                            candidate,
                            PrivacyExecutionItemStatus::Cancelled,
                            0,
                            false,
                            Some("preflightBlocked"),
                        )
                    }
                })
                .collect::<Vec<_>>();
            let failed_item_count = results
                .iter()
                .filter(|item| item.status == PrivacyExecutionItemStatus::Failed)
                .count() as u64;
            let cancelled_item_count = results
                .iter()
                .filter(|item| item.status == PrivacyExecutionItemStatus::Cancelled)
                .count() as u64;
            update(PrivacyExecutionProgress {
                stage: PrivacyExecutionStage::Finalizing,
                current_token: None,
                current_source_name: None,
                current_kind: None,
                completed_item_count: total_item_count,
                total_item_count,
                affected_item_count: 0,
                elapsed_ms: now_ms().saturating_sub(started_at_ms),
                completed_items: execution_progress_items(&results),
            });
            append_history_record(
                request.plan_id.clone(),
                pending.time_range,
                &pending.candidates,
                &results,
                started_at_ms,
                now_ms(),
            );
            log::warn!(
                "privacy_execution_preflight_blocked operation_id={} failed_count={} cancelled_count={}",
                operation.id(),
                failed_item_count,
                cancelled_item_count
            );
            operation.complete();
            let scan = current_scan_session().ok().and_then(|session| {
                (session.public_result.scan_id == pending.public_plan.scan_id)
                    .then_some(session.public_result)
            });
            return Ok(PrivacyExecutionResult {
                plan_id: request.plan_id,
                affected_item_count: 0,
                failed_item_count,
                items: results,
                scan,
            });
        }
        let mut results: Vec<PrivacyExecutionItemResult> = Vec::new();
        let mut database_fingerprints = database_fingerprint_expectations(&pending.candidates);
        for candidate in &pending.candidates {
            update(PrivacyExecutionProgress {
                stage: PrivacyExecutionStage::Cleaning,
                current_token: Some(candidate.token.clone()),
                current_source_name: Some(candidate.item.source_name.clone()),
                current_kind: Some(candidate.item.kind),
                completed_item_count: results.len() as u64,
                total_item_count,
                affected_item_count: results.iter().map(|item| item.affected_item_count).sum(),
                elapsed_ms: now_ms().saturating_sub(started_at_ms),
                completed_items: execution_progress_items(&results),
            });
            if operation.ensure_not_cancelled().is_err() {
                results.push(execution_result(
                    candidate,
                    PrivacyExecutionItemStatus::Cancelled,
                    0,
                    false,
                    Some("operationCancelled"),
                ));
                continue;
            }
            // A browser can start after the plan-wide preflight. Refresh process state before each
            // browser mutation so a newly launched process cannot share the database transaction.
            match browser_is_running(candidate) {
                Ok(false) => {}
                Ok(true) => {
                    log_privacy_item_failure(
                        operation.id(),
                        candidate,
                        "execution",
                        "browser_running",
                        None,
                        0,
                    );
                    results.push(execution_result(
                        candidate,
                        PrivacyExecutionItemStatus::Failed,
                        0,
                        false,
                        Some("browserRunning"),
                    ));
                    continue;
                }
                Err(error) => {
                    log_privacy_item_failure(
                        operation.id(),
                        candidate,
                        "process_check",
                        "operation_failed",
                        Some(&error),
                        0,
                    );
                    results.push(execution_result(
                        candidate,
                        PrivacyExecutionItemStatus::Failed,
                        0,
                        false,
                        Some("operationFailed"),
                    ));
                    continue;
                }
            }
            if !fingerprint_matches_expectation(candidate, &database_fingerprints) {
                log_privacy_item_failure(
                    operation.id(),
                    candidate,
                    "execution",
                    "source_changed",
                    None,
                    0,
                );
                results.push(execution_result(
                    candidate,
                    PrivacyExecutionItemStatus::Failed,
                    0,
                    false,
                    Some("sourceChanged"),
                ));
                continue;
            }
            match execute_candidate(candidate) {
                Ok(affected) => {
                    // Multiple selected kinds can share one browser database. The first successful
                    // transaction legitimately changes that file, so later items must compare
                    // against the post-transaction fingerprint rather than the original scan. This
                    // refresh is scoped to the exact database path; unrelated sources retain their
                    // scan fingerprint and still fail closed on drift.
                    refresh_database_fingerprint_expectation(candidate, &mut database_fingerprints);
                    results.push(execution_result(
                        candidate,
                        if affected == 0 {
                            PrivacyExecutionItemStatus::Unchanged
                        } else {
                            PrivacyExecutionItemStatus::Cleared
                        },
                        affected,
                        true,
                        None,
                    ));
                }
                Err(failure) => {
                    let mutation_uncertain =
                        failure.error.mutation_state() == PlatformMutationState::MayHaveChanged;
                    let reason = if failure.confirmed_affected_item_count > 0 {
                        "partiallyCleared"
                    } else if mutation_uncertain {
                        "outcomeUncertain"
                    } else {
                        "operationFailed"
                    };
                    log_privacy_item_failure(
                        operation.id(),
                        candidate,
                        "mutation_or_verification",
                        reason,
                        Some(&failure.error),
                        failure.confirmed_affected_item_count,
                    );
                    results.push(execution_result(
                        candidate,
                        PrivacyExecutionItemStatus::Failed,
                        failure.confirmed_affected_item_count,
                        false,
                        Some(reason),
                    ));
                }
            }
        }
        let failed_item_count = results
            .iter()
            .filter(|item| item.status == PrivacyExecutionItemStatus::Failed)
            .count() as u64;
        let cancelled_item_count = results
            .iter()
            .filter(|item| item.status == PrivacyExecutionItemStatus::Cancelled)
            .count() as u64;
        let affected_item_count = results.iter().map(|item| item.affected_item_count).sum();
        let partial_failure_count = results
            .iter()
            .filter(|item| item.failure_reason.as_deref() == Some("partiallyCleared"))
            .count();
        let uncertain_failure_count = results
            .iter()
            .filter(|item| item.failure_reason.as_deref() == Some("outcomeUncertain"))
            .count();
        update(PrivacyExecutionProgress {
            stage: PrivacyExecutionStage::Finalizing,
            current_token: None,
            current_source_name: None,
            current_kind: None,
            completed_item_count: total_item_count,
            total_item_count,
            affected_item_count,
            elapsed_ms: now_ms().saturating_sub(started_at_ms),
            completed_items: execution_progress_items(&results),
        });
        append_history_record(
            request.plan_id.clone(),
            pending.time_range,
            &pending.candidates,
            &results,
            started_at_ms,
            now_ms(),
        );
        // A verified cleanup already gives Core enough evidence to update the visible snapshot.
        // Rebuilding the entire privacy catalog here makes a successful action feel like another
        // scan and needlessly rereads every application. Reconcile only verified executed rows;
        // untouched rows retain their original authority and therefore require a rescan if their
        // underlying source changed.
        let scan =
            reconcile_scan_after_execution(&pending.public_plan.scan_id, &results, operation.id());
        let operation_id = operation.id();
        let scan_reconciled = scan.is_some();
        log::info!(
            "privacy_execution_completed operation_id={operation_id} affected_count={affected_item_count} failed_count={failed_item_count} cancelled_count={cancelled_item_count} partial_failure_count={partial_failure_count} uncertain_failure_count={uncertain_failure_count} scan_reconciled={scan_reconciled}"
        );
        operation.complete();
        Ok(PrivacyExecutionResult {
            plan_id: request.plan_id,
            affected_item_count,
            failed_item_count,
            items: results,
            scan,
        })
    }
}

fn validate_browser_process_request(plan_id: &str, source_ids: &[String]) -> CoreResult<()> {
    if plan_id.len() < 16 || plan_id.len() > 128 {
        return Err(CoreError::invalid_input(
            "privacy plan identifier is invalid",
        ));
    }
    let selected_source_ids = source_ids.iter().collect::<BTreeSet<_>>();
    if selected_source_ids.is_empty() || selected_source_ids.len() != source_ids.len() {
        return Err(CoreError::invalid_input(
            "privacy browser process selection is invalid",
        ));
    }
    Ok(())
}

/// Resolves process names exclusively from the pending plan. Both close and status-refresh
/// requests share this boundary so a renderer-provided source ID can never become an arbitrary
/// process-control or process-inspection primitive.
fn resolve_browser_process_targets(
    plan_id: &str,
    source_ids: &[String],
) -> CoreResult<Vec<ResolvedApplicationCloseTarget>> {
    let pending = current_pending_plan(plan_id)?;
    if now_ms() > pending.public_plan.expires_at_ms {
        return Err(CoreError::invalid_input(
            "privacy execution plan has expired",
        ));
    }
    let allowed = pending
        .public_plan
        .browser_close_requirements
        .iter()
        .map(|requirement| {
            (
                requirement.source_id.as_str(),
                requirement.processes.as_slice(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    source_ids
        .iter()
        .map(|source_id| {
            let executable_names = allowed.get(source_id.as_str()).ok_or_else(|| {
                CoreError::invalid_input(
                    "privacy browser process request contains an unknown source",
                )
            })?;
            Ok(ResolvedApplicationCloseTarget {
                target_id: source_id.clone(),
                executable_names: executable_names.to_vec(),
                executable_paths: Vec::new(),
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn add_database_candidate(
    scan_nonce: &str,
    source_id: &str,
    source_name: &str,
    profile_id: &str,
    profile_name: &str,
    path: &Path,
    browser: PlatformPrivacyBrowserKind,
    kind: PrivacyDataKind,
    range: PrivacyTimeRange,
    observed_at_ms: u64,
    browser_running: bool,
    process_names: &[String],
    candidates: &mut BTreeMap<String, NativePrivacyCandidate>,
    items: &mut Vec<PrivacyItem>,
) {
    let token = candidate_token(scan_nonce, source_id, Some(profile_id), kind);
    let supports_time_range = !matches!(
        kind,
        PrivacyDataKind::SitePermissions
            | PrivacyDataKind::SavedPasswords
            | PrivacyDataKind::AutofillData
            | PrivacyDataKind::FrequentlyVisitedSites
            | PrivacyDataKind::WebsiteIcons
    ) || range == PrivacyTimeRange::AllTime;
    let file_access_error = fs::File::open(path).err();
    let source_busy = file_access_error
        .as_ref()
        .is_some_and(database_access_is_busy);
    let permission_denied = file_access_error.as_ref().is_some_and(|error| {
        error.kind() == std::io::ErrorKind::PermissionDenied && !database_access_is_busy(error)
    });
    let file_unavailable = file_access_error.is_some() && !permission_denied && !source_busy;
    // SQLite's read-only connection observes committed WAL records without copying browser data.
    // A short busy timeout bounds races with an active writer. Positive live results remain gated
    // by BrowserRunning so execution still requires process closure and a candidate-scoped refresh
    // before any mutation.
    let count = if !supports_time_range || permission_denied || file_unavailable || source_busy {
        Ok(0)
    } else {
        browser_database::count(path, browser, kind, range, observed_at_ms)
    };
    // A read-only SQLite query can still create or update WAL/SHM coordination files. Capture
    // the fingerprint after the query so the scan cannot make its own result look stale. A running
    // browser result is used only for review until execution refreshes that selected candidate.
    // Windows Chromium can deny every shared read while its network service owns the Cookie
    // database. Bind the close-browser plan to safe file metadata without reading contents; this
    // fingerprint can authorize only the close prompt and is replaced before mutation.
    let fingerprint = if source_busy && browser_running {
        deferred_database_fingerprint(path)
    } else {
        file_fingerprint(path)
    };
    let capability = match (&fingerprint, &count) {
        (Ok(_), _) if !supports_time_range => PrivacyCapabilityState::Unsupported,
        (_, _) if permission_denied => PrivacyCapabilityState::PermissionRequired,
        (Ok(_), _) if source_busy && browser_running => PrivacyCapabilityState::BrowserRunning,
        (_, _) if source_busy => PrivacyCapabilityState::Unavailable,
        (_, _) if file_unavailable => PrivacyCapabilityState::Unavailable,
        (Ok(_), Ok(0)) => PrivacyCapabilityState::Empty,
        (Ok(_), Ok(_)) if browser_running => PrivacyCapabilityState::BrowserRunning,
        (Ok(_), Ok(_)) => PrivacyCapabilityState::Ready,
        _ => PrivacyCapabilityState::SchemaUnsupported,
    };
    let item_count = count.unwrap_or(0);
    let estimated_bytes = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let item = privacy_item(PrivacyItemInput {
        token: token.clone(),
        source_id: source_id.into(),
        source_name: source_name.into(),
        profile_id: Some(profile_id.into()),
        profile_name: Some(profile_name.into()),
        kind,
        capability,
        item_count,
        estimated_bytes,
        requires_browser_close: true,
    });
    if let Ok(fingerprint) = fingerprint {
        candidates.insert(
            token.clone(),
            NativePrivacyCandidate {
                token,
                item: item.clone(),
                fingerprint,
                action: if item.recommendation == PrivacyRecommendation::ReviewOnly {
                    NativePrivacyAction::ReviewOnlyFile {
                        path: path.to_path_buf(),
                        browser,
                        kind,
                        range,
                        scan_now_ms: observed_at_ms,
                    }
                } else {
                    NativePrivacyAction::Database {
                        path: path.to_path_buf(),
                        browser,
                        kind,
                        range,
                        scan_now_ms: observed_at_ms,
                    }
                },
                browser_process_names: process_names.to_vec(),
            },
        );
    }
    items.push(item);
}

fn privacy_item_is_actionable(item: &PrivacyItem) -> bool {
    matches!(
        item.capability,
        PrivacyCapabilityState::Ready
            | PrivacyCapabilityState::BrowserRunning
            | PrivacyCapabilityState::ApplicationRunning
    ) && (item.item_count > 0
        || matches!(
            item.capability,
            PrivacyCapabilityState::BrowserRunning | PrivacyCapabilityState::ApplicationRunning
        ))
}

fn database_access_is_busy(error: &std::io::Error) -> bool {
    #[cfg(windows)]
    {
        // ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION indicate a live owner, not a missing
        // privacy permission. Chromium's Windows network service commonly uses the former.
        matches!(error.raw_os_error(), Some(32 | 33))
    }
    #[cfg(not(windows))]
    {
        error.kind() == std::io::ErrorKind::WouldBlock
    }
}

fn deferred_database_fingerprint(path: &Path) -> CoreResult<String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CoreError::operation_failed(format!(
            "privacy deferred database identity failed kind={:?}",
            error.kind()
        ))
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CoreError::operation_failed(
            "privacy deferred database identity is unsafe",
        ));
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"privacy-deferred-database-v1");
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(&metadata.len().to_le_bytes());
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    hasher.update(&modified_ns.to_le_bytes());
    Ok(hasher.finalize().to_hex().to_string())
}

#[allow(clippy::too_many_arguments)]
fn add_directory_candidate(
    scan_nonce: &str,
    source_id: &str,
    source_name: &str,
    profile_id: &str,
    profile_name: &str,
    kind: PrivacyDataKind,
    roots: Vec<PathBuf>,
    range: PrivacyTimeRange,
    browser_running: bool,
    process_names: &[String],
    cancellation: &PlatformCancellation,
    candidates: &mut BTreeMap<String, NativePrivacyCandidate>,
    items: &mut Vec<PrivacyItem>,
) {
    if roots.is_empty() {
        return;
    }
    let token = candidate_token(scan_nonce, source_id, Some(profile_id), kind);
    let snapshot = roots_summary_fingerprint(&roots, cancellation);
    let (capability, count, bytes, fingerprint) = match snapshot {
        Ok((fingerprint, count, bytes)) if range != PrivacyTimeRange::AllTime => (
            PrivacyCapabilityState::Unsupported,
            count,
            bytes,
            Some(fingerprint),
        ),
        Ok((fingerprint, 0, bytes)) => (PrivacyCapabilityState::Empty, 0, bytes, Some(fingerprint)),
        Ok((fingerprint, count, bytes)) if browser_running => (
            PrivacyCapabilityState::BrowserRunning,
            count,
            bytes,
            Some(fingerprint),
        ),
        Ok((fingerprint, count, bytes)) => (
            PrivacyCapabilityState::Ready,
            count,
            bytes,
            Some(fingerprint),
        ),
        Err(error) if error.code() == CoreErrorCode::PermissionDenied => {
            (PrivacyCapabilityState::PermissionRequired, 0, 0, None)
        }
        Err(_) => (PrivacyCapabilityState::Unavailable, 0, 0, None),
    };
    let item = privacy_item(PrivacyItemInput {
        token: token.clone(),
        source_id: source_id.into(),
        source_name: source_name.into(),
        profile_id: Some(profile_id.into()),
        profile_name: Some(profile_name.into()),
        kind,
        capability,
        item_count: count,
        estimated_bytes: bytes,
        requires_browser_close: true,
    });
    if let Some(fingerprint) = fingerprint {
        candidates.insert(
            token.clone(),
            NativePrivacyCandidate {
                token,
                item: item.clone(),
                fingerprint,
                action: NativePrivacyAction::Directories { roots },
                browser_process_names: process_names.to_vec(),
            },
        );
    }
    items.push(item);
}

fn privacy_item(input: PrivacyItemInput) -> PrivacyItem {
    let PrivacyItemInput {
        token,
        source_id,
        source_name,
        profile_id,
        profile_name,
        kind,
        capability,
        item_count,
        estimated_bytes,
        requires_browser_close,
    } = input;
    let (category, sensitivity, impact, recommendation, synchronization_may_propagate) = match kind
    {
        PrivacyDataKind::BrowsingHistory => (
            PrivacyCategory::BrowserActivity,
            PrivacySensitivity::Activity,
            // Removing activity history does not sign the user out or delete downloaded content.
            // Synchronization impact remains visible in the row and confirmation, but this low-
            // impact trace follows the same recommendation policy as other browser history.
            PrivacyImpact::Low,
            PrivacyRecommendation::Recommended,
            true,
        ),
        PrivacyDataKind::DownloadHistory => (
            PrivacyCategory::BrowserActivity,
            PrivacySensitivity::Activity,
            PrivacyImpact::Low,
            PrivacyRecommendation::Recommended,
            false,
        ),
        PrivacyDataKind::Cookies => (
            PrivacyCategory::BrowserAccountState,
            PrivacySensitivity::AccountState,
            PrivacyImpact::SignOut,
            PrivacyRecommendation::Manual,
            true,
        ),
        PrivacyDataKind::SiteStorage => (
            PrivacyCategory::BrowserAccountState,
            PrivacySensitivity::AccountState,
            PrivacyImpact::SignOut,
            PrivacyRecommendation::Manual,
            true,
        ),
        PrivacyDataKind::SitePermissions => (
            PrivacyCategory::BrowserAccountState,
            PrivacySensitivity::AccountState,
            PrivacyImpact::SignOut,
            PrivacyRecommendation::Manual,
            true,
        ),
        PrivacyDataKind::Sessions => (
            PrivacyCategory::BrowserActivity,
            PrivacySensitivity::ContentDerived,
            // Session traces do not remove bookmarks, downloads, or account data. The browser-
            // close confirmation still gives users a final chance to preserve open work.
            PrivacyImpact::Low,
            PrivacyRecommendation::Recommended,
            false,
        ),
        PrivacyDataKind::BrowserCache => (
            PrivacyCategory::BrowserActivity,
            PrivacySensitivity::ContentDerived,
            PrivacyImpact::Low,
            // Browser caches are regenerated and contain copies derived from visited pages. They
            // are safe to recommend while still requiring the owning browser to be closed.
            PrivacyRecommendation::Recommended,
            false,
        ),
        PrivacyDataKind::SearchHistory => (
            PrivacyCategory::BrowserActivity,
            PrivacySensitivity::Activity,
            PrivacyImpact::Low,
            // Search terms may participate in browser synchronization, which remains explicit in
            // the item description and confirmation without changing their low-impact selection.
            PrivacyRecommendation::Recommended,
            true,
        ),
        PrivacyDataKind::WebsiteIcons => (
            PrivacyCategory::BrowserActivity,
            PrivacySensitivity::ContentDerived,
            PrivacyImpact::Low,
            PrivacyRecommendation::Recommended,
            false,
        ),
        PrivacyDataKind::FrequentlyVisitedSites => (
            PrivacyCategory::BrowserActivity,
            PrivacySensitivity::Activity,
            // Removing the new-tab ranking changes convenience rather than account state. Keep it
            // manual because the visible shortcut layout is still part of the user's workflow.
            PrivacyImpact::Workflow,
            PrivacyRecommendation::Manual,
            false,
        ),
        PrivacyDataKind::AddressBarShortcuts => (
            PrivacyCategory::BrowserActivity,
            PrivacySensitivity::Activity,
            PrivacyImpact::Low,
            PrivacyRecommendation::Recommended,
            false,
        ),
        PrivacyDataKind::SavedPasswords | PrivacyDataKind::AutofillData => (
            PrivacyCategory::BrowserAccountState,
            PrivacySensitivity::PersonalContent,
            PrivacyImpact::DataLoss,
            // Personal browser data is never selected automatically. It remains available as an
            // explicit user choice and requires high-impact confirmation before Core execution.
            PrivacyRecommendation::Manual,
            true,
        ),
        PrivacyDataKind::CurrentClipboard => (
            PrivacyCategory::SystemActivity,
            PrivacySensitivity::ContentDerived,
            PrivacyImpact::Workflow,
            PrivacyRecommendation::Manual,
            false,
        ),
        PrivacyDataKind::ClipboardHistory => (
            PrivacyCategory::SystemActivity,
            PrivacySensitivity::ContentDerived,
            PrivacyImpact::Workflow,
            PrivacyRecommendation::Manual,
            false,
        ),
        PrivacyDataKind::RecentItems => (
            PrivacyCategory::SystemActivity,
            PrivacySensitivity::Activity,
            PrivacyImpact::Low,
            PrivacyRecommendation::Recommended,
            false,
        ),
        PrivacyDataKind::RecentApplications
        | PrivacyDataKind::ApplicationUsageHistory
        | PrivacyDataKind::NetworkConnectionHistory
        | PrivacyDataKind::PrinterHistory => (
            PrivacyCategory::SystemActivity,
            PrivacySensitivity::Activity,
            PrivacyImpact::Low,
            PrivacyRecommendation::Recommended,
            false,
        ),
        PrivacyDataKind::FolderViewHistory => (
            PrivacyCategory::SystemActivity,
            PrivacySensitivity::Activity,
            // Shell folder-view history also owns layout preferences. Clearing it improves privacy
            // but can reset view modes or desktop icon placement, so it requires explicit choice.
            PrivacyImpact::Workflow,
            PrivacyRecommendation::Manual,
            false,
        ),
        PrivacyDataKind::ShellHistory => (
            PrivacyCategory::SystemActivity,
            PrivacySensitivity::PersonalContent,
            // Command histories can contain paths, host names, and arguments. They are valuable
            // privacy traces but clearing them removes a user's searchable workflow history.
            PrivacyImpact::Workflow,
            PrivacyRecommendation::Manual,
            false,
        ),
        PrivacyDataKind::JumpLists => (
            PrivacyCategory::SystemActivity,
            PrivacySensitivity::Activity,
            PrivacyImpact::Low,
            PrivacyRecommendation::Recommended,
            false,
        ),
        PrivacyDataKind::RunDialogHistory
        | PrivacyDataKind::FileDialogHistory
        | PrivacyDataKind::SystemSearchHistory
        | PrivacyDataKind::ExplorerPathHistory => (
            PrivacyCategory::SystemActivity,
            PrivacySensitivity::Activity,
            PrivacyImpact::Low,
            PrivacyRecommendation::Recommended,
            false,
        ),
        PrivacyDataKind::ApplicationCache | PrivacyDataKind::ApplicationLogs => (
            PrivacyCategory::ApplicationActivity,
            PrivacySensitivity::ContentDerived,
            PrivacyImpact::Low,
            PrivacyRecommendation::Recommended,
            false,
        ),
        PrivacyDataKind::ApplicationSessions => (
            PrivacyCategory::ApplicationActivity,
            PrivacySensitivity::AccountState,
            // Session storage can restore an application's workflow and may also hold transient
            // sign-in state. It remains user-selected even though the source is fully clearable.
            PrivacyImpact::Workflow,
            PrivacyRecommendation::Manual,
            false,
        ),
        PrivacyDataKind::EditorLocalHistory => (
            PrivacyCategory::ApplicationActivity,
            PrivacySensitivity::PersonalContent,
            // Editor local history may be the only remaining copy of unsaved work. Exposing it as
            // a clearable item preserves user choice, but it must never be selected by default.
            PrivacyImpact::DataLoss,
            PrivacyRecommendation::Manual,
            false,
        ),
        PrivacyDataKind::RecentProjects => (
            PrivacyCategory::ApplicationActivity,
            PrivacySensitivity::Activity,
            PrivacyImpact::Workflow,
            PrivacyRecommendation::Manual,
            false,
        ),
        PrivacyDataKind::RecentDocuments
        | PrivacyDataKind::RecentConnections
        | PrivacyDataKind::PlaybackHistory
        | PrivacyDataKind::RecentPaths
        | PrivacyDataKind::RecentSearches => (
            PrivacyCategory::ApplicationActivity,
            PrivacySensitivity::Activity,
            PrivacyImpact::Low,
            PrivacyRecommendation::Recommended,
            false,
        ),
    };
    let selected_by_default = recommendation == PrivacyRecommendation::Recommended
        && matches!(
            capability,
            PrivacyCapabilityState::Ready
                | PrivacyCapabilityState::BrowserRunning
                | PrivacyCapabilityState::ApplicationRunning
        )
        && item_count > 0;
    PrivacyItem {
        token,
        source_id,
        source_name,
        profile_id,
        profile_name,
        category,
        kind,
        sensitivity,
        impact,
        recommendation,
        capability,
        item_count,
        estimated_bytes,
        selected_by_default,
        requires_browser_close,
        synchronization_may_propagate,
    }
}

fn application_data_kind(kind: PlatformPrivacyApplicationTraceKind) -> PrivacyDataKind {
    match kind {
        PlatformPrivacyApplicationTraceKind::Cache => PrivacyDataKind::ApplicationCache,
        PlatformPrivacyApplicationTraceKind::Logs => PrivacyDataKind::ApplicationLogs,
        PlatformPrivacyApplicationTraceKind::Sessions => PrivacyDataKind::ApplicationSessions,
        PlatformPrivacyApplicationTraceKind::EditorLocalHistory => {
            PrivacyDataKind::EditorLocalHistory
        }
        PlatformPrivacyApplicationTraceKind::RecentDocuments => PrivacyDataKind::RecentDocuments,
        PlatformPrivacyApplicationTraceKind::RecentProjects => PrivacyDataKind::RecentProjects,
        PlatformPrivacyApplicationTraceKind::RecentConnections => {
            PrivacyDataKind::RecentConnections
        }
        PlatformPrivacyApplicationTraceKind::PlaybackHistory => PrivacyDataKind::PlaybackHistory,
        PlatformPrivacyApplicationTraceKind::RecentPaths => PrivacyDataKind::RecentPaths,
        PlatformPrivacyApplicationTraceKind::RecentSearches => PrivacyDataKind::RecentSearches,
    }
}

fn application_availability_capability(
    availability: PlatformPrivacyApplicationTraceAvailability,
) -> Option<PrivacyCapabilityState> {
    match availability {
        PlatformPrivacyApplicationTraceAvailability::Available => None,
        PlatformPrivacyApplicationTraceAvailability::PermissionRequired => {
            Some(PrivacyCapabilityState::PermissionRequired)
        }
        PlatformPrivacyApplicationTraceAvailability::Unavailable => {
            Some(PrivacyCapabilityState::Unavailable)
        }
    }
}

fn source_coverage_capability(
    capabilities: &[PrivacyCapabilityState],
    item_count: u64,
) -> PrivacyCapabilityState {
    for capability in [
        PrivacyCapabilityState::SchemaUnsupported,
        PrivacyCapabilityState::PermissionRequired,
        PrivacyCapabilityState::Unavailable,
        PrivacyCapabilityState::Unsupported,
        PrivacyCapabilityState::ApplicationRunning,
        PrivacyCapabilityState::BrowserRunning,
    ] {
        if capabilities.contains(&capability) {
            return capability;
        }
    }
    if item_count == 0 {
        PrivacyCapabilityState::Empty
    } else {
        PrivacyCapabilityState::Ready
    }
}

fn browser_close_requirements(
    candidates: &[NativePrivacyCandidate],
) -> Vec<PrivacyBrowserCloseRequirement> {
    let mut groups = BTreeMap::<String, (String, BTreeMap<String, String>)>::new();
    for candidate in candidates.iter().filter(|candidate| {
        matches!(
            candidate.item.capability,
            PrivacyCapabilityState::BrowserRunning | PrivacyCapabilityState::ApplicationRunning
        )
    }) {
        let entry = groups
            .entry(candidate.item.source_id.clone())
            .or_insert_with(|| (candidate.item.source_name.clone(), BTreeMap::new()));
        for process in &candidate.browser_process_names {
            entry
                .1
                .entry(process_leaf(process).to_lowercase())
                .or_insert_with(|| process.clone());
        }
    }
    groups
        .into_iter()
        .map(
            |(source_id, (source_name, processes))| PrivacyBrowserCloseRequirement {
                source_id,
                source_name,
                processes: processes.into_values().collect(),
            },
        )
        .collect()
}

fn execute_candidate(candidate: &NativePrivacyCandidate) -> Result<u64, PrivacyMutationFailure> {
    match &candidate.action {
        NativePrivacyAction::Database {
            path,
            browser,
            kind,
            range,
            scan_now_ms,
        } => {
            browser_database::clear(path, *browser, *kind, *range, *scan_now_ms).map_err(Into::into)
        }
        NativePrivacyAction::Directories { roots } => clear_directories(roots),
        NativePrivacyAction::ReviewOnlyFile { .. } => Err(CoreError::operation_failed(
            "review-only privacy evidence cannot be executed",
        )
        .into()),
        NativePrivacyAction::System {
            kind:
                kind @ (PlatformPrivacySystemTraceKind::RecentItems
                | PlatformPrivacySystemTraceKind::RecentDocumentHistory),
            roots,
            has_native_revision,
        } => {
            let mut removed = 0_u64;
            for root in roots {
                match clear_recent_items(root) {
                    Ok(root_removed) => removed = removed.saturating_add(root_removed),
                    Err(mut failure) => {
                        failure.confirmed_affected_item_count = failure
                            .confirmed_affected_item_count
                            .saturating_add(removed);
                        if removed > 0 {
                            failure.error = failure.error.with_possible_side_effects();
                        }
                        return Err(failure);
                    }
                }
            }
            let native_verified = if *kind == PlatformPrivacySystemTraceKind::RecentDocumentHistory
                && *has_native_revision
            {
                Some(
                    current_platform()
                        .clear_system_privacy_trace(
                            PlatformPrivacySystemTraceKind::RecentDocumentHistory,
                        )
                        .map_err(|error| {
                            let mut failure = PrivacyMutationFailure::from(error);
                            failure.confirmed_affected_item_count = removed;
                            if removed > 0 {
                                failure = failure.with_possible_side_effects();
                            }
                            failure
                        })?,
                )
            } else {
                None
            };
            if native_verified == Some(false) {
                return Err(PrivacyMutationFailure::new(
                    CoreError::operation_failed("recent document registry verification failed")
                        .with_possible_side_effects(),
                    removed,
                ));
            }
            Ok(removed)
        }
        NativePrivacyAction::System { roots, .. } if !roots.is_empty() => {
            let removed = clear_directories(roots)?;
            Ok(if removed == 0 {
                0
            } else {
                candidate.item.item_count
            })
        }
        NativePrivacyAction::System { kind, .. } => {
            let verified = current_platform().clear_system_privacy_trace(*kind)?;
            if !verified {
                return Err(CoreError::operation_failed(
                    "privacy native action verification failed",
                )
                .with_possible_side_effects()
                .into());
            }
            if *kind != PlatformPrivacySystemTraceKind::ClipboardHistory {
                return Ok(candidate.item.item_count);
            }

            // Windows preserves pinned clipboard-history entries by contract. Recounting after the
            // native call reports only entries actually removed instead of claiming pinned data was
            // cleared. The platform adapter hashes IDs and never reads clipboard content.
            let cancellation = PlatformCancellation::new(|| false);
            let discovery = current_platform()
                .discover_privacy_sources(&cancellation)
                .map_err(|error| {
                    PrivacyMutationFailure::from(error).with_possible_side_effects()
                })?;
            let remaining = discovery
                .system_traces
                .into_iter()
                .find(|trace| trace.kind == *kind && trace.available)
                .ok_or_else(|| {
                    PrivacyMutationFailure::from(CoreError::operation_failed(
                        "privacy native action post-verification is unavailable",
                    ))
                    .with_possible_side_effects()
                })?
                .item_count;
            Ok(candidate.item.item_count.saturating_sub(remaining))
        }
        NativePrivacyAction::ApplicationNative { kind } => {
            let verified = current_platform().clear_application_privacy_trace(*kind)?;
            if verified {
                Ok(candidate.item.item_count)
            } else {
                Err(CoreError::operation_failed(
                    "application privacy native action verification failed",
                )
                .with_possible_side_effects()
                .into())
            }
        }
    }
}

fn browser_is_running(candidate: &NativePrivacyCandidate) -> CoreResult<bool> {
    if candidate.browser_process_names.is_empty() {
        return Ok(false);
    }
    let running = current_platform()
        .running_process_names()?
        .into_iter()
        .map(|name| process_leaf(&name).to_lowercase())
        .collect::<BTreeSet<_>>();
    Ok(candidate
        .browser_process_names
        .iter()
        .any(|name| running.contains(&name.to_lowercase())))
}

fn current_fingerprint(candidate: &NativePrivacyCandidate) -> CoreResult<String> {
    match &candidate.action {
        NativePrivacyAction::Database { path, .. }
        | NativePrivacyAction::ReviewOnlyFile { path, .. } => file_fingerprint(path),
        NativePrivacyAction::Directories { roots } => {
            roots_summary_fingerprint(roots, &PlatformCancellation::new(|| false))
                .map(|value| value.0)
        }
        NativePrivacyAction::System {
            kind,
            roots,
            has_native_revision,
        } if !roots.is_empty() => {
            let root_revision =
                roots_summary_fingerprint(roots, &PlatformCancellation::new(|| false))?.0;
            let native_revision = if *has_native_revision {
                Some(
                    current_platform()
                        .system_privacy_trace_revision(*kind)?
                        .ok_or_else(|| {
                            CoreError::operation_failed(
                                "privacy system native revision is unavailable",
                            )
                        })?,
                )
            } else {
                None
            };
            Ok(system_trace_fingerprint(
                &root_revision,
                native_revision.as_deref(),
            ))
        }
        NativePrivacyAction::System { kind, .. } => {
            let cancellation = PlatformCancellation::new(|| false);
            let discovery = current_platform().discover_privacy_sources(&cancellation)?;
            let current = discovery
                .system_traces
                .into_iter()
                .find(|trace| trace.kind == *kind)
                .ok_or_else(|| {
                    CoreError::operation_failed("privacy system source is unavailable")
                })?;
            Ok(current.revision)
        }
        NativePrivacyAction::ApplicationNative { kind } => {
            let cancellation = PlatformCancellation::new(|| false);
            let discovery = current_platform().discover_privacy_sources(&cancellation)?;
            discovery
                .applications
                .into_iter()
                .flat_map(|application| application.traces)
                .find(|trace| trace.native_kind == Some(*kind))
                .map(|trace| trace.revision)
                .ok_or_else(|| {
                    CoreError::operation_failed("application privacy source is unavailable")
                })
        }
    }
}

fn refresh_database_fingerprints(
    candidates: &mut BTreeMap<String, NativePrivacyCandidate>,
) -> CoreResult<()> {
    for candidate in candidates.values_mut() {
        match &candidate.action {
            NativePrivacyAction::Database { path, .. }
            | NativePrivacyAction::ReviewOnlyFile { path, .. } => {
                match file_fingerprint(path) {
                    Ok(fingerprint) => candidate.fingerprint = fingerprint,
                    Err(_)
                        if candidate.item.capability == PrivacyCapabilityState::BrowserRunning
                            && fs::File::open(path)
                                .err()
                                .as_ref()
                                .is_some_and(database_access_is_busy) =>
                    {
                        // Keep the metadata-only fingerprint created for an exclusively locked
                        // database. This candidate can only authorize the close prompt; Core
                        // replaces the deferred fingerprint after process closure and before
                        // destructive execution.
                    }
                    Err(error) => return Err(error),
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn database_fingerprint_expectations(
    candidates: &[NativePrivacyCandidate],
) -> BTreeMap<PathBuf, String> {
    candidates
        .iter()
        .filter_map(|candidate| match &candidate.action {
            NativePrivacyAction::Database { path, .. } => {
                Some((path.clone(), candidate.fingerprint.clone()))
            }
            _ => None,
        })
        .collect()
}

fn fingerprint_matches_expectation(
    candidate: &NativePrivacyCandidate,
    database_fingerprints: &BTreeMap<PathBuf, String>,
) -> bool {
    let expected = match &candidate.action {
        NativePrivacyAction::Database { path, .. } => database_fingerprints
            .get(path)
            .unwrap_or(&candidate.fingerprint),
        _ => &candidate.fingerprint,
    };
    matches!(current_fingerprint(candidate), Ok(ref value) if value == expected)
}

fn refresh_database_fingerprint_expectation(
    candidate: &NativePrivacyCandidate,
    database_fingerprints: &mut BTreeMap<PathBuf, String>,
) {
    if let NativePrivacyAction::Database { path, .. } = &candidate.action {
        if let Ok(fingerprint) = file_fingerprint(path) {
            database_fingerprints.insert(path.clone(), fingerprint);
        }
    }
}

fn file_fingerprint(path: &Path) -> CoreResult<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(path.to_string_lossy().as_bytes());
    update_file_fingerprint(&mut hasher, path)?;
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", path.to_string_lossy()));
        hasher.update(suffix.as_bytes());
        match fs::symlink_metadata(&sidecar) {
            Ok(_) => {
                hasher.update(&[1]);
                update_file_fingerprint(&mut hasher, &sidecar)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(&[0]);
            }
            Err(error) => return Err(privacy_io_error("privacy metadata unavailable", error)),
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn update_file_fingerprint(hasher: &mut blake3::Hasher, path: &Path) -> CoreResult<()> {
    const SAMPLE_BYTES: usize = 4096;

    let mut file = open_fingerprint_file(path)
        .map_err(|error| privacy_io_error("privacy fingerprint open failed", error))?;
    let before = file
        .metadata()
        .map_err(|error| privacy_io_error("privacy fingerprint metadata failed", error))?;
    if !before.is_file() || current_platform().is_link_like(&before) {
        return Err(CoreError::operation_failed(
            "privacy database identity is unsafe",
        ));
    }
    let modified = before
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_nanos());
    hasher.update(&before.len().to_le_bytes());
    hasher.update(&modified.to_le_bytes());

    // SQLite updates its file-change counter in the first page on every main-database commit,
    // while WAL commits append frames at the end. Hashing both bounded regions detects those
    // changes even on filesystems whose timestamp resolution is too coarse for rapid writes.
    let head_bytes = before.len().min(SAMPLE_BYTES as u64) as usize;
    let mut sample = vec![0_u8; head_bytes];
    file.read_exact(&mut sample)
        .map_err(|error| privacy_io_error("privacy fingerprint read failed", error))?;
    hasher.update(&sample);
    if before.len() > SAMPLE_BYTES as u64 {
        let tail_offset = before.len().saturating_sub(SAMPLE_BYTES as u64);
        file.seek(SeekFrom::Start(tail_offset))
            .map_err(|error| privacy_io_error("privacy fingerprint seek failed", error))?;
        sample.resize(SAMPLE_BYTES, 0);
        file.read_exact(&mut sample)
            .map_err(|error| privacy_io_error("privacy fingerprint read failed", error))?;
        hasher.update(&sample);
    }

    let after = file
        .metadata()
        .map_err(|error| privacy_io_error("privacy fingerprint metadata failed", error))?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(CoreError::operation_failed(
            "privacy database changed while fingerprinting",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn open_fingerprint_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(windows)]
fn open_fingerprint_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

/// Captures only root and direct-child metadata during an ordinary scan.
///
/// Browser storage can contain hundreds of thousands of cache entries. Recursive safety
/// validation is still mandatory immediately before deletion, but doing it during every read-only
/// scan would make the page unusable and cancellation sluggish. This bounded summary is sufficient
/// for scan-to-plan drift detection; execution performs the complete link-aware traversal.
fn roots_summary_fingerprint(
    roots: &[PathBuf],
    cancellation: &PlatformCancellation,
) -> CoreResult<(String, u64, u64)> {
    let mut facts = Vec::new();
    let mut count = 0;
    let mut bytes = 0;
    for root in roots {
        if cancellation.is_cancelled() {
            return Err(CoreError::operation_cancelled());
        }
        if facts.len() as u64 >= MAX_DIRECTORY_SCAN_ENTRIES {
            return Err(CoreError::operation_failed(
                "privacy directory exceeds the bounded scan limit",
            ));
        }
        let metadata = fs::symlink_metadata(root)
            .map_err(|error| privacy_io_error("privacy directory metadata unavailable", error))?;
        if current_platform().is_link_like(&metadata) {
            return Err(CoreError::operation_failed(
                "privacy directory contains an unsafe root link",
            ));
        }
        facts.push((
            root.to_string_lossy().into_owned(),
            metadata.len(),
            metadata.is_dir(),
            modified_at_nanos(&metadata),
        ));
        if metadata.is_dir() {
            for entry in fs::read_dir(root)
                .map_err(|error| privacy_io_error("privacy directory is unreadable", error))?
            {
                let entry = entry.map_err(|error| {
                    privacy_io_error("privacy directory entry is unreadable", error)
                })?;
                if facts.len() as u64 >= MAX_DIRECTORY_SCAN_ENTRIES {
                    return Err(CoreError::operation_failed(
                        "privacy directory exceeds the bounded scan limit",
                    ));
                }
                let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
                    privacy_io_error("privacy entry metadata unavailable", error)
                })?;
                if current_platform().is_link_like(&metadata) {
                    return Err(CoreError::operation_failed(
                        "privacy directory contains a link-like entry",
                    ));
                }
                facts.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    metadata.len(),
                    metadata.is_dir(),
                    modified_at_nanos(&metadata),
                ));
                count += 1;
                if metadata.is_file() {
                    bytes += metadata.len();
                }
            }
        } else {
            count += 1;
            bytes += metadata.len();
        }
    }
    facts.sort();
    let mut hasher = blake3::Hasher::new();
    for (name, len, directory, modified) in facts {
        hasher.update(name.as_bytes());
        hasher.update(&len.to_le_bytes());
        hasher.update(&[u8::from(directory)]);
        hasher.update(&modified.to_le_bytes());
    }
    Ok((hasher.finalize().to_hex().to_string(), count, bytes))
}

fn modified_at_nanos(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_nanos())
}

fn system_trace_fingerprint(root_revision: &str, native_revision: Option<&str>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-system-privacy-trace-v1\0");
    hasher.update(root_revision.as_bytes());
    if let Some(native_revision) = native_revision {
        hasher.update(b"\0native\0");
        hasher.update(native_revision.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

/// Lists exactly the same root or direct-child units used by the bounded aggregate scan.
///
/// Detail requests are explicit local user actions, so an available full path is more useful than
/// an ambiguous leaf name. Paths are never logged or persisted and remain bounded before crossing
/// the local adapter boundary.
fn root_detail_entries(
    roots: &[PathBuf],
    offset: u64,
    limit: u32,
) -> CoreResult<Vec<super::PrivacyDetailEntry>> {
    root_detail_entries_matching(roots, offset, limit, |_| true)
}

/// Uses the same logical units as platform discovery for file-backed system traces.
///
/// Windows Recent Items and Jump Lists may contain housekeeping files beside the actual privacy
/// records. Filtering those entries keeps the detail count aligned with the aggregate shown to the
/// user instead of making the preview appear larger than the scan result.
fn system_root_detail_entries(
    kind: PlatformPrivacySystemTraceKind,
    roots: &[PathBuf],
    offset: u64,
    limit: u32,
) -> CoreResult<Vec<super::PrivacyDetailEntry>> {
    match kind {
        PlatformPrivacySystemTraceKind::ShellHistory => {
            text_record_detail_entries(roots, offset, limit)
        }
        PlatformPrivacySystemTraceKind::RecentItems
        | PlatformPrivacySystemTraceKind::RecentDocumentHistory => {
            root_detail_entries_matching(roots, offset, limit, |path| {
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
            })
        }
        PlatformPrivacySystemTraceKind::JumpLists => {
            root_detail_entries_matching(roots, offset, limit, |path| {
                path.extension().is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("automaticDestinations-ms")
                        || extension.eq_ignore_ascii_case("customDestinations-ms")
                })
            })
        }
        _ => root_detail_entries(roots, offset, limit),
    }
}

fn root_detail_entries_matching(
    roots: &[PathBuf],
    offset: u64,
    limit: u32,
    include: impl Fn(&Path) -> bool,
) -> CoreResult<Vec<super::PrivacyDetailEntry>> {
    let mut entries = Vec::new();
    for root in roots {
        let metadata = fs::symlink_metadata(root)
            .map_err(|error| privacy_io_error("privacy detail root is unavailable", error))?;
        if current_platform().is_link_like(&metadata) {
            return Err(CoreError::operation_failed(
                "privacy detail root is link-like",
            ));
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(root).map_err(|error| {
                privacy_io_error("privacy detail directory is unreadable", error)
            })? {
                let entry = entry.map_err(|error| {
                    privacy_io_error("privacy detail entry is unreadable", error)
                })?;
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).map_err(|error| {
                    privacy_io_error("privacy detail entry metadata is unavailable", error)
                })?;
                if current_platform().is_link_like(&metadata) {
                    return Err(CoreError::operation_failed(
                        "privacy detail directory contains a link-like entry",
                    ));
                }
                if !include(&path) {
                    continue;
                }
                if entries.len() >= MAX_DETAIL_LIST_ENTRIES {
                    return Err(CoreError::operation_failed(
                        "privacy detail source exceeds the bounded list limit",
                    ));
                }
                entries.push(super::PrivacyDetailEntry {
                    label: path_detail_label(&path),
                    item_count: 1,
                });
            }
        } else if metadata.is_file() && include(root) {
            if entries.len() >= MAX_DETAIL_LIST_ENTRIES {
                return Err(CoreError::operation_failed(
                    "privacy detail source exceeds the bounded list limit",
                ));
            }
            entries.push(super::PrivacyDetailEntry {
                label: path_detail_label(root),
                item_count: 1,
            });
        }
    }
    entries.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(page_entries(entries, offset, limit))
}

/// Shell histories are counted by logical text records rather than files, so their preview follows
/// the same unit. Lines are held only for this call, bounded by the same 100k-record scan limit,
/// sanitized for UI rendering, and never logged.
fn text_record_detail_entries(
    roots: &[PathBuf],
    offset: u64,
    limit: u32,
) -> CoreResult<Vec<super::PrivacyDetailEntry>> {
    let start = usize::try_from(offset).unwrap_or(usize::MAX);
    let take = limit as usize;
    let mut seen = 0_usize;
    let mut entries = Vec::new();
    for root in roots {
        let metadata = fs::symlink_metadata(root)
            .map_err(|error| privacy_io_error("privacy text detail is unavailable", error))?;
        if !metadata.is_file() || current_platform().is_link_like(&metadata) {
            return Err(CoreError::operation_failed(
                "privacy text detail source is unsafe",
            ));
        }
        let file = fs::File::open(root)
            .map_err(|error| privacy_io_error("privacy text detail is unreadable", error))?;
        let source_label = history_source_label(root);
        let mut reader = BufReader::new(file);
        let mut record = Vec::new();
        for _ in 0..MAX_DETAIL_OFFSET {
            record.clear();
            let bytes_read = reader.read_until(b'\n', &mut record).map_err(|error| {
                privacy_io_error("privacy text detail record is unreadable", error)
            })?;
            if bytes_read == 0 {
                break;
            }
            if seen >= start && entries.len() < take {
                entries.push(super::PrivacyDetailEntry {
                    label: history_record_label(source_label, &record),
                    item_count: 1,
                });
            }
            seen = seen.saturating_add(1);
            if entries.len() >= take {
                return Ok(entries);
            }
        }
    }
    Ok(entries)
}

fn history_source_label(path: &Path) -> &'static str {
    match path.file_name().and_then(|name| name.to_str()) {
        Some(".zsh_history") => "zsh",
        Some(".bash_history") => "Bash",
        Some(".sh_history") => "Shell",
        Some(".python_history") => "Python",
        Some(".node_repl_history") => "Node.js",
        Some(".psql_history") => "PostgreSQL",
        Some(".mysql_history") => "MySQL",
        Some(".sqlite_history") => "SQLite",
        Some("ConsoleHost_history.txt") => "PowerShell",
        _ => "Terminal",
    }
}

fn history_record_label(source: &str, record: &[u8]) -> String {
    let record = record.strip_suffix(b"\n").unwrap_or(record);
    let record = record.strip_suffix(b"\r").unwrap_or(record);
    let decoded = String::from_utf8_lossy(record);
    let command = strip_zsh_history_metadata(&decoded);
    let command = sanitize_detail_label(command);
    if command.is_empty() {
        source.to_owned()
    } else {
        sanitize_detail_label(&format!("{source} · {command}"))
    }
}

/// zsh's extended history starts records with `: <epoch>:<duration>;`. Removing only a prefix
/// that matches this exact shape keeps ordinary commands beginning with a colon untouched.
fn strip_zsh_history_metadata(value: &str) -> &str {
    let Some(metadata) = value.strip_prefix(": ") else {
        return value;
    };
    let Some((metadata, command)) = metadata.split_once(';') else {
        return value;
    };
    let mut fields = metadata.split(':');
    let timestamp = fields.next();
    let duration = fields.next();
    if fields.next().is_none()
        && timestamp.is_some_and(|field| {
            !field.is_empty() && field.bytes().all(|byte| byte.is_ascii_digit())
        })
        && duration.is_some_and(|field| {
            !field.is_empty() && field.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        command
    } else {
        value
    }
}

fn page_entries(
    entries: Vec<super::PrivacyDetailEntry>,
    offset: u64,
    limit: u32,
) -> Vec<super::PrivacyDetailEntry> {
    entries
        .into_iter()
        .skip(usize::try_from(offset).unwrap_or(usize::MAX))
        .take(limit as usize)
        .collect()
}

fn platform_detail_entries(
    entries: Vec<mangodisk_platform::PlatformPrivacyDetailEntry>,
) -> (PrivacyDetailsPresentation, Vec<super::PrivacyDetailEntry>) {
    if entries.is_empty() {
        return (PrivacyDetailsPresentation::AggregateOnly, Vec::new());
    }
    let entries = entries
        .into_iter()
        .map(|entry| super::PrivacyDetailEntry {
            label: sanitize_detail_label(&entry.label),
            item_count: entry.item_count,
        })
        .collect();
    (PrivacyDetailsPresentation::List, entries)
}

fn path_detail_label(path: &Path) -> String {
    sanitize_detail_label(&path.to_string_lossy())
}

fn sanitize_detail_label(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_DETAIL_LABEL_CHARS)
        .collect()
}

fn privacy_io_error(context: &str, error: std::io::Error) -> CoreError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        CoreError::new(
            CoreErrorCode::PermissionDenied,
            format!("{context}: {error}"),
        )
    } else {
        CoreError::operation_failed(format!("{context}: {error}"))
    }
}

fn privacy_mutation_failure(
    error: CoreError,
    confirmed_affected_item_count: u64,
    may_have_changed: bool,
) -> PrivacyMutationFailure {
    let error = if may_have_changed {
        error.with_possible_side_effects()
    } else {
        error
    };
    PrivacyMutationFailure::new(error, confirmed_affected_item_count)
}

fn tree_fingerprint_with_cancellation(
    root: &Path,
    cancellation: &PlatformCancellation,
) -> CoreResult<(String, u64, u64)> {
    let metadata = fs::symlink_metadata(root).map_err(|error| {
        CoreError::operation_failed(format!("privacy directory metadata unavailable: {error}"))
    })?;
    if current_platform().is_link_like(&metadata) {
        return Err(CoreError::operation_failed(
            "privacy directory contains an unsafe root link",
        ));
    }
    let mut entries = vec![root.to_path_buf()];
    let mut facts = Vec::new();
    let mut count = 0;
    let mut bytes = 0;
    while let Some(path) = entries.pop() {
        if cancellation.is_cancelled() {
            return Err(CoreError::operation_cancelled());
        }
        if facts.len() as u64 >= MAX_DIRECTORY_SCAN_ENTRIES {
            return Err(CoreError::operation_failed(
                "privacy directory exceeds the bounded scan limit",
            ));
        }
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            CoreError::operation_failed(format!("privacy entry metadata unavailable: {error}"))
        })?;
        if current_platform().is_link_like(&metadata) {
            return Err(CoreError::operation_failed(
                "privacy directory contains a link-like entry",
            ));
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        facts.push((relative, metadata.len(), metadata.is_dir()));
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).map_err(|error| {
                CoreError::operation_failed(format!("privacy directory is unreadable: {error}"))
            })? {
                entries.push(
                    entry
                        .map_err(|error| {
                            CoreError::operation_failed(format!(
                                "privacy directory entry is unreadable: {error}"
                            ))
                        })?
                        .path(),
                );
            }
        } else {
            count += 1;
            bytes += metadata.len();
        }
    }
    facts.sort();
    let mut hasher = blake3::Hasher::new();
    for (relative, len, directory) in facts {
        hasher.update(relative.as_bytes());
        hasher.update(&len.to_le_bytes());
        hasher.update(&[u8::from(directory)]);
    }
    Ok((hasher.finalize().to_hex().to_string(), count, bytes))
}

fn clear_directories(roots: &[PathBuf]) -> Result<u64, PrivacyMutationFailure> {
    // Capture each root's count before the first mutation. When a later root fails, the execution
    // result can still report the minimum number of entries whose removal already succeeded.
    let root_counts = roots
        .iter()
        .map(|root| {
            tree_fingerprint_with_cancellation(root, &PlatformCancellation::new(|| false))
                .map(|(_, count, _)| count)
        })
        .collect::<CoreResult<Vec<_>>>()
        .map_err(PrivacyMutationFailure::from)?;
    let count = root_counts.iter().copied().sum();
    let mut root_kinds = Vec::with_capacity(roots.len());
    let mut confirmed_affected_item_count = 0_u64;
    for (root, root_count) in roots.iter().zip(&root_counts) {
        let mut root_changed = false;
        // Re-read root metadata immediately before mutation. `Path::is_dir` follows links and
        // would allow a root swapped after preflight to redirect deletion outside the discovered
        // browser profile. Keeping the original root itself also matches browser expectations:
        // directory contents are cleared, while a session file is removed as one atomic item.
        let metadata = fs::symlink_metadata(root).map_err(|error| {
            privacy_mutation_failure(
                privacy_io_error("privacy root metadata unavailable", error),
                confirmed_affected_item_count,
                confirmed_affected_item_count > 0,
            )
        })?;
        if current_platform().is_link_like(&metadata) {
            return Err(privacy_mutation_failure(
                CoreError::operation_failed("privacy root changed to a link-like entry"),
                confirmed_affected_item_count,
                confirmed_affected_item_count > 0,
            ));
        }
        if metadata.is_dir() {
            root_kinds.push(true);
            let entries = fs::read_dir(root).map_err(|error| {
                privacy_mutation_failure(
                    CoreError::operation_failed(format!(
                        "privacy directory is unreadable: {error}"
                    )),
                    confirmed_affected_item_count,
                    confirmed_affected_item_count > 0,
                )
            })?;
            for entry in entries {
                let path = entry
                    .map_err(|error| {
                        privacy_mutation_failure(
                            CoreError::operation_failed(format!(
                                "privacy entry is unreadable: {error}"
                            )),
                            confirmed_affected_item_count,
                            root_changed || confirmed_affected_item_count > 0,
                        )
                    })?
                    .path();
                let metadata = fs::symlink_metadata(&path).map_err(|error| {
                    privacy_mutation_failure(
                        CoreError::operation_failed(format!(
                            "privacy entry metadata unavailable: {error}"
                        )),
                        confirmed_affected_item_count,
                        root_changed || confirmed_affected_item_count > 0,
                    )
                })?;
                if current_platform().is_link_like(&metadata) {
                    return Err(privacy_mutation_failure(
                        CoreError::operation_failed(
                            "privacy directory changed to a link-like entry",
                        ),
                        confirmed_affected_item_count,
                        root_changed || confirmed_affected_item_count > 0,
                    ));
                }
                let removal = if metadata.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                removal.map_err(|error| {
                    privacy_mutation_failure(
                        CoreError::operation_failed(format!(
                            "privacy entry removal failed: {error}"
                        )),
                        confirmed_affected_item_count,
                        true,
                    )
                })?;
                root_changed = true;
            }
        } else if metadata.is_file() {
            root_kinds.push(false);
            fs::remove_file(root).map_err(|error| {
                privacy_mutation_failure(
                    CoreError::operation_failed(format!("privacy entry removal failed: {error}")),
                    confirmed_affected_item_count,
                    true,
                )
            })?;
            root_changed = true;
        } else {
            return Err(privacy_mutation_failure(
                CoreError::operation_failed("privacy root changed to an unsupported file type"),
                confirmed_affected_item_count,
                confirmed_affected_item_count > 0,
            ));
        }
        if root_changed {
            confirmed_affected_item_count =
                confirmed_affected_item_count.saturating_add(*root_count);
        }
    }

    for (root, was_directory) in roots.iter().zip(root_kinds) {
        if was_directory {
            let metadata = fs::symlink_metadata(root).map_err(|error| {
                privacy_mutation_failure(
                    privacy_io_error("privacy directory verification failed", error),
                    confirmed_affected_item_count,
                    true,
                )
            })?;
            if !metadata.is_dir() || current_platform().is_link_like(&metadata) {
                return Err(privacy_mutation_failure(
                    CoreError::operation_failed("privacy directory verification failed"),
                    confirmed_affected_item_count,
                    true,
                ));
            }
            let mut entries = fs::read_dir(root).map_err(|error| {
                privacy_mutation_failure(
                    privacy_io_error("privacy directory verification failed", error),
                    confirmed_affected_item_count,
                    true,
                )
            })?;
            if entries.next().is_some() {
                return Err(privacy_mutation_failure(
                    CoreError::operation_failed("privacy directory verification failed"),
                    confirmed_affected_item_count,
                    true,
                ));
            }
        } else {
            match fs::symlink_metadata(root) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(privacy_mutation_failure(
                        privacy_io_error("privacy file verification failed", error),
                        confirmed_affected_item_count,
                        true,
                    ))
                }
                Ok(_) => {
                    return Err(privacy_mutation_failure(
                        CoreError::operation_failed("privacy file verification failed"),
                        confirmed_affected_item_count,
                        true,
                    ))
                }
            }
        }
    }
    Ok(count)
}

fn clear_recent_items(root: &Path) -> Result<u64, PrivacyMutationFailure> {
    let metadata = fs::symlink_metadata(root).map_err(|error| {
        PrivacyMutationFailure::from(privacy_io_error("recent items metadata unavailable", error))
    })?;
    if !metadata.is_dir() || current_platform().is_link_like(&metadata) {
        return Err(
            CoreError::operation_failed("recent items directory identity is unsafe").into(),
        );
    }
    let mut removed = 0;
    for entry in fs::read_dir(root).map_err(|error| {
        PrivacyMutationFailure::from(CoreError::operation_failed(format!(
            "recent items are unreadable: {error}"
        )))
    })? {
        let path = entry
            .map_err(|error| {
                privacy_mutation_failure(
                    CoreError::operation_failed(format!("recent item is unreadable: {error}")),
                    removed,
                    removed > 0,
                )
            })?
            .path();
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            privacy_mutation_failure(
                CoreError::operation_failed(format!("recent item metadata unavailable: {error}")),
                removed,
                removed > 0,
            )
        })?;
        if !metadata.is_file() || current_platform().is_link_like(&metadata) {
            return Err(privacy_mutation_failure(
                CoreError::operation_failed("recent item identity is unsafe"),
                removed,
                removed > 0,
            ));
        }
        fs::remove_file(&path).map_err(|error| {
            privacy_mutation_failure(
                CoreError::operation_failed(format!("recent item removal failed: {error}")),
                removed,
                true,
            )
        })?;
        removed += 1;
    }
    let remaining = fs::read_dir(root)
        .map_err(|error| {
            privacy_mutation_failure(
                CoreError::operation_failed(format!("recent items are unreadable: {error}")),
                removed,
                true,
            )
        })?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        })
        .count();
    if remaining != 0 {
        return Err(privacy_mutation_failure(
            CoreError::operation_failed("recent items verification failed"),
            removed,
            true,
        ));
    }
    Ok(removed)
}

fn candidate_token(
    scan_nonce: &str,
    source_id: &str,
    profile_name: Option<&str>,
    kind: PrivacyDataKind,
) -> String {
    blake3::hash(
        format!(
            "{scan_nonce}:{source_id}:{}:{kind:?}",
            profile_name.unwrap_or_default()
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string()
}
fn revision_for(candidates: &BTreeMap<String, NativePrivacyCandidate>) -> String {
    let mut hasher = blake3::Hasher::new();
    for candidate in candidates.values() {
        hasher.update(candidate.token.as_bytes());
        hasher.update(candidate.fingerprint.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}
fn plan_id(
    scan: &PrivacyScanResult,
    candidates: &[NativePrivacyCandidate],
    expires_at_ms: u64,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(scan.scan_id.as_bytes());
    hasher.update(scan.revision.as_bytes());
    hasher.update(&expires_at_ms.to_le_bytes());
    for candidate in candidates {
        hasher.update(candidate.token.as_bytes());
        hasher.update(candidate.fingerprint.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}
fn process_leaf(value: &str) -> String {
    Path::new(value)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| value.into())
}

fn reconcile_scan_after_execution(
    executed_scan_id: &str,
    results: &[PrivacyExecutionItemResult],
    operation_id: u64,
) -> Option<PrivacyScanResult> {
    let mut session = match current_scan_session() {
        Ok(session) if session.public_result.scan_id == executed_scan_id => session,
        Ok(_) => {
            log::warn!(
                "privacy_scan_reconcile_skipped operation_id={operation_id} reason=scan_id_mismatch"
            );
            return None;
        }
        Err(error) => {
            let error_detail = mangodisk_platform::diagnostics::text(&error);
            log::warn!(
                "privacy_scan_reconcile_skipped operation_id={operation_id} reason=session_unavailable error_detail={error_detail}"
            );
            return None;
        }
    };
    let completed_tokens = results
        .iter()
        .filter(|result| {
            result.verified
                && matches!(
                    result.status,
                    PrivacyExecutionItemStatus::Cleared | PrivacyExecutionItemStatus::Unchanged
                )
        })
        .map(|result| result.token.as_str())
        .collect::<BTreeSet<_>>();

    // Completed tokens remain as empty presentation rows so users can see that the category was
    // scanned. They are removed only from the private candidate map, which makes them impossible
    // to select for another destructive plan without a later real scan finding new data.
    session
        .candidates
        .retain(|token, _| !completed_tokens.contains(token.as_str()));
    for item in &mut session.public_result.items {
        if completed_tokens.contains(item.token.as_str()) {
            item.capability = PrivacyCapabilityState::Empty;
            item.item_count = 0;
            item.estimated_bytes = 0;
            item.selected_by_default = false;
            item.requires_browser_close = false;
        }
    }
    for coverage in &mut session.public_result.coverage {
        let source_items = session
            .public_result
            .items
            .iter()
            .filter(|item| item.source_id == coverage.source_id)
            .collect::<Vec<_>>();
        coverage.item_count = source_items.iter().map(|item| item.item_count).sum();
        coverage.capability = source_coverage_capability(
            &source_items
                .iter()
                .map(|item| item.capability)
                .collect::<Vec<_>>(),
            coverage.item_count,
        );
    }
    session.public_result.revision = revision_for(&session.candidates);
    session.public_result.scan_id = blake3::hash(
        format!(
            "privacy-reconciled-scan:{}:{}:{}",
            executed_scan_id,
            session.public_result.revision,
            now_ms()
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    let public_result = session.public_result.clone();
    if let Err(error) = replace_scan_session(session) {
        let error_detail = mangodisk_platform::diagnostics::text(&error);
        log::warn!(
            "privacy_scan_reconcile_publish_failed operation_id={operation_id} error_detail={error_detail}"
        );
        return None;
    }
    Some(public_result)
}

fn execution_result(
    candidate: &NativePrivacyCandidate,
    status: PrivacyExecutionItemStatus,
    affected_item_count: u64,
    verified: bool,
    failure_reason: Option<&str>,
) -> PrivacyExecutionItemResult {
    PrivacyExecutionItemResult {
        token: candidate.token.clone(),
        status,
        affected_item_count,
        verified,
        failure_reason: failure_reason.map(Into::into),
    }
}

fn execution_progress_items(
    results: &[PrivacyExecutionItemResult],
) -> Vec<PrivacyExecutionProgressItem> {
    results
        .iter()
        .map(|result| PrivacyExecutionProgressItem {
            token: result.token.clone(),
            status: result.status,
        })
        .collect()
}

/// Keep the source, mutation state and native cause together for post-failure diagnosis.
fn log_privacy_item_failure(
    operation_id: u64,
    candidate: &NativePrivacyCandidate,
    stage: &'static str,
    reason: &'static str,
    error: Option<&CoreError>,
    confirmed_affected_item_count: u64,
) {
    let error_detail = error
        .map(|error| mangodisk_platform::diagnostics::text(&error))
        .unwrap_or_else(|| "none".into());
    let mutation_state = error
        .map(|error| match error.mutation_state() {
            PlatformMutationState::NotAttempted => "not_attempted",
            PlatformMutationState::MayHaveChanged => "may_have_changed",
        })
        .unwrap_or("not_attempted");
    log::warn!(
        "privacy_item_failed operation_id={operation_id} source_id={} kind={:?} stage={stage} reason={reason} mutation_state={mutation_state} confirmed_affected_count={confirmed_affected_item_count} error_detail={error_detail}",
        candidate.item.source_id,
        candidate.item.kind
    );
}

fn append_history_record(
    plan_id: String,
    time_range: PrivacyTimeRange,
    candidates: &[NativePrivacyCandidate],
    results: &[PrivacyExecutionItemResult],
    started_at_ms: u64,
    finished_at_ms: u64,
) {
    let items = candidates
        .iter()
        .zip(results)
        .map(|(candidate, result)| PrivacyCleanupHistoryItem {
            source_id: candidate.item.source_id.clone(),
            kind: candidate.item.kind,
            affected_item_count: result.affected_item_count,
            status: match result.status {
                PrivacyExecutionItemStatus::Cleared => PrivacyCleanupHistoryItemStatus::Cleared,
                PrivacyExecutionItemStatus::Unchanged => PrivacyCleanupHistoryItemStatus::Unchanged,
                PrivacyExecutionItemStatus::Failed => PrivacyCleanupHistoryItemStatus::Failed,
                PrivacyExecutionItemStatus::Cancelled => PrivacyCleanupHistoryItemStatus::Cancelled,
            },
            failure_reason: result.failure_reason.clone(),
        })
        .collect::<Vec<_>>();
    let failed_item_count = items
        .iter()
        .filter(|item| item.status == PrivacyCleanupHistoryItemStatus::Failed)
        .count() as u64;
    let record = OperationRecord {
        schema_version: OPERATION_RECORD_SCHEMA_VERSION,
        operation_id: format!("privacy-{plan_id}"),
        category: OperationCategory::PrivacyCleanup,
        started_at_ms,
        finished_at_ms,
        outcome: privacy_history_outcome(&items),
        dry_run: false,
        selected_item_count: items.len() as u64,
        affected_item_count: items.iter().map(|item| item.affected_item_count).sum(),
        expected_bytes: 0,
        released_bytes: None,
        released_bytes_is_estimate: false,
        failed_item_count,
        details: OperationDetails::PrivacyCleanup(PrivacyCleanupOperationDetails {
            plan_id,
            time_range,
            items,
        }),
    };
    if let Err(error) = HistoryService::append(record) {
        log::warn!(
            "privacy_history_save_failed error={}",
            mangodisk_platform::diagnostics::text(&error)
        );
    }
}

fn privacy_history_outcome(items: &[PrivacyCleanupHistoryItem]) -> OperationOutcome {
    if items
        .iter()
        .all(|item| item.status == PrivacyCleanupHistoryItemStatus::Cancelled)
    {
        OperationOutcome::Cancelled
    } else if items.iter().any(|item| {
        matches!(
            item.status,
            PrivacyCleanupHistoryItemStatus::Failed | PrivacyCleanupHistoryItemStatus::Cancelled
        )
    }) {
        OperationOutcome::CompletedWithWarnings
    } else {
        OperationOutcome::Completed
    }
}
fn platform_cancellation(operation: &OperationGuard) -> PlatformCancellation {
    let flag = operation.cancellation_flag();
    PlatformCancellation::new(move || flag.load(std::sync::atomic::Ordering::Relaxed))
}
fn scan_session_lock() -> &'static Mutex<Option<PrivacyScanSession>> {
    SCAN_SESSION.get_or_init(|| Mutex::new(None))
}
fn plan_lock() -> &'static Mutex<Option<PendingPrivacyPlan>> {
    EXECUTION_PLAN.get_or_init(|| Mutex::new(None))
}
fn replace_scan_session(session: PrivacyScanSession) -> CoreResult<()> {
    *scan_session_lock()
        .lock()
        .map_err(|_| CoreError::operation_failed("privacy scan session is unavailable"))? =
        Some(session);
    Ok(())
}
fn current_scan_session() -> CoreResult<PrivacyScanSession> {
    scan_session_lock()
        .lock()
        .map_err(|_| CoreError::operation_failed("privacy scan session is unavailable"))?
        .clone()
        .ok_or_else(|| CoreError::invalid_input("privacy scan session has expired"))
}
fn clear_scan_session() -> CoreResult<()> {
    *scan_session_lock()
        .lock()
        .map_err(|_| CoreError::operation_failed("privacy scan session is unavailable"))? = None;
    Ok(())
}
fn replace_pending_plan(plan: PendingPrivacyPlan) -> CoreResult<()> {
    *plan_lock()
        .lock()
        .map_err(|_| CoreError::operation_failed("privacy execution plan is unavailable"))? =
        Some(plan);
    Ok(())
}
fn clear_pending_plan() -> CoreResult<()> {
    *plan_lock()
        .lock()
        .map_err(|_| CoreError::operation_failed("privacy execution plan is unavailable"))? = None;
    Ok(())
}
fn current_pending_plan(plan_id: &str) -> CoreResult<PendingPrivacyPlan> {
    let pending = plan_lock()
        .lock()
        .map_err(|_| CoreError::operation_failed("privacy execution plan is unavailable"))?
        .clone()
        .ok_or_else(|| CoreError::invalid_input("privacy execution plan has expired"))?;
    if pending.public_plan.plan_id != plan_id {
        return Err(CoreError::invalid_input(
            "privacy execution plan does not match",
        ));
    }
    Ok(pending)
}
fn take_pending_plan(plan_id: &str) -> CoreResult<PendingPrivacyPlan> {
    let mut guard = plan_lock()
        .lock()
        .map_err(|_| CoreError::operation_failed("privacy execution plan is unavailable"))?;
    let pending = guard
        .take()
        .ok_or_else(|| CoreError::invalid_input("privacy execution plan has expired"))?;
    if pending.public_plan.plan_id != plan_id {
        return Err(CoreError::invalid_input(
            "privacy execution plan does not match",
        ));
    }
    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::operation::test_operation_lock;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    fn execution_request(plan_id: String) -> PrivacyExecutionRunRequest {
        PrivacyExecutionRunRequest {
            plan_id,
            excluded_source_ids: Vec::new(),
        }
    }

    fn fixture_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "mangodisk-privacy-service-{name}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn install_directory_scan_session(root: &Path) -> (String, String) {
        let token = "a".repeat(64);
        let item = privacy_item(PrivacyItemInput {
            token: token.clone(),
            source_id: "synthetic".into(),
            source_name: "Synthetic browser".into(),
            profile_id: Some("synthetic:profile".into()),
            profile_name: Some("Test profile".into()),
            kind: PrivacyDataKind::SiteStorage,
            capability: PrivacyCapabilityState::Ready,
            item_count: 1,
            estimated_bytes: 1,
            requires_browser_close: false,
        });
        let roots = vec![root.to_path_buf()];
        let fingerprint = roots_summary_fingerprint(&roots, &PlatformCancellation::new(|| false))
            .unwrap()
            .0;
        let candidate = NativePrivacyCandidate {
            token: token.clone(),
            item: item.clone(),
            fingerprint: fingerprint.clone(),
            action: NativePrivacyAction::Directories { roots },
            browser_process_names: Vec::new(),
        };
        let scan_id = "synthetic-scan".to_string();
        replace_scan_session(PrivacyScanSession {
            public_result: PrivacyScanResult {
                schema_version: PRIVACY_SCAN_SCHEMA_VERSION,
                scan_id: scan_id.clone(),
                revision: fingerprint,
                time_range: PrivacyTimeRange::AllTime,
                scanned_at_ms: 1,
                elapsed_ms: 1,
                items: vec![item],
                coverage: Vec::new(),
            },
            candidates: BTreeMap::from([(token.clone(), candidate)]),
        })
        .unwrap();
        clear_pending_plan().unwrap();
        (scan_id, token)
    }

    #[test]
    fn application_permission_denial_remains_distinct_from_unsupported_capability() {
        assert_eq!(
            application_availability_capability(
                PlatformPrivacyApplicationTraceAvailability::PermissionRequired
            ),
            Some(PrivacyCapabilityState::PermissionRequired)
        );
        assert_eq!(
            application_availability_capability(
                PlatformPrivacyApplicationTraceAvailability::Unavailable
            ),
            Some(PrivacyCapabilityState::Unavailable)
        );
        assert_eq!(
            application_availability_capability(
                PlatformPrivacyApplicationTraceAvailability::Available
            ),
            None
        );
    }

    #[test]
    fn candidate_tokens_never_include_source_labels() {
        let token = candidate_token(
            "nonce",
            "browser",
            Some("Profile 1"),
            PrivacyDataKind::BrowsingHistory,
        );
        assert_eq!(token.len(), 64);
        assert!(!token.contains("Profile"));
    }

    #[test]
    fn detail_pages_are_read_only_and_bound_to_the_current_scan() {
        let _test_guard = test_operation_lock();
        let directory = fixture_directory("details");
        fs::write(directory.join("private-entry.db"), b"private").unwrap();
        let (scan_id, token) = install_directory_scan_session(&directory);

        let page = PrivacyService::details(PrivacyDetailsRequest {
            scan_id: scan_id.clone(),
            token: token.clone(),
            offset: 0,
            limit: 20,
        })
        .unwrap();
        assert_eq!(page.presentation, PrivacyDetailsPresentation::List);
        assert_eq!(page.entries.len(), 1);
        assert_eq!(
            page.entries[0].label,
            directory.join("private-entry.db").to_string_lossy()
        );
        assert_eq!(page.entries[0].item_count, 1);
        assert!(directory.join("private-entry.db").is_file());

        fs::write(directory.join("new-private-entry.db"), b"new private data").unwrap();
        let changed = PrivacyService::details(PrivacyDetailsRequest {
            scan_id: scan_id.clone(),
            token: token.clone(),
            offset: 0,
            limit: 20,
        })
        .unwrap();
        assert_eq!(changed.entries.len(), 2);
        assert!(changed
            .entries
            .iter()
            .any(|entry| entry.label.ends_with("new-private-entry.db")));
        assert!(directory.join("new-private-entry.db").is_file());

        let error = PrivacyService::details(PrivacyDetailsRequest {
            scan_id: "expired-scan".into(),
            token,
            offset: 0,
            limit: 20,
        })
        .unwrap_err();
        assert_eq!(error.code(), CoreErrorCode::InvalidInput);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn combined_system_fingerprint_covers_the_native_companion() {
        let files_only = system_trace_fingerprint("files", None);
        let first_native = system_trace_fingerprint("files", Some("native-a"));
        let second_native = system_trace_fingerprint("files", Some("native-b"));

        assert_ne!(files_only, first_native);
        assert_ne!(first_native, second_native);
    }

    #[test]
    fn directory_summary_detects_a_same_size_file_rewrite() {
        let directory = fixture_directory("same-size-rewrite");
        let path = directory.join("history");
        fs::write(&path, b"first").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(UNIX_EPOCH + std::time::Duration::from_secs(10)),
            )
            .unwrap();
        let before = roots_summary_fingerprint(
            std::slice::from_ref(&directory),
            &PlatformCancellation::new(|| false),
        )
        .unwrap()
        .0;

        fs::write(&path, b"other").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(UNIX_EPOCH + std::time::Duration::from_secs(20)),
            )
            .unwrap();
        let after = roots_summary_fingerprint(
            std::slice::from_ref(&directory),
            &PlatformCancellation::new(|| false),
        )
        .unwrap()
        .0;

        assert_ne!(before, after);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn shell_history_details_decode_zsh_metadata_and_non_utf8_records() {
        let directory = fixture_directory("shell-details");
        let history = directory.join(".zsh_history");
        fs::write(
            &history,
            b": 1788330000:4;git status\n: 1788330001:0;printf '\xff'\n",
        )
        .unwrap();

        let first_page = text_record_detail_entries(std::slice::from_ref(&history), 0, 1).unwrap();
        let second_page = text_record_detail_entries(&[history], 1, 1).unwrap();

        assert_eq!(first_page.len(), 1);
        assert_eq!(first_page[0].label, "zsh · git status");
        assert_eq!(second_page.len(), 1);
        assert!(second_page[0].label.starts_with("zsh · printf"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn recent_document_details_include_only_shortcuts() {
        let directory = fixture_directory("recent-document-details");
        let shortcut = directory.join("document.lnk");
        fs::write(&shortcut, b"shortcut").unwrap();
        fs::write(directory.join("desktop.ini"), b"metadata").unwrap();

        let entries = system_root_detail_entries(
            PlatformPrivacySystemTraceKind::RecentDocumentHistory,
            std::slice::from_ref(&directory),
            0,
            20,
        )
        .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, shortcut.to_string_lossy());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn application_privacy_policy_separates_regenerable_and_user_content_traces() {
        let application_cache = privacy_item(PrivacyItemInput {
            token: "cache".into(),
            source_id: "vscode".into(),
            source_name: "Visual Studio Code".into(),
            profile_id: None,
            profile_name: None,
            kind: PrivacyDataKind::ApplicationCache,
            capability: PrivacyCapabilityState::Ready,
            item_count: 3,
            estimated_bytes: 64,
            requires_browser_close: true,
        });
        let editor_history = privacy_item(PrivacyItemInput {
            token: "history".into(),
            source_id: "vscode".into(),
            source_name: "Visual Studio Code".into(),
            profile_id: None,
            profile_name: None,
            kind: PrivacyDataKind::EditorLocalHistory,
            capability: PrivacyCapabilityState::Ready,
            item_count: 2,
            estimated_bytes: 32,
            requires_browser_close: true,
        });

        assert_eq!(
            application_cache.category,
            PrivacyCategory::ApplicationActivity
        );
        assert_eq!(application_cache.impact, PrivacyImpact::Low);
        assert_eq!(
            application_cache.recommendation,
            PrivacyRecommendation::Recommended
        );
        assert!(application_cache.selected_by_default);
        assert_eq!(
            editor_history.category,
            PrivacyCategory::ApplicationActivity
        );
        assert_eq!(editor_history.impact, PrivacyImpact::DataLoss);
        assert_eq!(editor_history.recommendation, PrivacyRecommendation::Manual);
        assert!(!editor_history.selected_by_default);
    }

    #[test]
    fn high_impact_items_are_never_selected_by_default() {
        let item = privacy_item(PrivacyItemInput {
            token: "token".into(),
            source_id: "browser".into(),
            source_name: "Browser".into(),
            profile_id: None,
            profile_name: None,
            kind: PrivacyDataKind::Cookies,
            capability: PrivacyCapabilityState::Ready,
            item_count: 4,
            estimated_bytes: 10,
            requires_browser_close: false,
        });
        assert!(!item.selected_by_default);
        assert_eq!(item.impact, PrivacyImpact::SignOut);
    }

    #[test]
    fn low_impact_browser_activity_is_recommended_by_default() {
        for (kind, recommendation, selected_by_default) in [
            (
                PrivacyDataKind::BrowsingHistory,
                PrivacyRecommendation::Recommended,
                true,
            ),
            (
                PrivacyDataKind::DownloadHistory,
                PrivacyRecommendation::Recommended,
                true,
            ),
            (
                PrivacyDataKind::Sessions,
                PrivacyRecommendation::Recommended,
                true,
            ),
            (
                PrivacyDataKind::BrowserCache,
                PrivacyRecommendation::Recommended,
                true,
            ),
            (
                PrivacyDataKind::AddressBarShortcuts,
                PrivacyRecommendation::Recommended,
                true,
            ),
            (
                PrivacyDataKind::SearchHistory,
                PrivacyRecommendation::Recommended,
                true,
            ),
            (
                PrivacyDataKind::WebsiteIcons,
                PrivacyRecommendation::Recommended,
                true,
            ),
        ] {
            let item = privacy_item(PrivacyItemInput {
                token: "token".into(),
                source_id: "browser".into(),
                source_name: "Browser".into(),
                profile_id: Some("browser:Default".into()),
                profile_name: Some("Default".into()),
                kind,
                capability: PrivacyCapabilityState::Ready,
                item_count: 4,
                estimated_bytes: 10,
                requires_browser_close: true,
            });
            assert_eq!(item.impact, PrivacyImpact::Low);
            assert_eq!(item.recommendation, recommendation);
            assert_eq!(item.selected_by_default, selected_by_default);
        }
    }

    #[test]
    fn known_low_impact_traces_remain_recommended_while_the_owner_is_running() {
        let item = privacy_item(PrivacyItemInput {
            token: "token".into(),
            source_id: "browser".into(),
            source_name: "Browser".into(),
            profile_id: Some("browser:Default".into()),
            profile_name: Some("Default".into()),
            kind: PrivacyDataKind::BrowsingHistory,
            capability: PrivacyCapabilityState::BrowserRunning,
            item_count: 4,
            estimated_bytes: 10,
            requires_browser_close: true,
        });

        assert_eq!(item.recommendation, PrivacyRecommendation::Recommended);
        assert!(item.selected_by_default);
    }

    #[test]
    fn system_trace_policy_distinguishes_regenerable_and_workflow_data() {
        for (kind, impact, recommendation, selected_by_default) in [
            (
                PrivacyDataKind::JumpLists,
                PrivacyImpact::Low,
                PrivacyRecommendation::Recommended,
                true,
            ),
            (
                PrivacyDataKind::ShellHistory,
                PrivacyImpact::Workflow,
                PrivacyRecommendation::Manual,
                false,
            ),
            (
                PrivacyDataKind::ApplicationUsageHistory,
                PrivacyImpact::Low,
                PrivacyRecommendation::Recommended,
                true,
            ),
            (
                PrivacyDataKind::NetworkConnectionHistory,
                PrivacyImpact::Low,
                PrivacyRecommendation::Recommended,
                true,
            ),
            (
                PrivacyDataKind::FolderViewHistory,
                PrivacyImpact::Workflow,
                PrivacyRecommendation::Manual,
                false,
            ),
        ] {
            let item = privacy_item(PrivacyItemInput {
                token: "token".into(),
                source_id: "system".into(),
                source_name: "System".into(),
                profile_id: None,
                profile_name: None,
                kind,
                capability: PrivacyCapabilityState::Ready,
                item_count: 4,
                estimated_bytes: 10,
                requires_browser_close: false,
            });
            assert_eq!(item.category, PrivacyCategory::SystemActivity);
            assert_eq!(item.impact, impact);
            assert_eq!(item.recommendation, recommendation);
            assert_eq!(item.selected_by_default, selected_by_default);
        }
    }

    #[test]
    fn recent_application_searches_are_low_risk_and_selected_by_default() {
        let item = privacy_item(PrivacyItemInput {
            token: "searches".into(),
            source_id: "word".into(),
            source_name: "Microsoft Word".into(),
            profile_id: None,
            profile_name: None,
            kind: PrivacyDataKind::RecentSearches,
            capability: PrivacyCapabilityState::Ready,
            item_count: 3,
            estimated_bytes: 0,
            requires_browser_close: true,
        });

        assert_eq!(item.category, PrivacyCategory::ApplicationActivity);
        assert_eq!(item.impact, PrivacyImpact::Low);
        assert_eq!(item.recommendation, PrivacyRecommendation::Recommended);
        assert!(item.selected_by_default);
    }

    #[test]
    fn personal_browser_data_is_user_selectable_but_never_selected_by_default() {
        for kind in [
            PrivacyDataKind::SavedPasswords,
            PrivacyDataKind::AutofillData,
        ] {
            let item = privacy_item(PrivacyItemInput {
                token: "token".into(),
                source_id: "browser".into(),
                source_name: "Browser".into(),
                profile_id: Some("browser:Default".into()),
                profile_name: Some("Default".into()),
                kind,
                capability: PrivacyCapabilityState::Ready,
                item_count: 4,
                estimated_bytes: 10,
                requires_browser_close: true,
            });
            assert_eq!(item.recommendation, PrivacyRecommendation::Manual);
            assert_eq!(item.impact, PrivacyImpact::DataLoss);
            assert!(!item.selected_by_default);
        }
    }

    #[test]
    fn browser_close_requirements_group_running_sources_without_paths() {
        let candidate =
            |token: &str, source_id: &str, source_name: &str, capability| NativePrivacyCandidate {
                token: token.into(),
                item: privacy_item(PrivacyItemInput {
                    token: token.into(),
                    source_id: source_id.into(),
                    source_name: source_name.into(),
                    profile_id: Some(format!("{source_id}:profile")),
                    profile_name: Some("Profile".into()),
                    kind: PrivacyDataKind::Sessions,
                    capability,
                    item_count: 1,
                    estimated_bytes: 1,
                    requires_browser_close: true,
                }),
                fingerprint: "fingerprint".into(),
                action: NativePrivacyAction::Directories { roots: Vec::new() },
                browser_process_names: vec!["Browser.exe".into(), "browser.exe".into()],
            };
        let requirements = browser_close_requirements(&[
            candidate(
                "one",
                "browser",
                "Browser",
                PrivacyCapabilityState::BrowserRunning,
            ),
            candidate(
                "two",
                "browser",
                "Browser",
                PrivacyCapabilityState::BrowserRunning,
            ),
            candidate(
                "ready",
                "ready-browser",
                "Ready browser",
                PrivacyCapabilityState::Ready,
            ),
        ]);

        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].source_id, "browser");
        assert_eq!(requirements[0].source_name, "Browser");
        assert_eq!(requirements[0].processes, vec!["Browser.exe"]);
    }

    #[test]
    fn browser_process_requests_remain_bounded_to_the_pending_plan() {
        let _test_guard = test_operation_lock();
        let plan_id = "p".repeat(64);
        let candidate = NativePrivacyCandidate {
            token: "token".into(),
            item: privacy_item(PrivacyItemInput {
                token: "token".into(),
                source_id: "browser".into(),
                source_name: "Browser".into(),
                profile_id: Some("browser:profile".into()),
                profile_name: Some("Profile".into()),
                kind: PrivacyDataKind::Sessions,
                capability: PrivacyCapabilityState::BrowserRunning,
                item_count: 1,
                estimated_bytes: 1,
                requires_browser_close: true,
            }),
            fingerprint: "fingerprint".into(),
            action: NativePrivacyAction::Directories { roots: Vec::new() },
            browser_process_names: vec!["mangodisk-status-fixture-never-running.exe".into()],
        };
        replace_pending_plan(PendingPrivacyPlan {
            public_plan: PrivacyExecutionPlan {
                schema_version: PRIVACY_PLAN_SCHEMA_VERSION,
                plan_id: plan_id.clone(),
                scan_id: "scan".into(),
                created_at_ms: now_ms(),
                expires_at_ms: now_ms().saturating_add(60_000),
                items: Vec::new(),
                browser_close_requirements: browser_close_requirements(std::slice::from_ref(
                    &candidate,
                )),
                requires_confirmation: true,
                requires_browser_close: true,
            },
            candidates: vec![candidate],
            time_range: PrivacyTimeRange::AllTime,
        })
        .unwrap();

        let error = PrivacyService::close_browsers(PrivacyBrowserCloseRequest {
            plan_id,
            source_ids: vec!["untrusted".into()],
            mode: crate::ApplicationCloseMode::Graceful,
        })
        .unwrap_err();
        assert_eq!(error.code(), CoreErrorCode::InvalidInput);

        let status = PrivacyService::refresh_browser_status(PrivacyBrowserStatusRequest {
            plan_id: "p".repeat(64),
            source_ids: vec!["browser".into()],
        })
        .unwrap();
        assert_eq!(status.running_process_count, 0);
        assert_eq!(status.targets.len(), 1);
        assert!(status.targets[0].running_processes.is_empty());

        let error = PrivacyService::refresh_browser_status(PrivacyBrowserStatusRequest {
            plan_id: "p".repeat(64),
            source_ids: vec!["untrusted".into()],
        })
        .unwrap_err();
        assert_eq!(error.code(), CoreErrorCode::InvalidInput);
        clear_pending_plan().unwrap();
    }

    #[test]
    fn running_browser_drift_can_prepare_a_close_gated_plan() {
        let _test_guard = test_operation_lock();
        let directory = fixture_directory("running-browser-plan");
        let changing_file = directory.join("session");
        fs::write(&changing_file, b"before").unwrap();
        let (scan_id, token) = install_directory_scan_session(&directory);
        let mut session = current_scan_session().unwrap();
        let candidate = session.candidates.get_mut(&token).unwrap();
        candidate.item.capability = PrivacyCapabilityState::BrowserRunning;
        candidate.browser_process_names = vec!["synthetic-browser".into()];
        session.public_result.items[0].capability = PrivacyCapabilityState::BrowserRunning;
        replace_scan_session(session).unwrap();
        fs::write(&changing_file, b"changed-while-running").unwrap();

        let plan = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id,
            tokens: vec![token],
        })
        .unwrap();
        assert!(plan.requires_browser_close);
        assert_eq!(plan.browser_close_requirements.len(), 1);

        clear_scan_session().unwrap();
        clear_pending_plan().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn review_only_credential_evidence_cannot_enter_an_execution_plan() {
        let _test_guard = test_operation_lock();
        let directory = fixture_directory("review-only-plan");
        fs::write(directory.join("evidence"), b"aggregate-only").unwrap();
        let (scan_id, token) = install_directory_scan_session(&directory);
        let mut session = current_scan_session().unwrap();
        let candidate = session.candidates.get_mut(&token).unwrap();
        candidate.item.recommendation = PrivacyRecommendation::ReviewOnly;
        session.public_result.items[0].recommendation = PrivacyRecommendation::ReviewOnly;
        replace_scan_session(session).unwrap();

        let error = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id,
            tokens: vec![token],
        })
        .unwrap_err();
        assert_eq!(error.code(), CoreErrorCode::InvalidInput);

        clear_scan_session().unwrap();
        clear_pending_plan().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn partial_cancellation_is_recorded_as_completed_with_warnings() {
        let history_item = |status| PrivacyCleanupHistoryItem {
            source_id: "synthetic".into(),
            kind: PrivacyDataKind::RecentItems,
            affected_item_count: 0,
            status,
            failure_reason: None,
        };
        let partial = vec![
            history_item(PrivacyCleanupHistoryItemStatus::Cleared),
            history_item(PrivacyCleanupHistoryItemStatus::Cancelled),
        ];
        assert_eq!(
            privacy_history_outcome(&partial),
            OperationOutcome::CompletedWithWarnings
        );
        assert_eq!(
            privacy_history_outcome(&[history_item(PrivacyCleanupHistoryItemStatus::Cancelled)]),
            OperationOutcome::Cancelled
        );
    }

    #[test]
    fn database_candidate_fingerprint_is_captured_after_wal_read() {
        let directory = fixture_directory("database-wal-fingerprint");
        let path = directory.join("places.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE moz_places(
                     id INTEGER PRIMARY KEY,
                     url TEXT,
                     foreign_count INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE moz_historyvisits(
                     id INTEGER PRIMARY KEY,
                     place_id INTEGER,
                     visit_type INTEGER,
                     visit_date INTEGER
                 );
                 INSERT INTO moz_places VALUES (1, 'https://fixture.invalid', 0);
                 INSERT INTO moz_historyvisits VALUES (1, 1, 1, 1);",
            )
            .unwrap();
        let mut candidates = BTreeMap::new();
        let mut items = Vec::new();

        add_database_candidate(
            "nonce",
            "firefox",
            "Firefox",
            "firefox:test-profile",
            "Test profile",
            &path,
            PlatformPrivacyBrowserKind::Firefox,
            PrivacyDataKind::BrowsingHistory,
            PrivacyTimeRange::AllTime,
            1,
            false,
            &[],
            &mut candidates,
            &mut items,
        );

        connection
            .execute(
                "INSERT INTO moz_places VALUES (2, 'https://later-query.invalid', 0)",
                [],
            )
            .unwrap();
        assert_ne!(
            candidates.values().next().unwrap().fingerprint,
            file_fingerprint(&path).unwrap()
        );
        refresh_database_fingerprints(&mut candidates).unwrap();
        assert!(candidates
            .values()
            .all(|candidate| candidate.fingerprint == file_fingerprint(&path).unwrap()));
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn running_browser_database_is_counted_but_remains_close_gated() {
        let directory = fixture_directory("running-database-scan");
        let path = directory.join("History");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT);
                 CREATE TABLE visits(id INTEGER PRIMARY KEY, url INTEGER, visit_time INTEGER);
                 INSERT INTO urls VALUES (1, 'https://fixture.invalid');
                 INSERT INTO visits VALUES (1, 1, 1);",
            )
            .unwrap();
        let mut candidates = BTreeMap::new();
        let mut items = Vec::new();

        add_database_candidate(
            "nonce",
            "chrome",
            "Google Chrome",
            "chrome:work",
            "Work",
            &path,
            PlatformPrivacyBrowserKind::Chromium,
            PrivacyDataKind::BrowsingHistory,
            PrivacyTimeRange::AllTime,
            1,
            true,
            &["Google Chrome".into()],
            &mut candidates,
            &mut items,
        );

        assert_eq!(items[0].item_count, 1);
        assert_eq!(items[0].capability, PrivacyCapabilityState::BrowserRunning);
        assert!(items[0].requires_browser_close);
        assert_eq!(candidates.len(), 1);
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn locked_windows_cookie_database_remains_selectable_for_close_and_refresh() {
        use std::os::windows::fs::OpenOptionsExt;

        let directory = fixture_directory("locked-windows-cookie");
        let path = directory.join("Cookies");
        fs::write(&path, b"locked synthetic cookie database").unwrap();
        let locked = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let mut candidates = BTreeMap::new();
        let mut items = Vec::new();

        add_database_candidate(
            "nonce",
            "chrome",
            "Google Chrome",
            "chrome:Default",
            "Default",
            &path,
            PlatformPrivacyBrowserKind::Chromium,
            PrivacyDataKind::Cookies,
            PrivacyTimeRange::AllTime,
            1,
            true,
            &["chrome.exe".into()],
            &mut candidates,
            &mut items,
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].capability, PrivacyCapabilityState::BrowserRunning);
        assert_eq!(items[0].item_count, 0);
        assert!(privacy_item_is_actionable(&items[0]));
        assert_eq!(candidates.len(), 1);
        drop(locked);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn sequential_items_sharing_one_database_refresh_the_expected_fingerprint() {
        let directory = fixture_directory("shared-database-execution");
        let path = directory.join("History");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT);
                 CREATE TABLE visits(id INTEGER PRIMARY KEY, url INTEGER, visit_time INTEGER);
                 CREATE TABLE downloads(id INTEGER PRIMARY KEY, start_time INTEGER);
                 CREATE TABLE downloads_url_chains(id INTEGER, chain_index INTEGER, url TEXT);
                 INSERT INTO urls VALUES (1, 'https://fixture.invalid');
                 INSERT INTO visits VALUES (1, 1, 1);
                 INSERT INTO downloads VALUES (1, 1);
                 INSERT INTO downloads_url_chains VALUES (1, 0, 'https://download.invalid');",
            )
            .unwrap();
        drop(connection);
        let fingerprint = file_fingerprint(&path).unwrap();
        let candidate = |token: &str, kind| NativePrivacyCandidate {
            token: token.into(),
            item: privacy_item(PrivacyItemInput {
                token: token.into(),
                source_id: "chromium".into(),
                source_name: "Chromium".into(),
                profile_id: Some("chromium:test-profile".into()),
                profile_name: Some("Test profile".into()),
                kind,
                capability: PrivacyCapabilityState::Ready,
                item_count: 1,
                estimated_bytes: 1,
                requires_browser_close: true,
            }),
            fingerprint: fingerprint.clone(),
            action: NativePrivacyAction::Database {
                path: path.clone(),
                browser: PlatformPrivacyBrowserKind::Chromium,
                kind,
                range: PrivacyTimeRange::AllTime,
                scan_now_ms: 0,
            },
            browser_process_names: Vec::new(),
        };
        let history = candidate("history", PrivacyDataKind::BrowsingHistory);
        let downloads = candidate("downloads", PrivacyDataKind::DownloadHistory);
        let mut expectations =
            database_fingerprint_expectations(&[history.clone(), downloads.clone()]);

        assert_eq!(execute_candidate(&history).unwrap(), 1);
        assert!(!fingerprint_matches_expectation(&downloads, &expectations));
        refresh_database_fingerprint_expectation(&history, &mut expectations);
        assert!(fingerprint_matches_expectation(&downloads, &expectations));
        assert_eq!(execute_candidate(&downloads).unwrap(), 1);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn recent_items_cleanup_removes_only_shortcuts() {
        let directory = fixture_directory("recent-items");
        let target = directory.join("important-document.txt");
        let shortcut = directory.join("important-document.lnk");
        let unrelated = directory.join("desktop.ini");
        fs::write(&target, b"protected-target").unwrap();
        fs::write(&shortcut, b"synthetic-shortcut").unwrap();
        fs::write(&unrelated, b"protected-metadata").unwrap();

        assert_eq!(clear_recent_items(&directory).unwrap(), 1);
        assert_eq!(fs::read(&target).unwrap(), b"protected-target");
        assert_eq!(fs::read(&unrelated).unwrap(), b"protected-metadata");
        assert!(!shortcut.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn directory_cleanup_supports_a_single_session_file_root() {
        let directory = fixture_directory("session-file-root");
        let session_file = directory.join("LastSession.plist");
        fs::write(&session_file, b"synthetic-session").unwrap();

        assert_eq!(
            clear_directories(std::slice::from_ref(&session_file)).unwrap(),
            1
        );
        assert!(!session_file.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn directory_cleanup_supports_multiple_fixed_history_files() {
        let directory = fixture_directory("multiple-history-files");
        let first = directory.join("first-history");
        let second = directory.join("second-history");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();

        assert_eq!(
            clear_directories(&[first.clone(), second.clone()]).unwrap(),
            2
        );
        assert!(!first.exists());
        assert!(!second.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn directory_cleanup_preserves_confirmed_count_when_a_later_root_fails() {
        use std::os::unix::net::UnixListener;

        let directory = fixture_directory("partial-directory-cleanup");
        let first = directory.join("first-history");
        let unsupported = std::env::temp_dir().join(format!(
            "md-privacy-{}.socket",
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&first, b"first").unwrap();
        let listener = UnixListener::bind(&unsupported).unwrap();

        let failure = clear_directories(&[first.clone(), unsupported.clone()]).unwrap_err();

        assert_eq!(failure.confirmed_affected_item_count, 1);
        assert_eq!(
            failure.error.mutation_state(),
            PlatformMutationState::MayHaveChanged
        );
        assert!(!first.exists());
        assert!(unsupported.exists());
        drop(listener);
        fs::remove_file(unsupported).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn directory_cleanup_refuses_nested_symbolic_links() {
        use std::os::unix::fs::symlink;

        let directory = fixture_directory("directory-link");
        let protected = fixture_directory("protected-target");
        let protected_file = protected.join("keep.txt");
        fs::write(&protected_file, b"protected").unwrap();
        symlink(&protected, directory.join("nested-link")).unwrap();

        assert!(clear_directories(std::slice::from_ref(&directory)).is_err());
        assert_eq!(fs::read(&protected_file).unwrap(), b"protected");
        fs::remove_dir_all(directory).unwrap();
        fs::remove_dir_all(protected).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn directory_cleanup_refuses_a_symbolic_link_root() {
        use std::os::unix::fs::symlink;

        let directory = fixture_directory("root-link-holder");
        let protected = fixture_directory("root-link-target");
        let protected_file = protected.join("keep.txt");
        let root_link = directory.join("storage-link");
        fs::write(&protected_file, b"protected").unwrap();
        symlink(&protected, &root_link).unwrap();

        assert!(clear_directories(std::slice::from_ref(&root_link)).is_err());
        assert_eq!(fs::read(&protected_file).unwrap(), b"protected");
        fs::remove_dir_all(directory).unwrap();
        fs::remove_dir_all(protected).unwrap();
    }

    #[test]
    fn plan_rejects_forged_tokens_and_stale_sessions_while_execution_blocks_source_drift() {
        let _test_guard = test_operation_lock();
        let directory = fixture_directory("plan-validation");
        fs::write(directory.join("storage"), b"original").unwrap();
        let (scan_id, token) = install_directory_scan_session(&directory);

        let forged = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id: scan_id.clone(),
            tokens: vec!["f".repeat(64)],
        })
        .unwrap_err();
        assert_eq!(forged.code(), CoreErrorCode::InvalidInput);

        fs::write(directory.join("storage"), b"changed-after-scan").unwrap();
        let changed_plan = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id: scan_id.clone(),
            tokens: vec![token.clone()],
        })
        .unwrap();
        let changed = PrivacyService::execute(execution_request(changed_plan.plan_id)).unwrap();
        assert_eq!(changed.affected_item_count, 0);
        assert_eq!(changed.failed_item_count, 1);
        assert_eq!(
            changed.items[0].failure_reason.as_deref(),
            Some("sourceChanged")
        );
        assert_eq!(
            fs::read(directory.join("storage")).unwrap(),
            b"changed-after-scan"
        );

        clear_scan_session().unwrap();
        let stale = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id,
            tokens: vec![token],
        })
        .unwrap_err();
        assert_eq!(stale.code(), CoreErrorCode::InvalidInput);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn mismatched_plan_identifier_cannot_execute_a_prepared_action() {
        let _test_guard = test_operation_lock();
        let directory = fixture_directory("plan-identifier");
        let protected = directory.join("storage");
        fs::write(&protected, b"protected").unwrap();
        let (scan_id, token) = install_directory_scan_session(&directory);
        PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id,
            tokens: vec![token],
        })
        .unwrap();

        let error = PrivacyService::execute(execution_request("f".repeat(64))).unwrap_err();
        assert_eq!(error.code(), CoreErrorCode::InvalidInput);
        assert_eq!(fs::read(&protected).unwrap(), b"protected");
        clear_scan_session().unwrap();
        clear_pending_plan().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn verified_execution_reconciles_the_scan_without_a_second_scan() {
        let _test_guard = test_operation_lock();
        let directory = fixture_directory("execution-scan-reconcile");
        let remaining_directory = fixture_directory("execution-scan-remaining");
        fs::write(directory.join("storage"), b"privacy trace").unwrap();
        fs::write(remaining_directory.join("history"), b"remaining trace").unwrap();
        let (scan_id, token) = install_directory_scan_session(&directory);
        let remaining_token = "b".repeat(64);
        let remaining_roots = vec![remaining_directory.clone()];
        let remaining_item = privacy_item(PrivacyItemInput {
            token: remaining_token.clone(),
            source_id: "synthetic".into(),
            source_name: "Synthetic browser".into(),
            profile_id: Some("synthetic:profile".into()),
            profile_name: Some("Test profile".into()),
            kind: PrivacyDataKind::DownloadHistory,
            capability: PrivacyCapabilityState::Ready,
            item_count: 1,
            estimated_bytes: 1,
            requires_browser_close: false,
        });
        let mut original_session = current_scan_session().unwrap();
        original_session
            .public_result
            .items
            .push(remaining_item.clone());
        original_session.candidates.insert(
            remaining_token.clone(),
            NativePrivacyCandidate {
                token: remaining_token.clone(),
                item: remaining_item,
                fingerprint: roots_summary_fingerprint(
                    &remaining_roots,
                    &PlatformCancellation::new(|| false),
                )
                .unwrap()
                .0,
                action: NativePrivacyAction::Directories {
                    roots: remaining_roots,
                },
                browser_process_names: Vec::new(),
            },
        );
        original_session.public_result.revision = revision_for(&original_session.candidates);
        replace_scan_session(original_session).unwrap();
        let plan = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id,
            tokens: vec![token.clone()],
        })
        .unwrap();

        let mut progress_updates = Vec::new();
        let result =
            PrivacyService::execute_with_progress(execution_request(plan.plan_id), |progress| {
                progress_updates.push(progress);
            })
            .unwrap();
        let reconciled = result
            .scan
            .expect("verified execution must return an updated scan snapshot");

        assert_eq!(
            progress_updates.first().unwrap().stage,
            PrivacyExecutionStage::Validating
        );
        assert_eq!(progress_updates.first().unwrap().total_item_count, 1);
        assert!(progress_updates
            .iter()
            .any(|progress| progress.stage == PrivacyExecutionStage::Cleaning));
        let final_progress = progress_updates.last().unwrap();
        assert_eq!(final_progress.stage, PrivacyExecutionStage::Finalizing);
        assert_eq!(final_progress.completed_item_count, 1);
        assert_eq!(final_progress.affected_item_count, 1);
        assert_eq!(final_progress.completed_items.len(), 1);
        assert_eq!(final_progress.completed_items[0].token, token);
        assert_eq!(
            final_progress.completed_items[0].status,
            PrivacyExecutionItemStatus::Cleared
        );
        assert_eq!(result.items[0].status, PrivacyExecutionItemStatus::Cleared);
        assert!(result.items[0].verified);
        assert_eq!(reconciled.items[0].token, token);
        assert_eq!(reconciled.items[0].item_count, 0);
        assert_eq!(reconciled.items[0].estimated_bytes, 0);
        assert_eq!(
            reconciled.items[0].capability,
            PrivacyCapabilityState::Empty
        );
        assert!(!reconciled.items[0].selected_by_default);
        let published = current_scan_session().unwrap().public_result;
        assert_eq!(published.scan_id, reconciled.scan_id);
        assert_eq!(published.revision, reconciled.revision);
        assert_eq!(published.items[0].item_count, 0);
        let remaining_plan = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id: reconciled.scan_id,
            tokens: vec![remaining_token],
        })
        .expect("an untouched item must remain actionable without a second scan");
        assert_eq!(remaining_plan.items.len(), 1);

        clear_scan_session().unwrap();
        clear_pending_plan().unwrap();
        fs::remove_dir_all(directory).unwrap();
        fs::remove_dir_all(remaining_directory).unwrap();
    }

    #[test]
    fn closed_browser_candidates_refresh_without_a_catalog_rescan() {
        let _test_guard = test_operation_lock();
        let directory = fixture_directory("closed-browser-refresh");
        fs::write(directory.join("original"), b"privacy trace").unwrap();
        let (scan_id, token) = install_directory_scan_session(&directory);
        let mut session = current_scan_session().unwrap();
        let candidate = session.candidates.get_mut(&token).unwrap();
        candidate.item.capability = PrivacyCapabilityState::BrowserRunning;
        candidate.item.requires_browser_close = true;
        candidate.browser_process_names = vec!["mangodisk-browser-not-running".into()];
        session.public_result.items[0] = candidate.item.clone();
        replace_scan_session(session).unwrap();
        let plan = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id,
            tokens: vec![token],
        })
        .unwrap();

        // Simulate the browser flushing one final record while it exits. Execution refreshes only
        // this already-authorized candidate instead of rebuilding the complete privacy catalog.
        fs::write(directory.join("flushed"), b"privacy trace").unwrap();
        let result = PrivacyService::execute(execution_request(plan.plan_id)).unwrap();

        assert_eq!(result.failed_item_count, 0);
        assert_eq!(result.items.len(), 1);
        assert!(result.items[0].verified);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        fs::remove_dir_all(&directory).unwrap();
        clear_scan_session().unwrap();
        clear_pending_plan().unwrap();
    }

    #[test]
    fn execution_exclusions_can_only_reduce_browser_close_sources() {
        let _test_guard = test_operation_lock();
        let directory = fixture_directory("browser-exclusion");
        fs::write(directory.join("history"), b"privacy trace").unwrap();
        let (scan_id, token) = install_directory_scan_session(&directory);
        let mut session = current_scan_session().unwrap();
        let candidate = session.candidates.get_mut(&token).unwrap();
        candidate.item.capability = PrivacyCapabilityState::BrowserRunning;
        candidate.item.requires_browser_close = true;
        candidate.browser_process_names = vec!["mangodisk-browser-not-running".into()];
        session.public_result.items[0] = candidate.item.clone();
        replace_scan_session(session).unwrap();
        let plan = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id,
            tokens: vec![token],
        })
        .unwrap();

        let error = PrivacyService::execute(PrivacyExecutionRunRequest {
            plan_id: plan.plan_id,
            excluded_source_ids: vec!["not-in-plan".into()],
        })
        .unwrap_err();

        assert_eq!(error.code(), CoreErrorCode::InvalidInput);
        assert!(directory.exists());
        clear_scan_session().unwrap();
        clear_pending_plan().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn reconciliation_does_not_authorize_changed_unselected_data() {
        let _test_guard = test_operation_lock();
        let selected_directory = fixture_directory("reconcile-selected");
        let remaining_directory = fixture_directory("reconcile-drifted");
        fs::write(selected_directory.join("selected"), b"privacy trace").unwrap();
        fs::write(remaining_directory.join("original"), b"privacy trace").unwrap();
        let (scan_id, selected_token) = install_directory_scan_session(&selected_directory);
        let remaining_token = "c".repeat(64);
        let remaining_roots = vec![remaining_directory.clone()];
        let remaining_item = privacy_item(PrivacyItemInput {
            token: remaining_token.clone(),
            source_id: "synthetic".into(),
            source_name: "Synthetic application".into(),
            profile_id: None,
            profile_name: None,
            kind: PrivacyDataKind::RecentItems,
            capability: PrivacyCapabilityState::Ready,
            item_count: 1,
            estimated_bytes: 1,
            requires_browser_close: false,
        });
        let mut original_session = current_scan_session().unwrap();
        original_session
            .public_result
            .items
            .push(remaining_item.clone());
        original_session.candidates.insert(
            remaining_token.clone(),
            NativePrivacyCandidate {
                token: remaining_token.clone(),
                item: remaining_item,
                fingerprint: roots_summary_fingerprint(
                    &remaining_roots,
                    &PlatformCancellation::new(|| false),
                )
                .unwrap()
                .0,
                action: NativePrivacyAction::Directories {
                    roots: remaining_roots,
                },
                browser_process_names: Vec::new(),
            },
        );
        original_session.public_result.revision = revision_for(&original_session.candidates);
        replace_scan_session(original_session).unwrap();
        let plan = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id,
            tokens: vec![selected_token],
        })
        .unwrap();

        // Simulate an unselected shared source changing while another item is being cleared.
        fs::write(remaining_directory.join("new-record"), b"new privacy trace").unwrap();
        let result = PrivacyService::execute(execution_request(plan.plan_id)).unwrap();
        let reconciled = result
            .scan
            .expect("execution must publish a reconciled scan");
        let remaining_plan = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id: reconciled.scan_id,
            tokens: vec![remaining_token.clone()],
        })
        .unwrap();
        let result = PrivacyService::execute(execution_request(remaining_plan.plan_id)).unwrap();

        assert_eq!(result.affected_item_count, 0);
        assert_eq!(result.failed_item_count, 1);
        assert_eq!(result.items[0].token, remaining_token);
        assert_eq!(
            result.items[0].failure_reason.as_deref(),
            Some("sourceChanged")
        );
        assert!(remaining_directory.join("original").is_file());
        assert!(remaining_directory.join("new-record").is_file());
        clear_scan_session().unwrap();
        clear_pending_plan().unwrap();
        fs::remove_dir_all(selected_directory).unwrap();
        fs::remove_dir_all(remaining_directory).unwrap();
    }

    #[test]
    #[ignore = "reads the host browser and system privacy source metadata"]
    fn actual_privacy_scan_exposes_only_aggregate_evidence() {
        let _test_guard = test_operation_lock();
        let progress = Mutex::new(Vec::new());
        let result = PrivacyService::scan_with_progress(
            PrivacyScanRequest {
                time_range: PrivacyTimeRange::AllTime,
            },
            |value| progress.lock().unwrap().push(value),
        )
        .expect("host privacy scan must complete");
        let progress = progress.into_inner().unwrap();
        assert_eq!(
            progress.first().map(|value| value.stage),
            Some(PrivacyScanStage::Discovering)
        );
        assert_eq!(
            progress.last().map(|value| value.stage),
            Some(PrivacyScanStage::Finalizing)
        );
        assert!(progress.iter().any(|value| {
            value.stage == PrivacyScanStage::Browser && value.source_name.is_some()
        }));
        assert!(progress.iter().any(|value| {
            value.stage == PrivacyScanStage::Application && value.source_name.is_some()
        }));
        println!(
            "source_count={} item_count={} elapsed_ms={}",
            result.coverage.len(),
            result.items.len(),
            result.elapsed_ms
        );
        assert!(result
            .items
            .iter()
            .all(|item| item.token.len() == 64 && !item.source_name.is_empty()));
        for kind in [
            PrivacyDataKind::BrowserCache,
            PrivacyDataKind::SearchHistory,
            PrivacyDataKind::WebsiteIcons,
            PrivacyDataKind::FrequentlyVisitedSites,
            PrivacyDataKind::AddressBarShortcuts,
            PrivacyDataKind::RecentItems,
            PrivacyDataKind::NetworkConnectionHistory,
        ] {
            let rows = result.items.iter().filter(|item| item.kind == kind).count();
            let count = result
                .items
                .iter()
                .filter(|item| item.kind == kind)
                .map(|item| item.item_count)
                .sum::<u64>();
            println!("expanded_privacy_kind={kind:?} row_count={rows} trace_count={count}");
            assert!(
                rows > 0,
                "expanded source must produce a visible aggregate row"
            );
        }
        #[cfg(target_os = "macos")]
        assert!(result
            .items
            .iter()
            .any(|item| item.kind == PrivacyDataKind::RecentApplications));
        #[cfg(windows)]
        for kind in [
            PrivacyDataKind::ApplicationUsageHistory,
            PrivacyDataKind::FolderViewHistory,
            PrivacyDataKind::PrinterHistory,
        ] {
            assert!(
                result.items.iter().any(|item| item.kind == kind),
                "Windows system privacy source must expose {kind:?}"
            );
        }
        let application_sources = result
            .items
            .iter()
            .filter(|item| item.category == PrivacyCategory::ApplicationActivity)
            .map(|item| item.source_id.as_str())
            .collect::<BTreeSet<_>>();
        println!(
            "application_privacy_source_count={} application_privacy_row_count={}",
            application_sources.len(),
            result
                .items
                .iter()
                .filter(|item| item.category == PrivacyCategory::ApplicationActivity)
                .count()
        );
        assert!(
            application_sources.len() >= 4,
            "host scan must expose multiple independent application sources"
        );
        let mut application_kinds = Vec::new();
        for kind in result
            .items
            .iter()
            .filter(|item| item.category == PrivacyCategory::ApplicationActivity)
            .map(|item| item.kind)
        {
            if !application_kinds.contains(&kind) {
                application_kinds.push(kind);
            }
        }
        assert!(application_kinds.contains(&PrivacyDataKind::ApplicationCache));
        assert!(application_kinds.contains(&PrivacyDataKind::ApplicationLogs));
        assert!(
            application_kinds.len() >= 3,
            "host applications must expose multiple independent privacy trace kinds"
        );
        #[cfg(target_os = "windows")]
        for kind in [
            PrivacyDataKind::JumpLists,
            PrivacyDataKind::RunDialogHistory,
            PrivacyDataKind::FileDialogHistory,
            PrivacyDataKind::SystemSearchHistory,
            PrivacyDataKind::ExplorerPathHistory,
        ] {
            assert!(result.items.iter().any(|item| item.kind == kind));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires Microsoft Edge with an initialized local profile"]
    fn actual_macos_edge_scan_returns_aggregate_evidence() {
        let _test_guard = test_operation_lock();
        let result = PrivacyService::scan(PrivacyScanRequest {
            time_range: PrivacyTimeRange::AllTime,
        })
        .expect("Microsoft Edge privacy scan must complete");
        let edge_items = result
            .items
            .iter()
            .filter(|item| item.source_id == "edge")
            .collect::<Vec<_>>();

        println!(
            "edge_item_count={} edge_trace_count={}",
            edge_items.len(),
            edge_items.iter().map(|item| item.item_count).sum::<u64>()
        );
        assert!(result
            .coverage
            .iter()
            .any(|source| source.source_id == "edge"));
        assert!(!edge_items.is_empty());
        assert!(edge_items.iter().any(|item| item.profile_id.is_some()));
        assert!(edge_items.iter().any(|item| item.item_count > 0));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires the additional Chromium browsers with initialized local profiles"]
    fn actual_macos_additional_chromium_scans_return_aggregate_evidence() {
        let _test_guard = test_operation_lock();
        let result = PrivacyService::scan(PrivacyScanRequest {
            time_range: PrivacyTimeRange::AllTime,
        })
        .expect("additional Chromium privacy scans must complete");

        for source_id in ["brave", "opera", "360_safe_browser", "qq_browser"] {
            let items = result
                .items
                .iter()
                .filter(|item| item.source_id == source_id)
                .collect::<Vec<_>>();
            println!(
                "browser_source={source_id} item_count={} trace_count={}",
                items.len(),
                items.iter().map(|item| item.item_count).sum::<u64>()
            );
            assert!(result
                .coverage
                .iter()
                .any(|source| { source.source_id == source_id && source.icon_path.is_some() }));
            assert!(!items.is_empty());
            assert!(items.iter().any(|item| item.profile_id.is_some()));
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires the supported Windows browsers to be running with initialized profiles"]
    fn actual_windows_browser_scans_return_aggregate_evidence() {
        let _test_guard = test_operation_lock();
        let result = PrivacyService::scan(PrivacyScanRequest {
            time_range: PrivacyTimeRange::AllTime,
        })
        .expect("Windows browser privacy scans must complete");

        for source_id in [
            "firefox",
            "brave",
            "opera",
            "360_safe_browser",
            "qq_browser",
        ] {
            let items = result
                .items
                .iter()
                .filter(|item| item.source_id == source_id)
                .collect::<Vec<_>>();
            println!(
                "browser_source={source_id} item_count={} trace_count={}",
                items.len(),
                items.iter().map(|item| item.item_count).sum::<u64>()
            );
            assert!(result
                .coverage
                .iter()
                .any(|source| { source.source_id == source_id && source.icon_path.is_some() }));
            assert!(!items.is_empty());
            assert!(items.iter().any(|item| item.profile_id.is_some()));
            assert!(items.iter().any(|item| {
                item.item_count > 0
                    && item.capability == PrivacyCapabilityState::BrowserRunning
                    && item.requires_browser_close
            }));
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires a running Google Chrome profile with a Cookie database"]
    fn actual_windows_locked_chrome_cookies_remain_selectable_for_close() {
        let _test_guard = test_operation_lock();
        let result = PrivacyService::scan(PrivacyScanRequest {
            time_range: PrivacyTimeRange::AllTime,
        })
        .expect("Windows Chrome privacy scan must complete");
        let cookies = result
            .items
            .iter()
            .filter(|item| item.source_id == "chrome" && item.kind == PrivacyDataKind::Cookies)
            .collect::<Vec<_>>();

        println!(
            "chrome_cookie_profile_count={} deferred_profile_count={}",
            cookies.len(),
            cookies
                .iter()
                .filter(|item| {
                    item.capability == PrivacyCapabilityState::BrowserRunning
                        && item.item_count == 0
                })
                .count()
        );
        assert!(!cookies.is_empty());
        assert!(cookies.iter().all(|item| {
            item.capability != PrivacyCapabilityState::Unavailable
                && (item.item_count > 0 || privacy_item_is_actionable(item))
        }));
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "closes the supported Windows browsers and rescans their initialized profiles"]
    fn actual_windows_browser_close_and_rescan_clears_running_state() {
        use crate::applications::process_control::ApplicationCloseMode;

        let _test_guard = test_operation_lock();
        let source_ids = [
            "firefox",
            "brave",
            "opera",
            "360_safe_browser",
            "qq_browser",
        ];
        let scan = PrivacyService::scan(PrivacyScanRequest {
            time_range: PrivacyTimeRange::AllTime,
        })
        .expect("running Windows browser scan must complete");
        let tokens = source_ids
            .iter()
            .map(|source_id| {
                scan.items
                    .iter()
                    .find(|item| {
                        item.source_id == *source_id
                            && item.item_count > 0
                            && item.capability == PrivacyCapabilityState::BrowserRunning
                            && !matches!(
                                item.recommendation,
                                PrivacyRecommendation::ReviewOnly
                                    | PrivacyRecommendation::Unsupported
                            )
                    })
                    .unwrap_or_else(|| panic!("{source_id} must expose one close-gated item"))
                    .token
                    .clone()
            })
            .collect::<Vec<_>>();
        let plan = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id: scan.scan_id,
            tokens,
        })
        .expect("running browser plan must be prepared");
        let source_ids = source_ids.map(str::to_owned).to_vec();
        let graceful = PrivacyService::close_browsers(PrivacyBrowserCloseRequest {
            plan_id: plan.plan_id.clone(),
            source_ids: source_ids.clone(),
            mode: ApplicationCloseMode::Graceful,
        })
        .expect("graceful browser close must complete");
        if graceful.remaining_process_count > 0 {
            let forced = PrivacyService::close_browsers(PrivacyBrowserCloseRequest {
                plan_id: plan.plan_id,
                source_ids: source_ids.clone(),
                mode: ApplicationCloseMode::Force,
            })
            .expect("forced browser close retry must complete");
            assert_eq!(forced.remaining_process_count, 0);
        }

        let rescanned = PrivacyService::scan(PrivacyScanRequest {
            time_range: PrivacyTimeRange::AllTime,
        })
        .expect("closed Windows browser rescan must complete");
        for source_id in source_ids {
            let items = rescanned
                .items
                .iter()
                .filter(|item| item.source_id == source_id)
                .collect::<Vec<_>>();
            assert!(items.iter().any(|item| item.item_count > 0));
            assert!(items
                .iter()
                .all(|item| item.capability != PrivacyCapabilityState::BrowserRunning));
        }
    }

    #[test]
    #[ignore = "requires a supported browser with a non-empty profile to be running"]
    fn actual_running_browser_scan_reports_reviewable_counts_and_icon_source() {
        let _test_guard = test_operation_lock();
        let result = PrivacyService::scan(PrivacyScanRequest {
            time_range: PrivacyTimeRange::AllTime,
        })
        .expect("running browser privacy scan must complete");
        let running_source_ids = result
            .items
            .iter()
            .filter(|item| {
                item.capability == PrivacyCapabilityState::BrowserRunning && item.item_count > 0
            })
            .map(|item| item.source_id.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!running_source_ids.is_empty());
        assert!(result.coverage.iter().any(|source| {
            running_source_ids.contains(source.source_id.as_str()) && source.icon_path.is_some()
        }));
        // This host-only check deliberately prints aggregate counts without profile names, paths,
        // URLs, or cookie contents so contributors can compare running-browser coverage safely.
        for kind in [
            PrivacyDataKind::BrowsingHistory,
            PrivacyDataKind::DownloadHistory,
            PrivacyDataKind::Cookies,
            PrivacyDataKind::BrowserCache,
            PrivacyDataKind::SearchHistory,
            PrivacyDataKind::WebsiteIcons,
            PrivacyDataKind::FrequentlyVisitedSites,
            PrivacyDataKind::AddressBarShortcuts,
            PrivacyDataKind::SavedPasswords,
            PrivacyDataKind::AutofillData,
        ] {
            let count = result
                .items
                .iter()
                .filter(|item| item.kind == kind)
                .map(|item| item.item_count)
                .sum::<u64>();
            println!("running_browser_kind={kind:?} aggregate_count={count}");
        }
    }

    #[test]
    #[ignore = "scans installed browser profiles without modifying personal browser data"]
    fn actual_personal_browser_data_is_manual_and_never_selected_by_default() {
        let _test_guard = test_operation_lock();
        let result = PrivacyService::scan(PrivacyScanRequest {
            time_range: PrivacyTimeRange::AllTime,
        })
        .expect("host privacy scan must complete");
        let personal_items = result
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item.kind,
                    PrivacyDataKind::SavedPasswords | PrivacyDataKind::AutofillData
                )
            })
            .collect::<Vec<_>>();
        assert!(
            !personal_items.is_empty(),
            "an installed supported browser must expose personal-data coverage"
        );
        assert!(personal_items.iter().all(|item| {
            item.recommendation == PrivacyRecommendation::Manual && !item.selected_by_default
        }));
        let actionable_count = personal_items
            .iter()
            .filter(|item| {
                item.item_count > 0
                    && matches!(
                        item.capability,
                        PrivacyCapabilityState::Ready | PrivacyCapabilityState::BrowserRunning
                    )
            })
            .map(|item| item.item_count)
            .sum::<u64>();
        println!("personal_browser_data_actionable_count={actionable_count}");
    }

    #[test]
    #[ignore = "reads bounded detail pages from installed privacy sources without modifying them"]
    fn actual_privacy_detail_pages_match_the_active_scan() {
        let _test_guard = test_operation_lock();
        let scan = PrivacyService::scan(PrivacyScanRequest {
            time_range: PrivacyTimeRange::AllTime,
        })
        .expect("host privacy scan must complete");
        let populated = scan
            .items
            .iter()
            .filter(|item| item.item_count > 0)
            .collect::<Vec<_>>();
        assert!(
            !populated.is_empty(),
            "the host must expose at least one populated privacy source"
        );
        #[cfg(windows)]
        {
            let recent_rows = scan
                .items
                .iter()
                .filter(|item| item.kind == PrivacyDataKind::RecentItems)
                .collect::<Vec<_>>();
            assert_eq!(
                recent_rows.len(),
                1,
                "Windows Recent and RecentDocs must be presented as one logical source"
            );
            assert_eq!(recent_rows[0].source_id, "recent_document_history");
        }
        for item in populated {
            let page = PrivacyService::details(PrivacyDetailsRequest {
                scan_id: scan.scan_id.clone(),
                token: item.token.clone(),
                offset: 0,
                limit: 5,
            })
            .unwrap_or_else(|error| {
                panic!(
                    "detail page failed source_id={} kind={:?} diagnostic={}",
                    item.source_id,
                    item.kind,
                    error.diagnostic()
                )
            });
            assert_eq!(page.total_item_count, item.item_count);
            assert!(page.entries.len() <= 5);
            assert!(page.entries.iter().all(|entry| entry.item_count > 0));
            #[cfg(windows)]
            if item.source_id == "recent_document_history" {
                assert!(page.entries.iter().all(|entry| {
                    Path::new(&entry.label)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
                }));
            }
            // Never print labels: this ignored validation can run against real personal data.
            println!(
                "privacy_detail_validated source_id={} kind={:?} presentation={:?} entry_count={}",
                item.source_id,
                item.kind,
                page.presentation,
                page.entries.len()
            );
        }
    }

    #[test]
    #[ignore = "clears the current system clipboard through the native platform adapter"]
    fn actual_current_clipboard_execution_is_verified() {
        let _test_guard = test_operation_lock();
        let scan = PrivacyService::scan(PrivacyScanRequest {
            time_range: PrivacyTimeRange::AllTime,
        })
        .expect("host privacy scan must complete");
        let clipboard = scan
            .items
            .iter()
            .find(|item| {
                item.kind == PrivacyDataKind::CurrentClipboard
                    && item.capability == PrivacyCapabilityState::Ready
                    && item.item_count > 0
            })
            .expect("current clipboard must be actionable after synthetic test data is inserted");
        let plan = PrivacyService::prepare(PrivacyExecutionRequest {
            scan_id: scan.scan_id.clone(),
            tokens: vec![clipboard.token.clone()],
        })
        .expect("current clipboard plan must be prepared");
        let result = PrivacyService::execute(execution_request(plan.plan_id))
            .expect("current clipboard execution must complete");
        assert_eq!(result.failed_item_count, 0);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].status, PrivacyExecutionItemStatus::Cleared);
        assert!(result.items[0].verified);
        assert!(result.affected_item_count > 0);
    }
}
