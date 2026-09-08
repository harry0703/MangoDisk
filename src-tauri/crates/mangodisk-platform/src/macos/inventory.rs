use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use plist::Value;

use crate::{
    command::{
        run_controlled_command, ControlledCommandLimits, ControlledEnvironmentPolicy,
        ControlledExecutable,
    },
    inventory::{detect_tools, normalize_fact},
    ApplicationInventorySource, ApplicationSourceIdentity, InstalledApplication,
    PlatformCancellation, SystemInventory,
};

const TOOL_NAMES: &[&str] = &[
    "cargo",
    "conda",
    "docker",
    "go",
    "java",
    "node",
    "npm",
    "pnpm",
    "python3",
    "rustc",
    "rustup",
    "swift",
    "xcrun",
    "xcodebuild",
];
const MAX_APPLICATION_DIRECTORY_DEPTH: usize = 2;
const SPOTLIGHT_METADATA_TIMEOUT: Duration = Duration::from_secs(5);
const SPOTLIGHT_METADATA_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const INVENTORY_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const INVENTORY_COMMAND_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Default)]
struct ApplicationInventoryDiagnostics {
    unreadable_directory_count: u64,
    unreadable_entry_count: u64,
    unreadable_bundle_count: u64,
    incomplete_component_bundle_count: u64,
}

pub(super) fn system_inventory(
    cancellation: &PlatformCancellation,
) -> Result<SystemInventory, String> {
    let home = super::directories::home_directory().map_err(|error| error.to_string())?;
    let mut application_roots =
        super::directories::application_installation_directories(&home).to_vec();
    application_roots.sort();
    application_roots.dedup();

    let mut installed_applications = Vec::new();
    let mut seen_bundles = HashSet::new();
    let mut installed_applications_complete = true;
    let mut diagnostics = ApplicationInventoryDiagnostics::default();
    for root in application_roots {
        if cancellation.is_cancelled() {
            return Err("macos_application_inventory_cancelled".to_string());
        }
        if root.exists() {
            installed_applications_complete &= discover_applications(
                &root,
                0,
                &mut seen_bundles,
                &mut installed_applications,
                &mut diagnostics,
                cancellation,
            );
        }
    }
    if cancellation.is_cancelled() {
        return Err("macos_application_inventory_cancelled".to_string());
    }
    if !installed_applications_complete {
        log::info!(
            "application_inventory_partial application_count={} unreadable_directory_count={} unreadable_entry_count={} unreadable_bundle_count={} incomplete_component_bundle_count={}",
            installed_applications.len(),
            diagnostics.unreadable_directory_count,
            diagnostics.unreadable_entry_count,
            diagnostics.unreadable_bundle_count,
            diagnostics.incomplete_component_bundle_count,
        );
    }
    enrich_spotlight_metadata(&mut installed_applications, cancellation);
    if cancellation.is_cancelled() {
        return Err("macos_application_inventory_cancelled".to_string());
    }
    installed_applications.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });

    let filesystem = command_text("/usr/bin/stat", &["-f", "%T", "/"], cancellation)
        .map(|value| normalize_fact(&value))
        .filter(|value| !value.is_empty());
    let filesystem_complete = filesystem.is_some();
    let mut capabilities = Vec::new();
    if Path::new("/usr/bin/mdfind").is_file() && Path::new("/usr/bin/mdutil").is_file() {
        capabilities.push("spotlight".to_string());
    }
    if Path::new("/usr/bin/tmutil").is_file() {
        capabilities.push("time-machine".to_string());
    }

    let (developer_tools, developer_tools_complete) = detect_tools(TOOL_NAMES);
    Ok(SystemInventory {
        installed_applications,
        installed_applications_complete,
        developer_tools,
        developer_tools_complete,
        filesystem_kinds: filesystem.into_iter().collect(),
        filesystem_complete,
        capabilities,
        capabilities_complete: true,
        os_version: command_text("/usr/bin/sw_vers", &["-productVersion"], cancellation)
            .unwrap_or_else(|| "unknown".to_string()),
    })
}

pub(super) fn system_inventory_revision() -> Result<String, String> {
    system_inventory_revision_with_cancellation(&PlatformCancellation::new(|| false))
}

pub(super) fn system_inventory_revision_with_cancellation(
    cancellation: &PlatformCancellation,
) -> Result<String, String> {
    let home = super::directories::home_directory().map_err(|error| error.to_string())?;
    let mut revisions = Vec::new();
    for root in super::directories::application_installation_directories(&home) {
        if cancellation.is_cancelled() {
            return Err("macos_application_inventory_revision_cancelled".to_string());
        }
        if !root.exists() {
            revisions.push(format!("{}:missing", root.display()));
            continue;
        }
        collect_directory_revisions(&root, 0, &mut revisions, cancellation);
    }
    if cancellation.is_cancelled() {
        return Err("macos_application_inventory_revision_cancelled".to_string());
    }
    Ok(blake3::hash(revisions.join("|").as_bytes())
        .to_hex()
        .to_string())
}

