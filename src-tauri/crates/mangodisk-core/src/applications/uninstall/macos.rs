use std::{
    fs,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Instant, UNIX_EPOCH},
};

use mangodisk_platform::{
    current_platform, ApplicationComponentAggregate, ApplicationComponentAggregateError, Platform,
};
use plist::Value;

use crate::{
    filesystem::metadata::{display_path, now_ms},
    shared::{
        operation::OPERATION_CANCELLED_ERROR,
        progress::{ProgressTracker, TraversalStage},
    },
};

use super::models::{
    ApplicationUninstallCandidate, ApplicationUninstallComponent,
    ApplicationUninstallComponentKind, ApplicationUninstallComponentSummary,
    ApplicationUninstallInspection, ApplicationUninstallRisk,
    APPLICATION_UNINSTALL_INSPECTION_SCHEMA_VERSION,
};

const MAX_APPLICATION_CONTAINER_DEPTH: usize = 1;
const PORTABLE_PROGRESS_ENTRY_BATCH: u64 = 4_096;

struct Association {
    kind: ApplicationUninstallComponentKind,
    risk: ApplicationUninstallRisk,
    path: PathBuf,
    default_selected: bool,
}

struct PathSnapshot {
    bytes: u64,
    file_count: u64,
    fingerprint: blake3::Hasher,
    complete: bool,
}

#[derive(Debug)]
struct PathAggregate {
    bytes: u64,
    file_count: u64,
    skipped_count: u64,
    strategy: &'static str,
}

impl PathAggregate {
    fn complete(&self) -> bool {
        self.skipped_count == 0
    }
}

/// Per-scan diagnostics are accumulated outside serialized product models. These fields explain
/// whether performance came from Spotlight, native bulk enumeration, or the safe portable fallback
/// without exposing a user's component paths in logs.
#[derive(Debug, Default)]
pub(super) struct ComponentSummaryMetrics {
    pub(super) native_component_count: u64,
    pub(super) portable_fallback_count: u64,
    pub(super) spotlight_size_hit_count: u64,
    pub(super) spotlight_size_fallback_count: u64,
    pub(super) association_tree_count: u64,
    pub(super) measured_file_count: u64,
    pub(super) measured_bytes: u64,
    pub(super) incomplete_component_count: u64,
}

impl Default for PathSnapshot {
    fn default() -> Self {
        let mut fingerprint = blake3::Hasher::new();
        fingerprint.update(b"mangodisk-uninstall-component-v1");
        Self {
            bytes: 0,
            file_count: 0,
            fingerprint,
            complete: false,
        }
    }
}

pub(super) fn summarize_candidate(
    candidate: &ApplicationUninstallCandidate,
    cancellation: &AtomicBool,
    progress: Option<&ProgressTracker>,
    metrics: &mut ComponentSummaryMetrics,
) -> Result<(Vec<ApplicationUninstallComponentSummary>, bool), String> {
    let Some(application_path) = candidate.application_path.as_deref().map(PathBuf::from) else {
        return Ok((Vec::new(), false));
    };
    let Ok(user_directories) = current_platform().user_directories() else {
        return Ok((Vec::new(), false));
    };
    let home = user_directories.home_directory();
    if !safe_application_path(&application_path, home) {
        return Ok((Vec::new(), false));
    }

    let mut components = vec![summarize_application_binary(
        candidate,
        &application_path,
        cancellation,
        progress,
        metrics,
    )?];
    if !safe_bundle_identifier(&candidate.primary_identifier)
        || !bundle_identifier_matches(&application_path, &candidate.primary_identifier)
    {
        return Ok((components, false));
    }

    let mut complete = true;
    for association in exact_bundle_associations(home, &candidate.primary_identifier) {
        if !association.path.exists() {
            continue;
        }
        if !safe_user_association_path(home, &association.path) {
            complete = false;
            metrics.incomplete_component_count =
                metrics.incomplete_component_count.saturating_add(1);
            continue;
        }
        metrics.association_tree_count = metrics.association_tree_count.saturating_add(1);
        let aggregate = aggregate_path(&association.path, cancellation, progress, metrics)?;
        if !aggregate.complete() {
            complete = false;
            metrics.incomplete_component_count =
                metrics.incomplete_component_count.saturating_add(1);
            continue;
        }
        components.push(ApplicationUninstallComponentSummary {
            component_id: component_id(association.kind, &association.path),
            kind: association.kind,
            risk: association.risk,
            path: Some(display_path(&association.path)),
            bytes: aggregate.bytes,
            file_count: aggregate.file_count,
            default_selected: association.default_selected,
        });
    }
    components.sort_by_key(|component| component.kind as u8);
    Ok((components, complete))
}