pub(super) fn running_process_names(
    cancellation: &PlatformCancellation,
) -> Result<Vec<String>, String> {
    let executable = ControlledExecutable::capture(Path::new("/bin/ps")).map_err(|error| {
        format!(
            "macos_process_snapshot_executable_invalid reason={}",
            error.as_str()
        )
    })?;
    let output = run_controlled_command(
        "macos-process-snapshot",
        &executable,
        &["-axo", "comm="],
        ControlledEnvironmentPolicy::Inherit,
        ControlledCommandLimits {
            timeout: INVENTORY_COMMAND_TIMEOUT,
            stdout_bytes: INVENTORY_COMMAND_OUTPUT_LIMIT,
            stderr_bytes: 64 * 1024,
        },
        &|| cancellation.is_cancelled(),
    )
    .map_err(|error| format!("macos_process_snapshot_failed reason={}", error.as_str()))?;
    if !output.status.success() {
        return Err("the macOS process snapshot command failed".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn discover_applications(
    directory: &Path,
    depth: usize,
    seen_bundles: &mut HashSet<PathBuf>,
    applications: &mut Vec<InstalledApplication>,
    diagnostics: &mut ApplicationInventoryDiagnostics,
    cancellation: &PlatformCancellation,
) -> bool {
    if cancellation.is_cancelled() || depth > MAX_APPLICATION_DIRECTORY_DEPTH {
        return true;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        diagnostics.unreadable_directory_count =
            diagnostics.unreadable_directory_count.saturating_add(1);
        return false;
    };
    let mut paths = Vec::new();
    let mut inventory_complete = true;
    for entry in entries {
        if cancellation.is_cancelled() {
            return false;
        }
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(_) => {
                diagnostics.unreadable_entry_count =
                    diagnostics.unreadable_entry_count.saturating_add(1);
                inventory_complete = false;
            }
        }
    }
    // Directory iteration order is undefined. A stable order makes catalog
    // diagnostics and regression tests reproducible across APFS versions.
    paths.sort();
    for path in paths {
        if cancellation.is_cancelled() {
            return false;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            diagnostics.unreadable_entry_count =
                diagnostics.unreadable_entry_count.saturating_add(1);
            inventory_complete = false;
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        {
            if seen_bundles.insert(path.clone()) {
                let Some((application, complete)) =
                    read_application_with_cancellation(&path, cancellation)
                else {
                    // A directory ending in `.app` without readable identity
                    // metadata proves that the catalog is incomplete, but it
                    // must not hide unrelated applications that follow it.
                    // Core still receives the incomplete flag and fails closed
                    // for orphan classification.
                    log::debug!(
                        "application_inventory_bundle_unreadable bundle={}",
                        path.file_name()
                            .map(|value| value.to_string_lossy())
                            .unwrap_or_default()
                    );
                    diagnostics.unreadable_bundle_count =
                        diagnostics.unreadable_bundle_count.saturating_add(1);
                    inventory_complete = false;
                    continue;
                };
                if complete {
                    applications.push(application);
                } else {
                    // Missing nested component identities can hide running
                    // helpers from process matching. Exclude only this bundle
                    // so unrelated complete applications remain actionable.
                    log::debug!(
                        "application_inventory_component_incomplete bundle={}",
                        path.file_name()
                            .map(|value| value.to_string_lossy())
                            .unwrap_or_default()
                    );
                    diagnostics.incomplete_component_bundle_count = diagnostics
                        .incomplete_component_bundle_count
                        .saturating_add(1);
                    inventory_complete = false;
                }
            }
        } else if depth < MAX_APPLICATION_DIRECTORY_DEPTH {
            inventory_complete &= discover_applications(
                &path,
                depth + 1,
                seen_bundles,
                applications,
                diagnostics,
                cancellation,
            );
        }
    }
    inventory_complete
}

fn collect_directory_revisions(
    directory: &Path,
    depth: usize,
    revisions: &mut Vec<String>,
    cancellation: &PlatformCancellation,
) {
    if cancellation.is_cancelled() {
        return;
    }
    let Ok(metadata) = fs::metadata(directory) else {
        revisions.push(format!("{}:unreadable", directory.display()));
        return;
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    revisions.push(format!("{}:{modified}", directory.display()));
    if depth >= MAX_APPLICATION_DIRECTORY_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        revisions.push(format!("{}:children-unreadable", directory.display()));
        return;
    };
    let mut paths = Vec::new();
    for entry in entries {
        if cancellation.is_cancelled() {
            return;
        }
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(_) => revisions.push(format!("{}:entry-unreadable", directory.display())),
        }
    }
    // Directory iteration order is undefined. Sorting keeps the revision
    // stable when application contents have not changed.
    paths.sort();
    for path in paths {
        if cancellation.is_cancelled() {
            return;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            revisions.push(format!("{}:unreadable", path.display()));
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        {
            collect_path_revision(&path, revisions);
            collect_path_revision(&path.join("Contents/Info.plist"), revisions);
            collect_path_revision(&path.join("WrappedBundle/Info.plist"), revisions);
            collect_nested_component_revisions(&path, revisions, cancellation);
            continue;
        }
        if metadata.is_dir() {
            collect_directory_revisions(&path, depth + 1, revisions, cancellation);
        }
    }
}

fn collect_path_revision(path: &Path, revisions: &mut Vec<String>) {
    let Ok(metadata) = fs::metadata(path) else {
        revisions.push(format!("{}:unreadable", path.display()));
        return;
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    revisions.push(format!("{}:{modified}:{}", path.display(), metadata.len()));
}

fn collect_nested_component_revisions(
    bundle: &Path,
    revisions: &mut Vec<String>,
    cancellation: &PlatformCancellation,
) {
    if cancellation.is_cancelled() {
        return;
    }
    let Ok(canonical_bundle) = fs::canonicalize(bundle) else {
        revisions.push(format!("{}:canonical-unreadable", bundle.display()));
        return;
    };
    for relative in COMPONENT_ROOTS {
        if cancellation.is_cancelled() {
            return;
        }
        let root = bundle.join(relative);
        match validated_component_root(&root, &canonical_bundle) {
            Ok(Some(root)) => {
                collect_path_revision(&root, revisions);
                collect_component_revisions(&root, &canonical_bundle, 0, revisions, cancellation);
            }
            Ok(None) => {}
            Err(reason) => revisions.push(format!("{}:{reason}", root.display())),
        }
    }
}

fn collect_component_revisions(
    directory: &Path,
    canonical_bundle: &Path,
    depth: usize,
    revisions: &mut Vec<String>,
    cancellation: &PlatformCancellation,
) {
    if cancellation.is_cancelled() || depth > COMPONENT_SEARCH_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        revisions.push(format!("{}:children-unreadable", directory.display()));
        return;
    };
    let mut paths = Vec::new();
    for entry in entries {
        if cancellation.is_cancelled() {
            return;
        }
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(_) => revisions.push(format!("{}:entry-unreadable", directory.display())),
        }
    }
    paths.sort();
    for path in paths {
        if cancellation.is_cancelled() {
            return;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            revisions.push(format!("{}:unreadable", path.display()));
            continue;
        };
        if metadata.file_type().is_symlink() {
            if is_application_component(&path) {
                collect_symlink_revision(&path, revisions);
                match fs::canonicalize(&path) {
                    Ok(target) if target.starts_with(canonical_bundle) => {
                        collect_path_revision(&target.join("Contents/Info.plist"), revisions);
                        collect_path_revision(&target.join("Info.plist"), revisions);
                    }
                    _ => revisions.push(format!("{}:external-link", path.display())),
                }
            }
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        if is_application_component(&path) {
            collect_path_revision(&path, revisions);
            collect_path_revision(&path.join("Contents/Info.plist"), revisions);
            collect_path_revision(&path.join("Info.plist"), revisions);
        }
        if depth < COMPONENT_SEARCH_DEPTH {
            collect_component_revisions(
                &path,
                canonical_bundle,
                depth + 1,
                revisions,
                cancellation,
            );
        }
    }
}

fn collect_symlink_revision(path: &Path, revisions: &mut Vec<String>) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        revisions.push(format!("{}:link-unreadable", path.display()));
        return;
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let target = fs::read_link(path)
        .map(|target| target.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "target-unreadable".to_string());
    revisions.push(format!(
        "{}:{modified}:{}:{target}",
        path.display(),
        metadata.len()
    ));
}

fn validated_component_root(
    root: &Path,
    canonical_bundle: &Path,
) -> Result<Option<PathBuf>, &'static str> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("root-unreadable"),
    };
    if metadata.file_type().is_symlink() {
        let target = fs::canonicalize(root).map_err(|_| "root-link-unreadable")?;
        if !target.starts_with(canonical_bundle) {
            return Err("root-link-outside-bundle");
        }
        return target
            .is_dir()
            .then_some(Some(target))
            .ok_or("root-link-not-directory");
    }
    metadata
        .is_dir()
        .then_some(Some(root.to_path_buf()))
        .ok_or("root-not-directory")
}

#[cfg(test)]
fn read_application(bundle: &Path) -> Option<(InstalledApplication, bool)> {
    read_application_with_cancellation(bundle, &PlatformCancellation::new(|| false))
}

fn read_application_with_cancellation(
    bundle: &Path,
    cancellation: &PlatformCancellation,
) -> Option<(InstalledApplication, bool)> {
    let normal_info = bundle.join("Contents/Info.plist");
    let (metadata_bundle, info_path, executable_directory) = if normal_info.is_file() {
        (
            bundle.to_path_buf(),
            normal_info,
            bundle.join("Contents/MacOS"),
        )
    } else {
        // Some iOS applications installed from the App Store use a top-level
        // wrapper and a WrappedBundle link. Follow it only when the canonical
        // target remains inside the application bundle.
        let canonical_bundle = fs::canonicalize(bundle).ok()?;
        let wrapped_bundle = fs::canonicalize(bundle.join("WrappedBundle")).ok()?;
        if !wrapped_bundle.starts_with(&canonical_bundle)
            || !wrapped_bundle
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        {
            return None;
        }
        (
            wrapped_bundle.clone(),
            wrapped_bundle.join("Info.plist"),
            wrapped_bundle.to_path_buf(),
        )
    };
    let dictionary = Value::from_file(info_path).ok()?.into_dictionary()?;
    let string = |key: &str| {
        dictionary
            .get(key)
            .and_then(Value::as_string)
            .map(str::to_string)
    };
    let name = string("CFBundleDisplayName")
        .or_else(|| string("CFBundleName"))
        .or_else(|| {
            bundle
                .file_stem()
                .map(|value| value.to_string_lossy().into_owned())
        })?;
    let primary_identifier = string("CFBundleIdentifier").unwrap_or_else(|| name.clone());
    let mut identifiers = vec![name.clone(), primary_identifier.clone()];
    let mut executable_paths = string("CFBundleExecutable")
        .map(|name| vec![executable_directory.join(name)])
        .unwrap_or_default();
    let component_inventory_complete = collect_nested_component_identities(
        bundle,
        &mut identifiers,
        &mut executable_paths,
        cancellation,
    );
    identifiers.sort_by_key(|value| value.to_ascii_lowercase());
    identifiers.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    executable_paths.sort();
    executable_paths.dedup();
    Some((
        InstalledApplication {
            uninstall_diagnostic: None,
            catalog_identifier: format!("macos-bundle:{}", bundle.to_string_lossy()),
            source_identities: vec![ApplicationSourceIdentity {
                source: ApplicationInventorySource::MacosBundle,
                identifier: primary_identifier.clone(),
            }],
            primary_identifier,
            identifiers,
            name,
            version: string("CFBundleShortVersionString").or_else(|| string("CFBundleVersion")),
            publisher: None,
            estimated_bytes: 0,
            last_used_at_ms: None,
            installed_at_ms: None,
            // Icon extraction needs the bundle that owns Info.plist and its
            // resources. The uninstall path intentionally remains the outer
            // application wrapper.
            icon_path: Some(metadata_bundle),
            bundle_path: Some(bundle.to_path_buf()),
            executable_paths,
            uninstall_registration: None,
        },
        component_inventory_complete,
    ))
}