fn summarize_application_binary(
    candidate: &ApplicationUninstallCandidate,
    application_path: &Path,
    cancellation: &AtomicBool,
    progress: Option<&ProgressTracker>,
    metrics: &mut ComponentSummaryMetrics,
) -> Result<ApplicationUninstallComponentSummary, String> {
    let (bytes, file_count) = if candidate.estimated_bytes > 0 {
        metrics.spotlight_size_hit_count = metrics.spotlight_size_hit_count.saturating_add(1);
        (candidate.estimated_bytes, 0)
    } else {
        // Spotlight does not publish kMDItemFSSize for every valid bundle.
        // Traverse only the affected bundle so the catalog never presents
        // associated-data bytes as if they were the complete application size.
        let started = Instant::now();
        metrics.spotlight_size_fallback_count =
            metrics.spotlight_size_fallback_count.saturating_add(1);
        let aggregate = aggregate_path(application_path, cancellation, progress, metrics)?;
        if aggregate.complete() {
            log::debug!(
                "application_uninstall_bundle_size_fallback bundle={} bytes={} file_count={} strategy={} elapsed_ms={}",
                application_path
                    .file_name()
                    .map(|value| value.to_string_lossy())
                    .unwrap_or_default(),
                aggregate.bytes,
                aggregate.file_count,
                aggregate.strategy,
                started.elapsed().as_millis()
            );
            (aggregate.bytes, aggregate.file_count)
        } else {
            metrics.incomplete_component_count =
                metrics.incomplete_component_count.saturating_add(1);
            log::debug!(
                "application_uninstall_bundle_size_fallback_failed bundle={} elapsed_ms={}",
                application_path
                    .file_name()
                    .map(|value| value.to_string_lossy())
                    .unwrap_or_default(),
                started.elapsed().as_millis()
            );
            (0, 0)
        }
    };

    Ok(ApplicationUninstallComponentSummary {
        component_id: component_id(
            ApplicationUninstallComponentKind::ApplicationBinary,
            application_path,
        ),
        kind: ApplicationUninstallComponentKind::ApplicationBinary,
        risk: ApplicationUninstallRisk::Required,
        path: Some(display_path(application_path)),
        bytes,
        file_count,
        default_selected: true,
    })
}