/// Enriches the lightweight catalog without recursively walking every bundle.
///
/// `mdls` accepts all discovered paths in one process and writes the requested
/// attributes in argument order. Missing or stale Spotlight values remain
/// optional catalog facts; they never make the application inventory
/// incomplete because uninstall safety does not depend on them.
fn enrich_spotlight_metadata(
    applications: &mut [InstalledApplication],
    cancellation: &PlatformCancellation,
) {
    let indexed = applications
        .iter()
        .enumerate()
        .filter_map(|(index, application)| {
            let path = application.bundle_path.as_deref()?;
            path.to_str().map(|path| (index, path.to_string()))
        })
        .collect::<Vec<_>>();
    if indexed.is_empty() {
        return;
    }

    let Ok(executable) = ControlledExecutable::capture(Path::new("/usr/bin/mdls")) else {
        log::debug!(
            "application_inventory_spotlight_metadata_unavailable reason=invalid_executable"
        );
        return;
    };
    let mut arguments = vec![
        "-name".to_string(),
        "kMDItemFSSize".to_string(),
        "-name".to_string(),
        "kMDItemLastUsedDate".to_string(),
    ];
    arguments.extend(indexed.iter().map(|(_, path)| path.clone()));
    let argument_refs = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    let output = run_controlled_command(
        "application_inventory_spotlight_metadata",
        &executable,
        &argument_refs,
        ControlledEnvironmentPolicy::Inherit,
        ControlledCommandLimits {
            timeout: SPOTLIGHT_METADATA_TIMEOUT,
            stdout_bytes: SPOTLIGHT_METADATA_OUTPUT_LIMIT,
            stderr_bytes: 16 * 1024,
        },
        &|| cancellation.is_cancelled(),
    );
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            log::debug!(
                "application_inventory_spotlight_metadata_unavailable reason={}",
                error.as_str()
            );
            return;
        }
    };
    if !output.status.success() {
        log::debug!(
            "application_inventory_spotlight_metadata_unavailable reason=nonzero_exit exit_code={:?}",
            output.status.code()
        );
        return;
    }

    let lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if lines.len() != indexed.len().saturating_mul(2) {
        log::debug!(
            "application_inventory_spotlight_metadata_unavailable reason=unexpected_output expected_lines={} actual_lines={}",
            indexed.len().saturating_mul(2),
            lines.len()
        );
        return;
    }

    for ((application_index, _), values) in indexed.into_iter().zip(lines.chunks_exact(2)) {
        let application = &mut applications[application_index];
        let spotlight_bytes = metadata_value(&values[0], "kMDItemFSSize")
            .and_then(|value| value.parse().ok())
            .unwrap_or_default();
        application.estimated_bytes = validated_spotlight_size(application, spotlight_bytes);
        application.last_used_at_ms =
            metadata_value(&values[1], "kMDItemLastUsedDate").and_then(parse_spotlight_date_ms);
    }
}

/// Rejects impossible Spotlight bundle sizes without introducing an arbitrary
/// byte threshold.
///
/// macOS may publish `kMDItemFSSize = 1` for otherwise valid application
/// bundles. The declared main executable is already known from Info.plist, so
/// a bundle estimate smaller than that file cannot represent the application.
/// Returning zero activates Core's exact, symlink-safe bundle measurement.
fn validated_spotlight_size(application: &InstalledApplication, spotlight_bytes: u64) -> u64 {
    if spotlight_bytes == 0 {
        return 0;
    }
    let executable_lower_bound =
        application
            .executable_paths
            .iter()
            .fold(0_u64, |total, executable| {
                total.saturating_add(
                    fs::metadata(executable)
                        .ok()
                        .filter(|metadata| metadata.is_file())
                        .map(|metadata| metadata.len())
                        .unwrap_or_default(),
                )
            });
    if executable_lower_bound > 0 && spotlight_bytes < executable_lower_bound {
        log::debug!(
            "application_inventory_spotlight_size_rejected bundle={} spotlight_bytes={} executable_lower_bound={}",
            application
                .bundle_path
                .as_deref()
                .and_then(Path::file_name)
                .map(|value| value.to_string_lossy())
                .unwrap_or_default(),
            spotlight_bytes,
            executable_lower_bound
        );
        return 0;
    }
    spotlight_bytes
}

fn metadata_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (actual_key, value) = line.split_once(" = ")?;
    (actual_key.trim() == key)
        .then_some(value.trim())
        .filter(|value| *value != "(null)")
}

fn parse_spotlight_date_ms(value: &str) -> Option<u64> {
    let mut parts = value.split_ascii_whitespace();
    let date = parts.next()?;
    let time = parts.next()?;
    let offset = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let mut date_parts = date.split('-').map(str::parse::<i64>);
    let year = date_parts.next()?.ok()?;
    let month = date_parts.next()?.ok()?;
    let day = date_parts.next()?.ok()?;
    if date_parts.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
    {
        return None;
    }

    let mut time_parts = time.split(':').map(str::parse::<i64>);
    let hour = time_parts.next()?.ok()?;
    let minute = time_parts.next()?.ok()?;
    let second = time_parts.next()?.ok()?;
    if time_parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let offset_bytes = offset.as_bytes();
    if offset_bytes.len() != 5 || !matches!(offset_bytes[0], b'+' | b'-') {
        return None;
    }
    let offset_hours = offset[1..3].parse::<i64>().ok()?;
    let offset_minutes = offset[3..5].parse::<i64>().ok()?;
    if offset_hours > 23 || offset_minutes > 59 {
        return None;
    }
    let offset_sign = if offset_bytes[0] == b'-' { -1 } else { 1 };
    let offset_seconds = offset_sign * (offset_hours * 3_600 + offset_minutes * 60);
    let seconds = days_from_civil(year, month, day)
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)?
        .checked_sub(offset_seconds)?;
    u64::try_from(seconds).ok()?.checked_mul(1_000)
}

/// Converts a Gregorian date to days since 1970-01-01.
///
/// This is the civil-date algorithm by Howard Hinnant, kept local so the
/// platform catalog does not add a date dependency for one optional metadata
/// field.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

const COMPONENT_SEARCH_DEPTH: usize = 4;
const COMPONENT_ROOTS: &[&str] = &[
    "Contents/Frameworks",
    "Contents/Library/LoginItems",
    "Contents/PlugIns",
    "Contents/XPCServices",
];

/// Collects identities that may own a user container independently of the
/// main application. This is intentionally limited to standard component
/// roots so catalog creation does not traverse application resources.
fn collect_nested_component_identities(
    bundle: &Path,
    identifiers: &mut Vec<String>,
    executable_paths: &mut Vec<PathBuf>,
    cancellation: &PlatformCancellation,
) -> bool {
    let Ok(canonical_bundle) = fs::canonicalize(bundle) else {
        return false;
    };
    let mut complete = true;
    for relative in COMPONENT_ROOTS {
        if cancellation.is_cancelled() {
            return false;
        }
        let root = bundle.join(relative);
        match validated_component_root(&root, &canonical_bundle) {
            Ok(Some(root)) => {
                if !collect_component_directory(
                    &root,
                    &canonical_bundle,
                    0,
                    identifiers,
                    executable_paths,
                    cancellation,
                ) {
                    complete = false;
                }
            }
            Ok(None) => {}
            Err(_) => complete = false,
        }
    }
    complete
}