/// Produces only the catalog aggregate. The safety snapshot below remains unchanged and is still
/// rebuilt for inspection, plan creation, and every preflight comparison.
fn aggregate_path(
    path: &Path,
    cancellation: &AtomicBool,
    progress: Option<&ProgressTracker>,
    metrics: &mut ComponentSummaryMetrics,
) -> Result<PathAggregate, String> {
    ensure_not_cancelled(cancellation)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => {
            return Ok(PathAggregate {
                bytes: 0,
                file_count: 0,
                skipped_count: 1,
                strategy: "portable-application-metadata",
            })
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok(PathAggregate {
            bytes: 0,
            file_count: 0,
            skipped_count: 0,
            strategy: "portable-application-link",
        });
    }
    if metadata.is_file() {
        metrics.measured_file_count = metrics.measured_file_count.saturating_add(1);
        metrics.measured_bytes = metrics.measured_bytes.saturating_add(metadata.len());
        if let Some(progress) = progress {
            progress.observe_files(
                TraversalStage::InspectingApplications,
                path,
                1,
                metadata.len(),
            );
        }
        return Ok(PathAggregate {
            bytes: metadata.len(),
            file_count: 1,
            skipped_count: 0,
            strategy: "portable-application-file-metadata",
        });
    }
    if !metadata.is_dir() {
        return Ok(PathAggregate {
            bytes: 0,
            file_count: 0,
            skipped_count: 1,
            strategy: "portable-application-unsupported-entry",
        });
    }

    let mut native_observation = progress.map(ProgressTracker::begin_scan_observation);
    let native_progress = |current: &Path, files: u64, bytes: u64| {
        if let Some(observation) = native_observation.as_ref() {
            observation.observe(
                TraversalStage::InspectingApplications,
                current,
                files,
                bytes,
            );
        }
    };
    match current_platform().fast_application_component_aggregate(
        path,
        &|| cancellation.load(Ordering::Relaxed),
        &native_progress,
    ) {
        Ok(Some(aggregate)) => {
            if let Some(observation) = native_observation.as_mut() {
                observation.commit_exact(
                    TraversalStage::InspectingApplications,
                    path,
                    aggregate.file_count,
                    aggregate.bytes,
                );
            }
            metrics.native_component_count = metrics.native_component_count.saturating_add(1);
            record_measurement(metrics, &aggregate);
            return Ok(PathAggregate {
                bytes: aggregate.bytes,
                file_count: aggregate.file_count,
                skipped_count: aggregate.skipped_count,
                strategy: aggregate.strategy,
            });
        }
        Ok(None) => {}
        Err(ApplicationComponentAggregateError::Cancelled) => {
            return Err(OPERATION_CANCELLED_ERROR.to_string())
        }
        Err(ApplicationComponentAggregateError::Platform(error)) => {
            log::warn!(
                "application_component_native_aggregate_failed error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
        }
    }
    // Dropping an uncommitted lease removes any partial native observations before the portable
    // retry starts, keeping user-visible progress exact even after a late native failure.
    drop(native_observation);
    metrics.portable_fallback_count = metrics.portable_fallback_count.saturating_add(1);
    let aggregate = portable_aggregate_path(path, cancellation, progress)?;
    metrics.measured_file_count = metrics
        .measured_file_count
        .saturating_add(aggregate.file_count);
    metrics.measured_bytes = metrics.measured_bytes.saturating_add(aggregate.bytes);
    Ok(aggregate)
}

fn record_measurement(
    metrics: &mut ComponentSummaryMetrics,
    aggregate: &ApplicationComponentAggregate,
) {
    metrics.measured_file_count = metrics
        .measured_file_count
        .saturating_add(aggregate.file_count);
    metrics.measured_bytes = metrics.measured_bytes.saturating_add(aggregate.bytes);
}

fn portable_aggregate_path(
    root: &Path,
    cancellation: &AtomicBool,
    progress: Option<&ProgressTracker>,
) -> Result<PathAggregate, String> {
    let mut aggregate = PathAggregate {
        bytes: 0,
        file_count: 0,
        skipped_count: 0,
        strategy: "portable-application-read-dir-v1",
    };
    let mut directories = vec![root.to_path_buf()];
    let mut pending_entries = 0_u64;
    let mut pending_files = 0_u64;
    let mut pending_bytes = 0_u64;
    while let Some(directory) = directories.pop() {
        ensure_not_cancelled(cancellation)?;
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                aggregate.skipped_count = aggregate.skipped_count.saturating_add(1);
                continue;
            }
        };
        for entry in entries {
            ensure_not_cancelled(cancellation)?;
            pending_entries = pending_entries.saturating_add(1);
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    aggregate.skipped_count = aggregate.skipped_count.saturating_add(1);
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => {
                    aggregate.skipped_count = aggregate.skipped_count.saturating_add(1);
                    continue;
                }
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file() {
                aggregate.bytes = aggregate.bytes.saturating_add(metadata.len());
                aggregate.file_count = aggregate.file_count.saturating_add(1);
                pending_files = pending_files.saturating_add(1);
                pending_bytes = pending_bytes.saturating_add(metadata.len());
            } else {
                aggregate.skipped_count = aggregate.skipped_count.saturating_add(1);
            }
            if pending_entries >= PORTABLE_PROGRESS_ENTRY_BATCH {
                if let Some(progress) = progress {
                    progress.observe_files(
                        TraversalStage::InspectingApplications,
                        &directory,
                        pending_files,
                        pending_bytes,
                    );
                }
                pending_entries = 0;
                pending_files = 0;
                pending_bytes = 0;
            }
        }
    }
    if let Some(progress) = progress {
        if pending_entries > 0 || pending_files > 0 || pending_bytes > 0 {
            progress.observe_files(
                TraversalStage::InspectingApplications,
                root,
                pending_files,
                pending_bytes,
            );
        }
    }
    Ok(aggregate)
}

fn ensure_not_cancelled(cancellation: &AtomicBool) -> Result<(), String> {
    if cancellation.load(Ordering::Relaxed) {
        Err(OPERATION_CANCELLED_ERROR.to_string())
    } else {
        Ok(())
    }
}

pub(super) fn inspect_candidate(
    candidate: &ApplicationUninstallCandidate,
    catalog_revision: &str,
    started: Instant,
) -> Result<ApplicationUninstallInspection, String> {
    let application_path = candidate
        .application_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "application uninstall candidate has no application path".to_string())?;
    let user_directories = current_platform()
        .user_directories()
        .map_err(|error| error.to_string())?;
    let home = user_directories.home_directory();
    if !safe_application_path(&application_path, home) {
        return Err("application path is not safe for uninstall inspection".to_string());
    }
    let has_verified_bundle_identifier = safe_bundle_identifier(&candidate.primary_identifier)
        && bundle_identifier_matches(&application_path, &candidate.primary_identifier);

    let mut associations = vec![Association {
        kind: ApplicationUninstallComponentKind::ApplicationBinary,
        risk: ApplicationUninstallRisk::Required,
        path: application_path,
        default_selected: true,
    }];
    if has_verified_bundle_identifier {
        associations.extend(exact_bundle_associations(
            home,
            &candidate.primary_identifier,
        ));
    } else {
        log::debug!(
            "application_uninstall_associations_skipped reason=unverified_bundle_identifier"
        );
    }

    let mut components = Vec::new();
    for association in associations {
        if association.kind != ApplicationUninstallComponentKind::ApplicationBinary
            && !safe_user_association_path(home, &association.path)
        {
            log::debug!(
                "application_uninstall_component_skipped reason=unsafe_association_path kind={:?}",
                association.kind
            );
            continue;
        }
        if !association.path.exists() {
            continue;
        }
        let snapshot = snapshot_path(&association.path);
        if !snapshot.complete {
            log::debug!(
                "application_uninstall_component_skipped reason=incomplete_snapshot kind={:?}",
                association.kind
            );
            continue;
        }
        let fingerprint = complete_fingerprint(&snapshot);
        let path = display_path(&association.path);
        components.push(ApplicationUninstallComponent {
            component_id: component_id(association.kind, &association.path),
            kind: association.kind,
            risk: association.risk,
            path: Some(path),
            bytes: snapshot.bytes,
            file_count: snapshot.file_count,
            default_selected: association.default_selected,
            snapshot_fingerprint: fingerprint,
        });
    }
    components.sort_by_key(|component| component.kind as u8);
    if !components
        .iter()
        .any(|component| component.kind == ApplicationUninstallComponentKind::ApplicationBinary)
    {
        return Err("application binary could not be measured safely".to_string());
    }
    let total_bytes = components.iter().fold(0_u64, |total, component| {
        total.saturating_add(component.bytes)
    });
    let default_selected_bytes = components
        .iter()
        .filter(|component| component.default_selected)
        .fold(0_u64, |total, component| {
            total.saturating_add(component.bytes)
        });

    Ok(ApplicationUninstallInspection {
        schema_version: APPLICATION_UNINSTALL_INSPECTION_SCHEMA_VERSION,
        inspected_at_ms: now_ms(),
        application_id: candidate.application_id.clone(),
        application_name: candidate.name.clone(),
        primary_identifier: candidate.primary_identifier.clone(),
        platform: candidate.platform,
        installer_kind: candidate.installer_kind,
        capability: candidate.capability,
        catalog_revision: catalog_revision.to_string(),
        components,
        total_bytes,
        default_selected_bytes,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

fn exact_bundle_associations(home: &Path, identifier: &str) -> Vec<Association> {
    let library = home.join("Library");
    [
        (
            ApplicationUninstallComponentKind::Cache,
            ApplicationUninstallRisk::Rebuildable,
            library.join("Caches").join(identifier),
            true,
        ),
        (
            ApplicationUninstallComponentKind::ApplicationSupport,
            ApplicationUninstallRisk::UserData,
            library.join("Application Support").join(identifier),
            false,
        ),
        (
            ApplicationUninstallComponentKind::Preferences,
            ApplicationUninstallRisk::UserData,
            library
                .join("Preferences")
                .join(format!("{identifier}.plist")),
            false,
        ),
        (
            ApplicationUninstallComponentKind::Logs,
            ApplicationUninstallRisk::Rebuildable,
            library.join("Logs").join(identifier),
            true,
        ),
        (
            ApplicationUninstallComponentKind::SavedState,
            ApplicationUninstallRisk::Rebuildable,
            library
                .join("Saved Application State")
                .join(format!("{identifier}.savedState")),
            true,
        ),
        (
            ApplicationUninstallComponentKind::SandboxContainer,
            ApplicationUninstallRisk::UserData,
            library.join("Containers").join(identifier),
            false,
        ),
        (
            ApplicationUninstallComponentKind::WebData,
            ApplicationUninstallRisk::UserData,
            library.join("WebKit").join(identifier),
            false,
        ),
        (
            ApplicationUninstallComponentKind::WebData,
            ApplicationUninstallRisk::Rebuildable,
            library.join("HTTPStorages").join(identifier),
            true,
        ),
    ]
    .into_iter()
    .map(|(kind, risk, path, default_selected)| Association {
        kind,
        risk,
        path,
        default_selected,
    })
    .collect()
}

fn bundle_identifier_matches(application_path: &Path, expected: &str) -> bool {
    let Some(info_path) = application_info_path(application_path) else {
        return false;
    };
    Value::from_file(info_path)
        .ok()
        .and_then(Value::into_dictionary)
        .and_then(|dictionary| {
            dictionary
                .get("CFBundleIdentifier")
                .and_then(Value::as_string)
                .map(str::to_string)
        })
        .is_some_and(|identifier| identifier == expected)
}

fn application_info_path(application_path: &Path) -> Option<PathBuf> {
    let standard_info = application_path.join("Contents/Info.plist");
    if standard_info.is_file() {
        return Some(standard_info);
    }

    // App Store iOS applications may use an outer wrapper whose
    // `WrappedBundle` link targets a flat bundle inside the same application.
    // Resolving both sides prevents a crafted link from reading identity
    // metadata outside the application selected for uninstall.
    let canonical_application = fs::canonicalize(application_path).ok()?;
    let wrapped_bundle = fs::canonicalize(application_path.join("WrappedBundle")).ok()?;
    if !wrapped_bundle.starts_with(&canonical_application)
        || !wrapped_bundle
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    {
        return None;
    }
    wrapped_bundle
        .join("Info.plist")
        .is_file()
        .then(|| wrapped_bundle.join("Info.plist"))
}

fn safe_bundle_identifier(identifier: &str) -> bool {
    !identifier.is_empty()
        && identifier.len() <= 255
        && !identifier.starts_with('.')
        && !identifier.ends_with('.')
        && !identifier.contains("..")
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

fn safe_application_path(path: &Path, home: &Path) -> bool {
    let system_root = Path::new("/Applications");
    let user_root = home.join("Applications");
    let safe = [system_root, user_root.as_path()].into_iter().any(|root| {
        let Ok(relative) = path.strip_prefix(root) else {
            return false;
        };
        let container_depth = relative
            .parent()
            .map(|parent| parent.components().count())
            .unwrap_or(0);
        container_depth <= MAX_APPLICATION_CONTAINER_DEPTH
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
            && safe_existing_path(root, path)
            && fs::symlink_metadata(path)
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    });
    safe
}

fn safe_user_association_path(home: &Path, path: &Path) -> bool {
    path.parent().is_some()
        && path.starts_with(home)
        && safe_existing_path(home, path)
        && fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.file_type().is_symlink())
}

fn safe_existing_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return false;
    }
    let mut current = root.to_path_buf();
    if !fs::symlink_metadata(&current).is_ok_and(|metadata| !metadata.file_type().is_symlink()) {
        return false;
    }
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return false;
        };
        current.push(component);
        if !fs::symlink_metadata(&current).is_ok_and(|metadata| !metadata.file_type().is_symlink())
        {
            return false;
        }
    }
    true
}