fn collect_component_directory(
    directory: &Path,
    canonical_bundle: &Path,
    depth: usize,
    identifiers: &mut Vec<String>,
    executable_paths: &mut Vec<PathBuf>,
    cancellation: &PlatformCancellation,
) -> bool {
    if cancellation.is_cancelled() {
        return false;
    }
    if depth > COMPONENT_SEARCH_DEPTH {
        return true;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return false;
    };
    let mut complete = true;
    for entry in entries {
        if cancellation.is_cancelled() {
            return false;
        }
        let Ok(entry) = entry else {
            complete = false;
            continue;
        };
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            complete = false;
            continue;
        };
        if metadata.file_type().is_symlink() {
            if is_application_component(&path) {
                let target = fs::canonicalize(&path).ok();
                let component = target
                    .as_deref()
                    .filter(|target| target.starts_with(canonical_bundle))
                    .and_then(read_component_identity);
                if let Some((identifier, executable)) = component {
                    identifiers.push(identifier);
                    if let Some(executable) = executable {
                        executable_paths.push(executable);
                    }
                } else {
                    complete = false;
                }
            }
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        if is_application_component(&path) {
            if let Some((identifier, executable)) = read_component_identity(&path) {
                identifiers.push(identifier);
                if let Some(executable) = executable {
                    executable_paths.push(executable);
                }
            } else {
                log::debug!(
                    "application_inventory_component_unreadable component={}",
                    path.file_name()
                        .map(|value| value.to_string_lossy())
                        .unwrap_or_default()
                );
                complete = false;
            }
        }
        if depth < COMPONENT_SEARCH_DEPTH
            && !collect_component_directory(
                &path,
                canonical_bundle,
                depth + 1,
                identifiers,
                executable_paths,
                cancellation,
            )
        {
            complete = false;
        }
    }
    complete
}

fn is_application_component(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("app")
            || extension.eq_ignore_ascii_case("appex")
            || extension.eq_ignore_ascii_case("xpc")
    })
}

fn read_component_identity(path: &Path) -> Option<(String, Option<PathBuf>)> {
    let contents_info = path.join("Contents/Info.plist");
    let (info_path, executable_root) = if contents_info.is_file() {
        (contents_info, path.join("Contents/MacOS"))
    } else {
        (path.join("Info.plist"), path.to_path_buf())
    };
    let dictionary = Value::from_file(info_path).ok()?.into_dictionary()?;
    let identifier = dictionary
        .get("CFBundleIdentifier")
        .and_then(Value::as_string)?
        .to_string();
    let executable = dictionary
        .get("CFBundleExecutable")
        .and_then(Value::as_string)
        .map(|name| executable_root.join(name));
    Some((identifier, executable))
}