fn snapshot_path(path: &Path) -> PathSnapshot {
    let mut snapshot = PathSnapshot {
        complete: true,
        ..PathSnapshot::default()
    };
    snapshot_entry(path, path, &mut snapshot);
    snapshot
}

fn snapshot_entry(root: &Path, path: &Path, snapshot: &mut PathSnapshot) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        snapshot.complete = false;
        return;
    };
    let relative = path.strip_prefix(root).unwrap_or(path);
    let Some(modified) = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
    else {
        snapshot.complete = false;
        return;
    };
    let kind = if metadata.file_type().is_symlink() {
        3
    } else if metadata.is_dir() {
        2
    } else if metadata.is_file() {
        1
    } else {
        snapshot.complete = false;
        return;
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-uninstall-component-entry-v1");
    hasher.update(relative.as_os_str().as_bytes());
    hasher.update(&[kind]);
    hasher.update(&metadata.len().to_le_bytes());
    hasher.update(&modified.to_le_bytes());
    if metadata.file_type().is_symlink() {
        match fs::read_link(path) {
            Ok(target) => hasher.update(target.as_os_str().as_bytes()),
            Err(_) => {
                snapshot.complete = false;
                return;
            }
        };
    }
    snapshot.fingerprint.update(hasher.finalize().as_bytes());

    if metadata.is_file() {
        snapshot.bytes = snapshot.bytes.saturating_add(metadata.len());
        snapshot.file_count = snapshot.file_count.saturating_add(1);
    } else if metadata.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            snapshot.complete = false;
            return;
        };
        let mut children = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => children.push(entry.path()),
                Err(_) => snapshot.complete = false,
            }
        }
        children.sort();
        for child in children {
            snapshot_entry(root, &child, snapshot);
        }
    }
}