fn command_text(
    program: &str,
    arguments: &[&str],
    cancellation: &PlatformCancellation,
) -> Option<String> {
    let executable = ControlledExecutable::capture(Path::new(program)).ok()?;
    run_controlled_command(
        "macos-application-inventory-command",
        &executable,
        arguments,
        ControlledEnvironmentPolicy::Inherit,
        ControlledCommandLimits {
            timeout: INVENTORY_COMMAND_TIMEOUT,
            stdout_bytes: 64 * 1024,
            stderr_bytes: 16 * 1024,
        },
        &|| cancellation.is_cancelled(),
    )
    .ok()
    .filter(|output| output.status.success())
    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::Dictionary;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mangodisk-inventory-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    fn write_bundle_info(bundle: &Path, identifier: &str, executable: &str) {
        let contents = bundle.join("Contents");
        fs::create_dir_all(contents.join("MacOS"))
            .expect("the application bundle fixture should be created");
        let mut dictionary = Dictionary::new();
        dictionary.insert(
            "CFBundleIdentifier".to_string(),
            Value::String(identifier.to_string()),
        );
        dictionary.insert(
            "CFBundleExecutable".to_string(),
            Value::String(executable.to_string()),
        );
        dictionary.insert(
            "CFBundleName".to_string(),
            Value::String(
                bundle
                    .file_stem()
                    .expect("the fixture bundle should have a name")
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
        Value::Dictionary(dictionary)
            .to_file_xml(contents.join("Info.plist"))
            .expect("the application identity fixture should be written");
    }

    #[test]
    fn cancelled_process_snapshot_does_not_start_the_command() {
        let cancellation = PlatformCancellation::new(|| true);

        let error = running_process_names(&cancellation)
            .expect_err("a pre-cancelled process snapshot must stop");

        assert!(error.contains("cancelled"));
    }

    #[test]
    fn cancelled_inventory_revision_stops_before_traversal() {
        let cancellation = PlatformCancellation::new(|| true);

        let error = system_inventory_revision_with_cancellation(&cancellation)
            .expect_err("a pre-cancelled revision capture must stop");

        assert!(error.contains("cancelled"));
    }

    fn write_flat_bundle_info(bundle: &Path, identifier: &str, executable: &str) {
        fs::create_dir_all(bundle).expect("the flat application bundle fixture should be created");
        let mut dictionary = Dictionary::new();
        dictionary.insert(
            "CFBundleIdentifier".to_string(),
            Value::String(identifier.to_string()),
        );
        dictionary.insert(
            "CFBundleExecutable".to_string(),
            Value::String(executable.to_string()),
        );
        dictionary.insert(
            "CFBundleName".to_string(),
            Value::String("Wrapped Example".to_string()),
        );
        Value::Dictionary(dictionary)
            .to_file_xml(bundle.join("Info.plist"))
            .expect("the flat application identity fixture should be written");
        fs::write(bundle.join(executable), b"wrapped executable")
            .expect("the wrapped executable fixture should be written");
    }

    #[test]
    fn unreadable_application_identity_does_not_hide_later_bundles() {
        let root = fixture_path("unreadable-application");
        fs::create_dir_all(root.join("A Broken.app"))
            .expect("the invalid application fixture should be created");
        write_bundle_info(
            &root.join("Z Visible.app"),
            "com.example.visible",
            "Visible",
        );
        let mut applications = Vec::new();
        let mut seen_bundles = HashSet::new();
        let mut diagnostics = ApplicationInventoryDiagnostics::default();

        let complete = discover_applications(
            &root,
            0,
            &mut seen_bundles,
            &mut applications,
            &mut diagnostics,
            &PlatformCancellation::new(|| false),
        );

        assert!(!complete);
        assert!(applications
            .iter()
            .any(|application| application.primary_identifier == "com.example.visible"));
        assert_eq!(diagnostics.unreadable_bundle_count, 1);
        fs::remove_dir_all(root).expect("the application fixtures should be removed");
    }

    #[test]
    fn incomplete_application_components_exclude_only_the_affected_bundle() {
        let root = fixture_path("incomplete-application-component");
        let incomplete_bundle = root.join("A Incomplete.app");
        let broken_helper = incomplete_bundle.join("Contents/Frameworks/Broken Helper.app");
        write_bundle_info(&incomplete_bundle, "com.example.incomplete", "Incomplete");
        fs::create_dir_all(&broken_helper)
            .expect("the incomplete component fixture should be created");
        write_bundle_info(
            &root.join("Z Visible.app"),
            "com.example.visible",
            "Visible",
        );
        let mut applications = Vec::new();
        let mut seen_bundles = HashSet::new();
        let mut diagnostics = ApplicationInventoryDiagnostics::default();

        let complete = discover_applications(
            &root,
            0,
            &mut seen_bundles,
            &mut applications,
            &mut diagnostics,
            &PlatformCancellation::new(|| false),
        );

        assert!(!complete);
        assert_eq!(applications.len(), 1);
        assert_eq!(applications[0].primary_identifier, "com.example.visible");
        assert_eq!(diagnostics.incomplete_component_bundle_count, 1);
        fs::remove_dir_all(root).expect("the application fixtures should be removed");
    }

    #[test]
    fn nested_component_identifiers_are_included_in_the_application_inventory() {
        let root = fixture_path("nested-component");
        let bundle = root.join("Example.app");
        let helper = bundle.join("Contents/Frameworks/Example Helper.app");
        write_bundle_info(&bundle, "com.example.main", "Example");
        write_bundle_info(&helper, "com.example.helper", "Example Helper");

        let (application, complete) =
            read_application(&bundle).expect("the application fixture should be readable");

        assert!(complete);
        assert!(application
            .identifiers
            .iter()
            .any(|identifier| identifier == "com.example.main"));
        assert!(application
            .identifiers
            .iter()
            .any(|identifier| identifier == "com.example.helper"));
        fs::remove_dir_all(root).expect("the application fixture should be removed");
    }

    #[test]
    fn internal_component_links_are_tracked_by_the_inventory_revision() {
        let root = fixture_path("linked-component");
        let bundle = root.join("Example.app");
        let helper = bundle.join("Contents/Internal/Linked Helper.app");
        let linked_helper = bundle.join("Contents/Frameworks/Linked Helper.app");
        write_bundle_info(&bundle, "com.example.main", "Example");
        write_bundle_info(&helper, "com.example.linked-helper", "Linked Helper");
        fs::create_dir_all(
            linked_helper
                .parent()
                .expect("the link should have a parent"),
        )
        .expect("the component root should be created");
        std::os::unix::fs::symlink(&helper, &linked_helper)
            .expect("the internal component link should be created");

        let mut before = Vec::new();
        let cancellation = PlatformCancellation::new(|| false);
        collect_nested_component_revisions(&bundle, &mut before, &cancellation);
        write_bundle_info(
            &helper,
            "com.example.linked-helper.changed",
            "Linked Helper",
        );
        let mut after = Vec::new();
        collect_nested_component_revisions(&bundle, &mut after, &cancellation);
        let (application, complete) =
            read_application(&bundle).expect("the application fixture should be readable");

        assert!(complete);
        assert_ne!(before, after);
        assert!(application
            .identifiers
            .iter()
            .any(|identifier| identifier == "com.example.linked-helper.changed"));
        fs::remove_dir_all(root).expect("the application fixture should be removed");
    }

    #[test]
    fn component_root_link_outside_the_bundle_makes_inventory_incomplete() {
        let root = fixture_path("external-component-root");
        let bundle = root.join("Example.app");
        let external = root.join("ExternalFrameworks");
        write_bundle_info(&bundle, "com.example.main", "Example");
        fs::create_dir_all(&external).expect("the external component root should be created");
        std::os::unix::fs::symlink(&external, bundle.join("Contents/Frameworks"))
            .expect("the external component root link should be created");

        let (_, complete) =
            read_application(&bundle).expect("the main application identity should be readable");

        assert!(!complete);
        fs::remove_dir_all(root).expect("the application fixture should be removed");
    }

    #[test]
    fn wrapped_application_uses_inner_bundle_for_icon_resources() {
        let root = fixture_path("wrapped-application");
        let bundle = root.join("Example.app");
        let wrapped_bundle = bundle.join("Wrapper/Example.app");
        write_flat_bundle_info(&wrapped_bundle, "com.example.wrapped", "Example");
        std::os::unix::fs::symlink("Wrapper/Example.app", bundle.join("WrappedBundle"))
            .expect("the wrapped bundle link should be created");

        let (application, complete) =
            read_application(&bundle).expect("the wrapped application should be readable");

        assert!(complete);
        assert_eq!(application.bundle_path.as_deref(), Some(bundle.as_path()));
        assert_eq!(
            application.icon_path,
            Some(fs::canonicalize(&wrapped_bundle).expect("wrapped bundle should canonicalize"))
        );
        fs::remove_dir_all(root).expect("the application fixture should be removed");
    }

    #[test]
    fn impossible_spotlight_size_activates_exact_bundle_measurement() {
        let root = fixture_path("spotlight-size");
        let executable = root.join("Example");
        fs::create_dir_all(&root).expect("the Spotlight fixture should be created");
        fs::write(&executable, b"application executable")
            .expect("the executable fixture should be written");
        let mut application = InstalledApplication {
            uninstall_diagnostic: None,
            catalog_identifier: format!("macos-bundle:{}", root.to_string_lossy()),
            source_identities: vec![ApplicationSourceIdentity {
                source: ApplicationInventorySource::MacosBundle,
                identifier: "com.example.application".to_string(),
            }],
            primary_identifier: "com.example.application".to_string(),
            identifiers: vec!["com.example.application".to_string()],
            name: "Example".to_string(),
            version: None,
            publisher: None,
            estimated_bytes: 0,
            last_used_at_ms: None,
            installed_at_ms: None,
            icon_path: None,
            bundle_path: Some(root.clone()),
            executable_paths: vec![executable],
            uninstall_registration: None,
        };

        assert_eq!(validated_spotlight_size(&application, 1), 0);
        application.executable_paths.clear();
        assert_eq!(validated_spotlight_size(&application, 1), 1);
        fs::remove_dir_all(root).expect("the Spotlight fixture should be removed");
    }

    #[test]
    fn spotlight_metadata_values_are_parsed_without_ui_specific_formatting() {
        assert_eq!(
            metadata_value("kMDItemFSSize       = 34859542", "kMDItemFSSize"),
            Some("34859542")
        );
        assert_eq!(
            metadata_value("kMDItemLastUsedDate = (null)", "kMDItemLastUsedDate"),
            None
        );
    }

    #[test]
    fn spotlight_dates_are_converted_to_unix_milliseconds() {
        assert_eq!(
            parse_spotlight_date_ms("1970-01-01 00:00:00 +0000"),
            Some(0)
        );
        assert_eq!(
            parse_spotlight_date_ms("1970-01-01 08:00:00 +0800"),
            Some(0)
        );
        assert_eq!(
            parse_spotlight_date_ms("2024-02-29 00:00:00 +0000"),
            Some(1_709_164_800_000)
        );
        assert_eq!(parse_spotlight_date_ms("2023-02-29 00:00:00 +0000"), None);
    }
}