fn complete_fingerprint(snapshot: &PathSnapshot) -> String {
    let mut hasher = snapshot.fingerprint.clone();
    hasher.update(&snapshot.bytes.to_le_bytes());
    hasher.update(&snapshot.file_count.to_le_bytes());
    hasher.finalize().to_hex().to_string()
}

pub(super) fn component_matches(component: &ApplicationUninstallComponent) -> bool {
    let Some(path) = component.path.as_deref() else {
        return false;
    };
    let snapshot = snapshot_path(Path::new(path));
    snapshot.complete
        && snapshot.bytes == component.bytes
        && snapshot.file_count == component.file_count
        && complete_fingerprint(&snapshot) == component.snapshot_fingerprint
}

pub(super) fn component_is_absent(component: &ApplicationUninstallComponent) -> bool {
    let Some(path) = component.path.as_deref() else {
        return false;
    };
    fs::symlink_metadata(path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

fn component_id(kind: ApplicationUninstallComponentKind, path: &Path) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-uninstall-component-id-v1");
    hasher.update(kind.stable_code().as_bytes());
    hasher.update(path.as_os_str().as_bytes());
    format!("component-{}", &hasher.finalize().to_hex()[..24])
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::symlink,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "mangodisk-uninstall-{label}-{}-{}",
                std::process::id(),
                NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("temporary directory must be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn component_snapshot_does_not_follow_symlinks() {
        let root = TestDirectory::new("application");
        let outside = TestDirectory::new("outside");
        fs::write(outside.path().join("private.txt"), b"outside")
            .expect("outside fixture must be written");
        symlink(outside.path(), root.path().join("linked"))
            .expect("fixture symlink must be created");
        fs::write(root.path().join("inside.txt"), b"inside")
            .expect("inside fixture must be written");

        let snapshot = snapshot_path(root.path());

        assert!(snapshot.complete);
        assert_eq!(snapshot.file_count, 1);
        assert_eq!(snapshot.bytes, 6);
    }

    #[test]
    fn catalog_aggregate_matches_the_safety_snapshot_for_physical_files() {
        let root = TestDirectory::new("catalog-snapshot-equivalence");
        fs::create_dir_all(root.path().join("nested/deep"))
            .expect("nested fixture must be created");
        fs::write(root.path().join("direct.bin"), [0_u8; 3])
            .expect("direct fixture must be written");
        fs::write(root.path().join("nested/child.bin"), [0_u8; 7])
            .expect("child fixture must be written");
        fs::write(root.path().join("nested/deep/grandchild.bin"), [0_u8; 11])
            .expect("grandchild fixture must be written");
        symlink("direct.bin", root.path().join("linked.bin"))
            .expect("fixture symlink must be created");

        let aggregate = aggregate_path(
            root.path(),
            &AtomicBool::new(false),
            None,
            &mut ComponentSummaryMetrics::default(),
        )
        .expect("catalog aggregate must succeed");
        let snapshot = snapshot_path(root.path());

        assert!(snapshot.complete);
        assert_eq!(aggregate.skipped_count, 0);
        assert_eq!(aggregate.bytes, snapshot.bytes);
        assert_eq!(aggregate.file_count, snapshot.file_count);
    }

    #[test]
    fn catalog_aggregate_honors_cancellation_before_accessing_the_path() {
        let missing = Path::new("/path-that-must-not-be-accessed-after-cancellation");

        let result = aggregate_path(
            missing,
            &AtomicBool::new(true),
            None,
            &mut ComponentSummaryMetrics::default(),
        );

        assert!(matches!(result, Err(error) if error == OPERATION_CANCELLED_ERROR));
    }

    #[test]
    fn application_summary_measures_bundle_when_platform_estimate_is_missing() {
        let bundle = TestDirectory::new("missing-estimate");
        fs::write(bundle.path().join("binary"), b"application")
            .expect("application fixture must be written");
        let candidate = ApplicationUninstallCandidate {
            system_kind: Default::default(),
            application_id: "application-example".to_string(),
            primary_identifier: "com.example.Editor".to_string(),
            source_identities: Vec::new(),
            name: "Editor".to_string(),
            version: None,
            publisher: None,
            estimated_bytes: 0,
            last_used_at_ms: None,
            installed_at_ms: None,
            platform: super::super::models::ApplicationUninstallPlatform::MacosBundle,
            installer_kind: None,
            execution_mode: None,
            capability: super::super::models::ApplicationUninstallCapability::Ready,
            record_state: super::super::models::ApplicationUninstallRecordState::Installed,
            uninstall_diagnostic: None,
            application_path: Some(display_path(bundle.path())),
            possible_related_paths: Vec::new(),
            icon_path: None,
            running_processes: Vec::new(),
            executable_paths: Vec::new(),
            total_bytes: 0,
            default_selected_bytes: 0,
            associated_data_complete: false,
            components: Vec::new(),
        };

        let summary = summarize_application_binary(
            &candidate,
            bundle.path(),
            &AtomicBool::new(false),
            None,
            &mut ComponentSummaryMetrics::default(),
        )
        .expect("application aggregate must succeed");

        assert_eq!(summary.bytes, 11);
        assert_eq!(summary.file_count, 1);
        assert_eq!(
            summary.kind,
            ApplicationUninstallComponentKind::ApplicationBinary
        );
    }

    #[test]
    fn application_summary_keeps_available_platform_estimate() {
        let bundle = TestDirectory::new("available-estimate");
        fs::write(bundle.path().join("binary"), b"application")
            .expect("application fixture must be written");
        let candidate = ApplicationUninstallCandidate {
            system_kind: Default::default(),
            application_id: "application-example".to_string(),
            primary_identifier: "com.example.Editor".to_string(),
            source_identities: Vec::new(),
            name: "Editor".to_string(),
            version: None,
            publisher: None,
            estimated_bytes: 42,
            last_used_at_ms: None,
            installed_at_ms: None,
            platform: super::super::models::ApplicationUninstallPlatform::MacosBundle,
            installer_kind: None,
            execution_mode: None,
            capability: super::super::models::ApplicationUninstallCapability::Ready,
            record_state: super::super::models::ApplicationUninstallRecordState::Installed,
            uninstall_diagnostic: None,
            application_path: Some(display_path(bundle.path())),
            possible_related_paths: Vec::new(),
            icon_path: None,
            running_processes: Vec::new(),
            executable_paths: Vec::new(),
            total_bytes: 0,
            default_selected_bytes: 0,
            associated_data_complete: false,
            components: Vec::new(),
        };

        let summary = summarize_application_binary(
            &candidate,
            bundle.path(),
            &AtomicBool::new(false),
            None,
            &mut ComponentSummaryMetrics::default(),
        )
        .expect("Spotlight estimate must not require aggregation");

        assert_eq!(summary.bytes, 42);
        assert_eq!(summary.file_count, 0);
    }

    #[test]
    fn exact_associations_never_use_display_names() {
        let home = Path::new("/Users/example");
        let associations = exact_bundle_associations(home, "com.example.Editor");

        assert!(associations.iter().all(|association| association
            .path
            .to_string_lossy()
            .contains("com.example.Editor")));
    }

    #[test]
    fn associations_require_the_bundle_identifier_from_info_plist() {
        let root = TestDirectory::new("bundle-id");
        let contents = root.path().join("Contents");
        fs::create_dir_all(&contents).expect("bundle contents must be created");
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert(
            "CFBundleIdentifier".to_string(),
            Value::String("com.example.Editor".to_string()),
        );
        Value::Dictionary(dictionary)
            .to_file_xml(contents.join("Info.plist"))
            .expect("fixture Info.plist must be written");

        assert!(bundle_identifier_matches(root.path(), "com.example.Editor"));
        assert!(!bundle_identifier_matches(root.path(), "Editor"));
    }

    #[test]
    fn wrapped_application_identifier_is_read_from_the_inner_bundle() {
        let root = TestDirectory::new("wrapped-bundle-id");
        let application = root.path().join("Example.app");
        let wrapped = application.join("Wrapper/Example.app");
        fs::create_dir_all(&wrapped).expect("wrapped application fixture must be created");
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert(
            "CFBundleIdentifier".to_string(),
            Value::String("com.example.Wrapped".to_string()),
        );
        Value::Dictionary(dictionary)
            .to_file_xml(wrapped.join("Info.plist"))
            .expect("wrapped Info.plist fixture must be written");
        symlink("Wrapper/Example.app", application.join("WrappedBundle"))
            .expect("wrapped bundle link must be created");

        assert!(bundle_identifier_matches(
            &application,
            "com.example.Wrapped"
        ));
    }

    #[test]
    fn wrapped_application_identifier_cannot_escape_the_outer_bundle() {
        let root = TestDirectory::new("wrapped-bundle-escape");
        let application = root.path().join("Example.app");
        let external = root.path().join("External.app");
        fs::create_dir_all(&application).expect("outer application fixture must be created");
        fs::create_dir_all(&external).expect("external application fixture must be created");
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert(
            "CFBundleIdentifier".to_string(),
            Value::String("com.example.External".to_string()),
        );
        Value::Dictionary(dictionary)
            .to_file_xml(external.join("Info.plist"))
            .expect("external Info.plist fixture must be written");
        symlink(&external, application.join("WrappedBundle"))
            .expect("escaping wrapped bundle link must be created");

        assert!(!bundle_identifier_matches(
            &application,
            "com.example.External"
        ));
    }

    #[test]
    fn bundle_identifiers_cannot_escape_standard_directories() {
        assert!(safe_bundle_identifier("com.example.Editor"));
        assert!(!safe_bundle_identifier("../../Documents"));
        assert!(!safe_bundle_identifier("com/example/Editor"));
        assert!(!safe_bundle_identifier(".com.example"));
    }

    #[test]
    fn application_bundle_cannot_be_reached_through_a_symlink() {
        let home = TestDirectory::new("home");
        let applications = home.path().join("Applications");
        let actual = home.path().join("Actual.app");
        fs::create_dir_all(&applications).expect("applications fixture must be created");
        fs::create_dir_all(&actual).expect("application fixture must be created");
        symlink(&actual, applications.join("Linked.app"))
            .expect("application symlink must be created");

        assert!(!safe_application_path(
            &applications.join("Linked.app"),
            home.path()
        ));
    }

    #[test]
    fn application_bundle_may_use_one_real_container_directory() {
        let home = TestDirectory::new("nested-home");
        let applications = home.path().join("Applications");
        let chrome_apps = applications.join("Chrome Apps.localized");
        let application = chrome_apps.join("Google Gemini.app");
        fs::create_dir_all(&application).expect("nested application fixture must be created");

        assert!(safe_application_path(&application, home.path()));
    }

    #[test]
    fn application_bundle_cannot_use_arbitrary_nested_directories() {
        let home = TestDirectory::new("deep-home");
        let applications = home.path().join("Applications");
        let application = applications
            .join("Vendor")
            .join("Archive")
            .join("Unexpected.app");
        fs::create_dir_all(&application).expect("deep application fixture must be created");

        assert!(!safe_application_path(&application, home.path()));
    }
}
