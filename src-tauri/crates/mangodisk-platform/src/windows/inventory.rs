use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env,
    ffi::OsString,
    fs,
    os::windows::ffi::{OsStrExt, OsStringExt},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    os::windows::io::AsRawHandle,
    path::{Component, Path, PathBuf},
    ptr,
    time::{Duration, Instant},
};

use serde::Deserialize;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::{
    Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS},
    System::ApplicationInstallationAndServicing::{
        MsiGetProductInfoW, INSTALLPROPERTY_PRODUCTICON,
    },
};

use winreg::{
    enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY},
    RegKey,
};

use crate::{
    application_uninstall_diagnostic_id,
    command::{
        run_controlled_command, ControlledCommandLimits, ControlledEnvironmentPolicy,
        ControlledExecutable,
    },
    inventory::{detect_tools, normalize_fact},
    ApplicationInstallScope, ApplicationInventorySource, ApplicationSourceIdentity,
    ApplicationUninstallDiagnostic, ApplicationUninstallRegistration, InstalledApplication,
    PlatformCancellation, SystemInventory, WindowsRegistryView,
};

use super::{native_uninstall, package_reconciliation, package_sources, path_identity};

const UNINSTALL_PATH: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const APPX_INVENTORY_SCRIPT: &str = r#"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$ProgressPreference = 'SilentlyContinue'
$startApps = @{}
Get-StartApps -ErrorAction SilentlyContinue | ForEach-Object {
  $family = ([string]$_.AppID).Split('!')[0]
  if ($family -and -not $startApps.ContainsKey($family)) { $startApps[$family] = [string]$_.Name }
}
$packages = @(Get-AppxPackage -ErrorAction Stop)
$items = @(
  $packages |
    Where-Object { -not $_.IsFramework -and -not $_.IsResourcePackage -and -not $_.NonRemovable -and -not $_.IsPartiallyStaged } |
    ForEach-Object {
      $package = $_
      $manifest = Get-AppxPackageManifest -Package $package.PackageFullName -ErrorAction SilentlyContinue
      $displayName = $startApps[$package.PackageFamilyName]
      if (-not $displayName -or $displayName.StartsWith('ms-resource:')) { $displayName = [string]$package.Name }
      $publisher = [string]$manifest.Package.Properties.PublisherDisplayName
      if (-not $publisher -or $publisher.StartsWith('ms-resource:')) { $publisher = [string]$package.Publisher }
      $application = @($manifest.Package.Applications.Application)[0]
      $executable = if ($null -ne $application) { [string]$application.Executable } else { '' }
      $icon = if ($null -ne $application) {
        $visualElements = $application.VisualElements
        $appListLogo = [string]$visualElements.Square44x44Logo
        if ($appListLogo) { $appListLogo } else { [string]$visualElements.Square150x150Logo }
      } else { '' }
      [pscustomobject]@{
        systemSigned = ([string]$package.SignatureKind -eq 'System')
        packageFamilyName = [string]$package.PackageFamilyName
        packageFullName = [string]$package.PackageFullName
        name = $displayName
        version = [string]$package.Version
        publisher = $publisher
        installLocation = [string]$package.InstallLocation
        executable = $executable
        icon = $icon
      }
    }
)
$result = [pscustomobject]@{
  schemaVersion = 1
  items = $items
  packageCount = $packages.Count
  excludedCount = $packages.Count - $items.Count
  frameworkCount = @($packages | Where-Object { $_.IsFramework }).Count
  resourceCount = @($packages | Where-Object { $_.IsResourcePackage }).Count
  nonRemovableCount = @($packages | Where-Object { $_.NonRemovable }).Count
  partiallyStagedCount = @($packages | Where-Object { $_.IsPartiallyStaged }).Count
}
ConvertTo-Json -InputObject $result -Depth 4 -Compress
"#;
// Get-AppxPackage reads the same current-user package repository but starts PowerShell and walks
// every package. The repository key's child set changes whenever a package full name is added or
// removed, which is exactly the opaque revision fact needed to validate the in-process inventory
// cache. Full package metadata remains sourced from Get-AppxPackage only after this token changes.
const APPX_REPOSITORY_PATH: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
const APPX_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const APPX_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const PROCESS_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_SNAPSHOT_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const TOOL_NAMES: &[&str] = &[
    "cargo.exe",
    "conda.exe",
    "docker.exe",
    "dotnet.exe",
    "go.exe",
    "java.exe",
    "node.exe",
    "npm.cmd",
    "pnpm.cmd",
    "python.exe",
    "rustc.exe",
    "rustup.exe",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegistryScope {
    CurrentUser,
    Machine,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackagedApplicationRecord {
    system_signed: bool,
    package_family_name: String,
    package_full_name: String,
    name: String,
    version: String,
    publisher: String,
    install_location: String,
    executable: String,
    icon: String,
}

// The command and decoder ship together. Exclusion counts can overlap because one package
// may be both a framework and non-removable; excluded_count is the distinct package count.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackagedApplicationInventory {
    schema_version: u32,
    items: Vec<PackagedApplicationRecord>,
    package_count: usize,
    excluded_count: usize,
    framework_count: usize,
    resource_count: usize,
    non_removable_count: usize,
    partially_staged_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WindowsFileIdentity {
    volume_serial_number: u32,
    file_index: u64,
}

#[derive(Clone, Copy, Debug)]
struct FileLinkSummary {
    bytes: u64,
    observed_links: u32,
    total_links: u32,
}

fn file_link_evidence(path: &Path) -> Option<(WindowsFileIdentity, u32)> {
    let file = fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
        .ok()?;
    let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, information.as_mut_ptr()) };
    if succeeded == 0 {
        return None;
    }
    let information = unsafe { information.assume_init() };
    let file_index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Some((
        WindowsFileIdentity {
            volume_serial_number: information.dwVolumeSerialNumber,
            file_index,
        },
        information.nNumberOfLinks.max(1),
    ))
}

impl RegistryScope {
    const fn stable_code(self) -> &'static str {
        match self {
            Self::CurrentUser => "current-user",
            Self::Machine => "machine",
        }
    }

    const fn install_scope(self) -> ApplicationInstallScope {
        match self {
            Self::CurrentUser => ApplicationInstallScope::CurrentUser,
            Self::Machine => ApplicationInstallScope::Machine,
        }
    }
}

fn registry_view(view: u32) -> WindowsRegistryView {
    if view == KEY_WOW64_32KEY {
        WindowsRegistryView::Registry32
    } else {
        WindowsRegistryView::Registry64
    }
}

pub(super) fn system_inventory(
    cancellation: &PlatformCancellation,
) -> Result<SystemInventory, String> {
    // WinGet export and Chocolatey enumeration may need process startup and
    // source-catalog access. Run package-source discovery alongside registry
    // and AppX inventory so broader identity coverage does not add its full
    // latency to every cold application scan.
    let inventory_started = Instant::now();
    let package_source_started = Instant::now();
    let package_source_cancellation = cancellation.clone();
    let mut package_source_worker = Some(std::thread::spawn(move || {
        package_sources::inventory(package_source_cancellation)
    }));
    let mut applications = HashMap::<String, InstalledApplication>::new();
    let mut conflicting_registrations = HashSet::<String>::new();
    let mut opened_views = 0_usize;
    let mut complete_views = 0_usize;
    let registry_started = Instant::now();
    for (root, view, scope) in [
        (
            RegKey::predef(HKEY_CURRENT_USER),
            KEY_WOW64_64KEY,
            RegistryScope::CurrentUser,
        ),
        (
            RegKey::predef(HKEY_CURRENT_USER),
            KEY_WOW64_32KEY,
            RegistryScope::CurrentUser,
        ),
        (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            KEY_WOW64_64KEY,
            RegistryScope::Machine,
        ),
        (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            KEY_WOW64_32KEY,
            RegistryScope::Machine,
        ),
    ] {
        if cancellation.is_cancelled() {
            if let Some(worker) = package_source_worker.take() {
                let _ = worker.join();
            }
            return Err("windows_application_inventory_cancelled".to_string());
        }
        let (opened, complete) = read_uninstall_view(
            &root,
            view,
            scope,
            &mut applications,
            &mut conflicting_registrations,
        );
        opened_views += usize::from(opened);
        complete_views += usize::from(complete);
    }
    let registry_elapsed_ms = registry_started.elapsed().as_millis();
    let appx_started = Instant::now();
    let mut appx_count = 0_usize;
    let appx_complete = match read_packaged_applications(cancellation) {
        Ok(packages) => {
            appx_count = packages.len();
            for package in packages {
                merge_packaged_application(&mut applications, package);
            }
            true
        }
        Err(error) => {
            log::warn!(
                "windows_packaged_application_inventory_failed error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
            false
        }
    };
    let appx_elapsed_ms = appx_started.elapsed().as_millis();
    let package_source_inventory = package_source_worker
        .take()
        .expect("the package source worker must be available")
        .join()
        .unwrap_or_else(|_| {
            log::warn!("windows_package_source_inventory_worker_failed");
            package_sources::PackageSourceInventory {
                complete: false,
                ..package_sources::PackageSourceInventory::default()
            }
        });
    if cancellation.is_cancelled() {
        return Err("windows_application_inventory_cancelled".to_string());
    }
    let package_source_elapsed_ms = package_source_started.elapsed().as_millis();
    let package_fact_count = package_source_inventory.facts.len();
    let reconciliation_started = Instant::now();
    package_reconciliation::merge(&mut applications, package_source_inventory.facts);
    let reconciliation_elapsed_ms = reconciliation_started.elapsed().as_millis();
    if opened_views == 0 {
        return Err("windows_application_registry_unavailable".to_string());
    }
    let mut installed_applications = applications.into_values().collect::<Vec<_>>();
    installed_applications.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });

    let filesystem_kinds = filesystem_name()
        .map(|value| normalize_fact(&value))
        .filter(|value| !value.is_empty())
        .into_iter()
        .collect::<Vec<_>>();
    let filesystem_complete = !filesystem_kinds.is_empty();
    let mut capabilities = Vec::new();
    if filesystem_kinds.iter().any(|value| value == "ntfs") {
        capabilities.push("usn-journal".to_string());
    }
    if RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            r"SYSTEM\CurrentControlSet\Services\WSearch",
            KEY_READ | KEY_WOW64_64KEY,
        )
        .is_ok()
    {
        capabilities.push("windows-search".to_string());
    }
    capabilities.extend(
        package_source_inventory
            .detected_sources
            .into_iter()
            .map(|source| format!("application-source:{source}")),
    );

    let (developer_tools, developer_tools_complete) = detect_tools(TOOL_NAMES);
    let result = SystemInventory {
        installed_applications,
        // A 32-bit or per-user uninstall key may legitimately not exist.
        // Completeness means every view that exists was enumerated without an
        // error, not that Windows created all four possible registry paths.
        //
        // Package managers are optional enrichment sources. A temporarily
        // unavailable Steam library or WinGet export must not invalidate the
        // authoritative registry/AppX catalog and hide every application from
        // the user. Their coverage is reported through capabilities_complete.
        installed_applications_complete: application_inventory_complete(
            opened_views,
            complete_views,
        ),
        developer_tools,
        developer_tools_complete,
        filesystem_kinds,
        filesystem_complete,
        capabilities,
        capabilities_complete: package_source_inventory.complete,
        os_version: windows_version(),
    };
    log::info!(
        "windows_application_inventory_ready application_count={} registry_view_count={} registry_complete_view_count={} registry_elapsed_ms={} appx_count={} appx_complete={} appx_elapsed_ms={} package_fact_count={} package_sources_elapsed_ms={} reconciliation_elapsed_ms={} elapsed_ms={}",
        result.installed_applications.len(),
        opened_views,
        complete_views,
        registry_elapsed_ms,
        appx_count,
        appx_complete,
        appx_elapsed_ms,
        package_fact_count,
        package_source_elapsed_ms,
        reconciliation_elapsed_ms,
        inventory_started.elapsed().as_millis(),
    );
    Ok(result)
}

pub(super) fn system_inventory_revision() -> Result<String, String> {
    system_inventory_revision_with_cancellation(&PlatformCancellation::new(|| false))
}

pub(super) fn system_inventory_revision_with_cancellation(
    cancellation: &PlatformCancellation,
) -> Result<String, String> {
    let started = Instant::now();
    let mut parts = Vec::new();
    let registry_started = Instant::now();
    for (root, view) in [
        (RegKey::predef(HKEY_CURRENT_USER), KEY_WOW64_64KEY),
        (RegKey::predef(HKEY_CURRENT_USER), KEY_WOW64_32KEY),
        (RegKey::predef(HKEY_LOCAL_MACHINE), KEY_WOW64_64KEY),
        (RegKey::predef(HKEY_LOCAL_MACHINE), KEY_WOW64_32KEY),
    ] {
        if cancellation.is_cancelled() {
            return Err("windows_application_inventory_revision_cancelled".to_string());
        }
        let Ok(uninstall) = root.open_subkey_with_flags(UNINSTALL_PATH, KEY_READ | view) else {
            parts.push(format!("{view}:missing"));
            continue;
        };
        parts.push(format!(
            "{view}:{}",
            uninstall_view_revision(&uninstall, cancellation)?
        ));
    }
    let registry_elapsed_ms = registry_started.elapsed().as_millis();
    if cancellation.is_cancelled() {
        return Err("windows_application_inventory_revision_cancelled".to_string());
    }
    let appx_started = Instant::now();
    parts.push(format!("appx:{}", appx_repository_revision()));
    let appx_elapsed_ms = appx_started.elapsed().as_millis();
    let package_sources_started = Instant::now();
    if cancellation.is_cancelled() {
        return Err("windows_application_inventory_revision_cancelled".to_string());
    }
    parts.push(format!(
        "package-sources:{}",
        package_sources::revision_fingerprint()
    ));
    let package_sources_elapsed_ms = package_sources_started.elapsed().as_millis();
    if cancellation.is_cancelled() {
        return Err("windows_application_inventory_revision_cancelled".to_string());
    }
    let revision = blake3::hash(parts.join("|").as_bytes())
        .to_hex()
        .to_string();
    log::debug!(
        "windows_application_inventory_revision_ready registry_elapsed_ms={} appx_elapsed_ms={} package_sources_elapsed_ms={} elapsed_ms={}",
        registry_elapsed_ms,
        appx_elapsed_ms,
        package_sources_elapsed_ms,
        started.elapsed().as_millis(),
    );
    Ok(revision)
}

fn uninstall_view_revision(
    uninstall: &RegKey,
    cancellation: &PlatformCancellation,
) -> Result<String, String> {
    let metadata = uninstall
        .query_info()
        .map_err(|error| format!("windows_inventory_revision_failed error={error}"))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(format!("{:?}", metadata.last_write_time).as_bytes());
    hasher.update(&metadata.sub_keys.to_le_bytes());

    // Updating a value inside an existing uninstall subkey does not change the
    // parent key's timestamp. Include every child's timestamp so a manual scan
    // cannot reuse stale commands, capabilities, or metadata after an installer
    // repairs or updates its registration.
    let mut key_names = uninstall
        .enum_keys()
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    key_names.sort_by_key(|name| name.to_ascii_lowercase());
    for key_name in key_names {
        if cancellation.is_cancelled() {
            return Err("windows_application_inventory_revision_cancelled".to_string());
        }
        hasher.update(key_name.to_ascii_lowercase().as_bytes());
        match uninstall
            .open_subkey_with_flags(&key_name, KEY_READ)
            .and_then(|key| key.query_info())
        {
            Ok(metadata) => {
                hasher.update(format!("{:?}", metadata.last_write_time).as_bytes());
                hasher.update(&metadata.values.to_le_bytes());
            }
            Err(_) => {
                hasher.update(b"unavailable");
            }
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn appx_repository_revision() -> String {
    let repository =
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(APPX_REPOSITORY_PATH, KEY_READ);
    match repository.and_then(|key| key.query_info()) {
        Ok(metadata) => format!("{:?}:{}", metadata.last_write_time, metadata.sub_keys),
        Err(_) => "unavailable".to_string(),
    }
}

fn read_packaged_applications(
    cancellation: &PlatformCancellation,
) -> Result<Vec<PackagedApplicationRecord>, String> {
    let json = powershell_json(APPX_INVENTORY_SCRIPT, cancellation)?;
    let inventory: PackagedApplicationInventory = serde_json::from_str(&json)
        .map_err(|error| format!("windows_packaged_application_parse_failed error={error}"))?;
    if inventory.schema_version != 1 {
        return Err("windows_packaged_application_schema_unsupported".to_string());
    }
    log::info!(
        "windows_packaged_application_filter_summary package_count={} visible_count={} excluded_count={} framework_count={} resource_count={} non_removable_count={} partially_staged_count={}",
        inventory.package_count, inventory.items.len(), inventory.excluded_count,
        inventory.framework_count, inventory.resource_count, inventory.non_removable_count,
        inventory.partially_staged_count
    );
    Ok(inventory.items)
}

fn powershell_json(script: &str, cancellation: &PlatformCancellation) -> Result<String, String> {
    let powershell = native_uninstall::system_powershell_path()
        .map_err(|error| format!("windows_powershell_resolve_failed error={error}"))?;
    let executable = ControlledExecutable::capture(&powershell).map_err(|error| {
        format!(
            "windows_powershell_capture_failed reason={}",
            error.as_str()
        )
    })?;
    let output = run_controlled_command(
        "windows-appx-inventory",
        &executable,
        &["-NoProfile", "-NonInteractive", "-Command", script],
        ControlledEnvironmentPolicy::Inherit,
        ControlledCommandLimits {
            timeout: APPX_COMMAND_TIMEOUT,
            stdout_bytes: APPX_OUTPUT_LIMIT,
            stderr_bytes: 256 * 1024,
        },
        &|| cancellation.is_cancelled(),
    )
    .map_err(|error| format!("windows_powershell_failed reason={}", error.as_str()))?;
    if !output.status.success() {
        return Err(format!(
            "windows_powershell_failed exit_code={:?}",
            output.status.code()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|error| format!("windows_powershell_output_invalid error={error}"))
}

fn merge_packaged_application(
    applications: &mut HashMap<String, InstalledApplication>,
    package: PackagedApplicationRecord,
) {
    if package.package_family_name.is_empty()
        || package.name.is_empty()
        || package.install_location.is_empty()
    {
        return;
    }
    let install_location = PathBuf::from(&package.install_location);
    let estimated_bytes = exclusive_directory_logical_size(&install_location).unwrap_or_default();
    let executable_path = (!package.executable.is_empty())
        .then(|| install_location.join(&package.executable))
        .filter(|path| path.is_file());
    let icon_path = package_icon_path(&install_location, &package.icon)
        .unwrap_or_else(|| install_location.clone());
    let uninstall_registration = ApplicationUninstallRegistration::WindowsAppx {
        package_family_name: package.package_family_name.clone(),
        package_full_name: package.package_full_name.clone(),
        estimated_bytes,
    };
    let matching_registry_key = applications.iter().find_map(|(key, existing)| {
        existing
            .identifiers
            .iter()
            .any(|identifier| {
                identifier.eq_ignore_ascii_case(&package.package_family_name)
                    || identifier.eq_ignore_ascii_case(&package.package_full_name)
            })
            .then(|| key.clone())
    });
    if let Some(key) = matching_registry_key {
        if let Some(existing) = applications.get_mut(&key) {
            merge_source_identity(
                &mut existing.source_identities,
                ApplicationSourceIdentity {
                    source: ApplicationInventorySource::WindowsAppx,
                    identifier: package.package_family_name.clone(),
                },
            );
            for identifier in [&package.package_family_name, &package.package_full_name] {
                if !existing
                    .identifiers
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(identifier))
                {
                    existing.identifiers.push(identifier.clone());
                }
            }
            // The AppX identity supersedes a matching registry display entry.
            // Keep the package-exclusive estimate instead of the larger value:
            // registry estimates and recursive logical sizes can include
            // single-instance files that another package still owns.
            existing.system_signed |= package.system_signed;
            existing.estimated_bytes = estimated_bytes;
            existing.installed_at_ms = install_location
                .metadata()
                .ok()
                .and_then(|metadata| metadata.created().or_else(|_| metadata.modified()).ok())
                .and_then(system_time_millis)
                .or(existing.installed_at_ms);
            existing.icon_path = Some(icon_path);
            existing.bundle_path = Some(install_location);
            if let Some(path) = executable_path {
                existing.executable_paths.push(path);
            }
            // Only an exact package identity may upgrade a display-only entry
            // to an executable uninstall candidate. Display names are not
            // unique and therefore cannot authorize package removal.
            existing.uninstall_registration = Some(uninstall_registration);
            existing.uninstall_diagnostic = None;
        }
        return;
    }

    let identity = format!("appx:{}", package.package_family_name.to_ascii_lowercase());
    applications.insert(
        identity,
        InstalledApplication {
            #[cfg(windows)]
            system_signed: package.system_signed,
            uninstall_diagnostic: None,
            catalog_identifier: format!(
                "windows-appx:{}",
                package.package_family_name.to_ascii_lowercase()
            ),
            source_identities: vec![ApplicationSourceIdentity {
                source: ApplicationInventorySource::WindowsAppx,
                identifier: package.package_family_name.clone(),
            }],
            primary_identifier: package.package_family_name.clone(),
            identifiers: vec![
                package.package_family_name,
                package.package_full_name,
                package.name.clone(),
            ],
            name: package.name,
            version: (!package.version.is_empty()).then_some(package.version),
            publisher: (!package.publisher.is_empty()).then_some(package.publisher),
            estimated_bytes,
            last_used_at_ms: None,
            installed_at_ms: install_location
                .metadata()
                .ok()
                .and_then(|metadata| metadata.created().or_else(|_| metadata.modified()).ok())
                .and_then(system_time_millis),
            icon_path: Some(icon_path),
            bundle_path: Some(install_location),
            executable_paths: executable_path.into_iter().collect(),
            uninstall_registration: Some(uninstall_registration),
        },
    );
}

/// Estimates bytes that can be released when a packaged application is removed.
///
/// Windows single-instance storage can expose one allocation through several
/// hard links. Counting every directory entry overstates the package size.
/// Links observed entirely inside this package are counted once, while an
/// allocation with links outside the package is excluded because removing this
/// package alone cannot release it.
fn exclusive_directory_logical_size(root: &Path) -> Option<u64> {
    const MAX_ENTRIES: usize = 500_000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let root_metadata = fs::symlink_metadata(root).ok()?;
    if !root_metadata.is_dir()
        || root_metadata.file_type().is_symlink()
        || root_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return None;
    }
    let mut directories = vec![root.to_path_buf()];
    let mut untracked_bytes = 0_u64;
    let mut files = HashMap::<WindowsFileIdentity, FileLinkSummary>::new();
    let mut entries_seen = 0_usize;
    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(directory).ok()?;
        for entry in entries {
            let entry = entry.ok()?;
            entries_seen += 1;
            if entries_seen > MAX_ENTRIES {
                return None;
            }
            let metadata = fs::symlink_metadata(entry.path()).ok()?;
            let file_type = metadata.file_type();
            if file_type.is_symlink()
                || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                continue;
            }
            if file_type.is_dir() {
                directories.push(entry.path());
            } else if file_type.is_file() {
                let Some((identity, total_links)) = file_link_evidence(&entry.path()) else {
                    // Filesystems without stable identities cannot be
                    // deduplicated. Preserve the visible logical size while
                    // the UI marks packaged-application sizes as estimates.
                    untracked_bytes = untracked_bytes.saturating_add(metadata.len());
                    continue;
                };
                files
                    .entry(identity)
                    .and_modify(|summary| {
                        summary.observed_links = summary.observed_links.saturating_add(1);
                        summary.total_links = summary.total_links.max(total_links);
                    })
                    .or_insert(FileLinkSummary {
                        bytes: metadata.len(),
                        observed_links: 1,
                        total_links,
                    });
            }
        }
    }
    Some(files.into_values().fold(untracked_bytes, |total, file| {
        if file.observed_links >= file.total_links {
            total.saturating_add(file.bytes)
        } else {
            total
        }
    }))
}

fn package_icon_path(install_location: &Path, declared_icon: &str) -> Option<PathBuf> {
    let relative = Path::new(declared_icon.trim());
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(install_location.join(relative))
}

pub(super) fn running_process_names(
    cancellation: &PlatformCancellation,
) -> Result<Vec<String>, String> {
    let tasklist = super::directories::system_directory()
        .map_err(|error| format!("windows_process_snapshot_resolve_failed error={error}"))?
        .join("System32")
        .join("tasklist.exe");
    let executable = ControlledExecutable::capture(&tasklist).map_err(|error| {
        format!(
            "windows_process_snapshot_capture_failed reason={}",
            error.as_str()
        )
    })?;
    let output = run_controlled_command(
        "windows-process-snapshot",
        &executable,
        &["/fo", "csv", "/nh"],
        ControlledEnvironmentPolicy::Inherit,
        ControlledCommandLimits {
            timeout: PROCESS_SNAPSHOT_TIMEOUT,
            stdout_bytes: PROCESS_SNAPSHOT_OUTPUT_LIMIT,
            stderr_bytes: 64 * 1024,
        },
        &|| cancellation.is_cancelled(),
    )
    .map_err(|error| format!("windows_process_snapshot_failed reason={}", error.as_str()))?;
    if !output.status.success() {
        return Err(format!(
            "windows_process_snapshot_failed exit_code={:?}",
            output.status.code()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split(',').next())
        .map(|name| name.trim().trim_matches('"').to_string())
        .filter(|name| !name.is_empty())
        .collect())
}

fn read_uninstall_view(
    root: &RegKey,
    view: u32,
    scope: RegistryScope,
    applications: &mut HashMap<String, InstalledApplication>,
    conflicting_registrations: &mut HashSet<String>,
) -> (bool, bool) {
    let uninstall = match root.open_subkey_with_flags(UNINSTALL_PATH, KEY_READ | view) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            log::info!(
                "windows_uninstall_inventory_view_absent scope={} registry_view={:?}",
                scope.stable_code(),
                registry_view(view)
            );
            return (false, false);
        }
        Err(error) => {
            log::warn!(
                "windows_uninstall_inventory_view_unavailable scope={} registry_view={:?} native_code={:?} error_kind={:?}",
                scope.stable_code(), registry_view(view), error.raw_os_error(), error.kind()
            );
            return (false, false);
        }
    };
    let mut complete = true;
    let mut excluded_counts = BTreeMap::<&str, usize>::new();
    for key_name in uninstall.enum_keys() {
        let key_name = match key_name {
            Ok(name) => name,
            Err(error) => {
                log::warn!("windows_uninstall_inventory_entry_failed scope={} registry_view={:?} stage=enumerate native_code={:?} error_kind={:?}", scope.stable_code(), registry_view(view), error.raw_os_error(), error.kind());
                complete = false;
                continue;
            }
        };
        let entry = match uninstall.open_subkey_with_flags(&key_name, KEY_READ | view) {
            Ok(entry) => entry,
            Err(error) => {
                log::warn!("windows_uninstall_inventory_entry_failed application_id={} scope={} registry_view={:?} stage=open_entry native_code={:?} error_kind={:?}", application_uninstall_diagnostic_id(&format!("windows-registry:{}:{}", scope.stable_code(), key_name.to_ascii_lowercase())), scope.stable_code(), registry_view(view), error.raw_os_error(), error.kind());
                complete = false;
                continue;
            }
        };
        if let Some(reason) = uninstall_entry_exclusion_reason(&entry) {
            *excluded_counts.entry(reason).or_default() += 1;
            continue;
        }
        let name = string_value(&entry, "DisplayName")
            .or_else(|| string_value(&entry, "QuietDisplayName"));
        let Some(name) = name.filter(|value| !value.trim().is_empty()) else {
            *excluded_counts.entry("display_name_missing").or_default() += 1;
            continue;
        };
        let estimated_size_kib = entry.get_value::<u32, _>("EstimatedSize").ok();
        let registry_estimated_bytes = estimated_bytes_from_kib(estimated_size_kib);
        let registry_install_date =
            string_value(&entry, "InstallDate").and_then(|value| parse_install_date(&value));
        let mut uninstall_diagnostic = None;
        let mut uninstall_registration =
            msi_registration(&entry, &key_name).or_else(
                || match registered_uninstall_registration(
                    &entry,
                    &key_name,
                    scope,
                    registry_view(view),
                ) {
                    Ok(registration) => Some(registration),
                    Err(reason) => {
                        uninstall_diagnostic = Some(reason);
                        None
                    }
                },
            );
        let chocolatey_package = string_value(&entry, "ChocolateyPackageName").or_else(|| {
            string_value(&entry, "InstallSource")
                .as_deref()
                .and_then(package_sources::chocolatey_package_from_install_source)
        });
        if let Some(registration) = chocolatey_package.as_deref().and_then(|package_name| {
            package_sources::chocolatey_uninstall_registration(
                package_name,
                registry_estimated_bytes,
            )
        }) {
            // A verified package-manager registration takes precedence over
            // the underlying MSI registration. Direct MSI removal would leave
            // Chocolatey's package database stale.
            uninstall_registration = Some(registration);
            uninstall_diagnostic = None;
        }
        let identity_scope = uninstall_registration
            .as_ref()
            .map(|registration| match registration {
                ApplicationUninstallRegistration::WindowsMsi { scope, .. } => match scope {
                    ApplicationInstallScope::CurrentUser => "current-user",
                    ApplicationInstallScope::Machine => "machine",
                },
                ApplicationUninstallRegistration::WindowsAppx { .. } => "current-user",
                ApplicationUninstallRegistration::WindowsScoop { scope, .. } => match scope {
                    ApplicationInstallScope::CurrentUser => "current-user",
                    ApplicationInstallScope::Machine => "machine",
                },
                ApplicationUninstallRegistration::WindowsChocolatey { .. } => "machine",
                ApplicationUninstallRegistration::WindowsRegistered { scope, .. } => match scope {
                    ApplicationInstallScope::CurrentUser => "current-user",
                    ApplicationInstallScope::Machine => "machine",
                },
            })
            .unwrap_or_else(|| scope.stable_code());
        let scoped_identity = format!("windows-registry/{identity_scope}/{key_name}");
        let mut identifiers = vec![key_name.clone(), name.clone(), scoped_identity];
        if let Some(publisher) = string_value(&entry, "Publisher") {
            identifiers.push(format!("{publisher}/{name}"));
        }
        identifiers.sort_by_key(|value| value.to_ascii_lowercase());
        identifiers.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        let mut additional_source_identities = Vec::new();
        for (value_name, source) in [(
            "WinGetPackageIdentifier",
            ApplicationInventorySource::Winget,
        )] {
            if let Some(identifier) = string_value(&entry, value_name) {
                additional_source_identities.push(ApplicationSourceIdentity { source, identifier });
            }
        }
        if let Some(identifier) = string_value(&entry, "ScoopPackageName") {
            additional_source_identities.push(ApplicationSourceIdentity {
                source: ApplicationInventorySource::Scoop,
                identifier: format!("{}:{identifier}", scope.stable_code()),
            });
        }
        if let Some(identifier) = chocolatey_package {
            additional_source_identities.push(ApplicationSourceIdentity {
                source: ApplicationInventorySource::Chocolatey,
                identifier,
            });
        }
        if let Some(app_id) = key_name.strip_prefix("Steam App ").filter(|value| {
            !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
        }) {
            additional_source_identities.push(ApplicationSourceIdentity {
                source: ApplicationInventorySource::Steam,
                identifier: app_id.to_string(),
            });
        }

        let install_location = string_value(&entry, "InstallLocation")
            .as_deref()
            .and_then(parse_registry_path);
        let display_icon = string_value(&entry, "DisplayIcon");
        let msi_product_code =
            uninstall_registration
                .as_ref()
                .and_then(|registration| match registration {
                    ApplicationUninstallRegistration::WindowsMsi { product_code, .. } => {
                        Some(product_code.as_str())
                    }
                    _ => None,
                });
        let icon_path = application_icon_path(
            display_icon.as_deref(),
            install_location.as_deref(),
            msi_product_code,
            msi_product_icon_path,
        );
        let executable_paths = icon_path
            .as_ref()
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
            })
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        let estimated_bytes = if registry_estimated_bytes > 0 {
            registry_estimated_bytes
        } else if matches!(
            uninstall_registration.as_ref(),
            Some(ApplicationUninstallRegistration::WindowsRegistered {
                scope: ApplicationInstallScope::CurrentUser,
                ..
            })
        ) {
            current_user_install_location_bytes(install_location.as_deref(), icon_path.as_deref())
                .unwrap_or_default()
        } else {
            0
        };
        if let Some(ApplicationUninstallRegistration::WindowsRegistered {
            estimated_bytes: registration_bytes,
            ..
        }) = uninstall_registration.as_mut()
        {
            *registration_bytes = estimated_bytes;
        }
        // InstallDate is optional and frequently missing for portable or
        // user-scoped installers. Directory metadata is display-only
        // fallback evidence: it improves sorting without authorizing an
        // uninstall or changing the registered command identity.
        let installed_at_ms = registry_install_date.or_else(|| {
            install_location
                .as_deref()
                .or_else(|| executable_paths.first().and_then(|path| path.parent()))
                .and_then(path_timestamp_millis)
        });
        let identity = format!("{identity_scope}:{}", key_name.to_ascii_lowercase());
        let mut source_identities = vec![ApplicationSourceIdentity {
            source: ApplicationInventorySource::WindowsRegistry,
            identifier: format!("{identity_scope}:{key_name}"),
        }];
        if let Some(ApplicationUninstallRegistration::WindowsMsi { product_code, .. }) =
            uninstall_registration.as_ref()
        {
            source_identities.push(ApplicationSourceIdentity {
                source: ApplicationInventorySource::WindowsMsi,
                identifier: product_code.clone(),
            });
        }
        for source_identity in additional_source_identities {
            merge_source_identity(&mut source_identities, source_identity);
        }
        if conflicting_registrations.contains(&identity) {
            uninstall_registration = None;
            uninstall_diagnostic = Some(ApplicationUninstallDiagnostic::RegistrationConflict);
        }
        applications
            .entry(identity.clone())
            .and_modify(|existing| {
                for source_identity in &source_identities {
                    merge_source_identity(&mut existing.source_identities, source_identity.clone());
                }
                for identifier in &identifiers {
                    if !existing
                        .identifiers
                        .iter()
                        .any(|value| value.eq_ignore_ascii_case(identifier))
                    {
                        existing.identifiers.push(identifier.clone());
                    }
                }
                for path in &executable_paths {
                    if !existing.executable_paths.contains(path) {
                        existing.executable_paths.push(path.clone());
                    }
                }
                match (
                    existing.uninstall_registration.as_ref(),
                    uninstall_registration.as_ref(),
                ) {
                    (None, Some(_)) => {
                        existing
                            .uninstall_registration
                            .clone_from(&uninstall_registration);
                        existing.uninstall_diagnostic = None;
                    }
                    (Some(_), Some(_))
                        if !registrations_are_compatible(
                            existing.uninstall_registration.as_ref(),
                            uninstall_registration.as_ref(),
                        ) =>
                    {
                        // Conflicting 32-bit and 64-bit registration facts
                        // must never produce an executable native-uninstall
                        // candidate. Remember the conflict so a later registry
                        // view cannot accidentally restore one side.
                        conflicting_registrations.insert(identity.clone());
                        existing.uninstall_registration = None;
                        existing.uninstall_diagnostic = Some(ApplicationUninstallDiagnostic::RegistrationConflict);
                        log::warn!(
                            "windows_uninstall_registration_rejected application_id={} scope={} registry_view={:?} reason=registration_conflict",
                            application_uninstall_diagnostic_id(&existing.catalog_identifier),
                            scope.stable_code(), registry_view(view)
                        );
                    }
                    _ => {}
                }
                // Size is presentation metadata rather than executable
                // evidence, so it remains useful even when two registry views
                // disagree about the native uninstall registration.
                existing.estimated_bytes = existing.estimated_bytes.max(estimated_bytes);
                if existing.installed_at_ms.is_none() {
                    existing.installed_at_ms = installed_at_ms;
                }
                if existing.icon_path.is_none() {
                    existing.icon_path.clone_from(&icon_path);
                }
                if existing.bundle_path.is_none() {
                    existing.bundle_path.clone_from(&install_location);
                }
            })
            .or_insert(InstalledApplication {
                system_signed: false,
                uninstall_diagnostic,
                catalog_identifier: format!("windows-registry:{identity}"),
                source_identities,
                primary_identifier: key_name,
                identifiers,
                name,
                version: string_value(&entry, "DisplayVersion"),
                publisher: string_value(&entry, "Publisher"),
                estimated_bytes,
                last_used_at_ms: None,
                installed_at_ms,
                icon_path,
                bundle_path: install_location,
                executable_paths,
                uninstall_registration,
            });
    }
    log::info!(
        "windows_uninstall_inventory_filter_summary scope={} registry_view={:?} complete={} excluded_counts={:?}",
        scope.stable_code(), registry_view(view), complete, excluded_counts
    );
    (true, complete)
}

fn merge_source_identity(
    identities: &mut Vec<ApplicationSourceIdentity>,
    identity: ApplicationSourceIdentity,
) {
    if identities.iter().any(|existing| {
        existing.source == identity.source
            && existing
                .identifier
                .eq_ignore_ascii_case(&identity.identifier)
    }) {
        return;
    }
    identities.push(identity);
    identities.sort_by(|left, right| {
        left.source.cmp(&right.source).then_with(|| {
            left.identifier
                .to_ascii_lowercase()
                .cmp(&right.identifier.to_ascii_lowercase())
        })
    });
}

fn path_timestamp_millis(path: &Path) -> Option<u64> {
    let metadata = path.metadata().ok()?;
    metadata
        .created()
        .or_else(|_| metadata.modified())
        .ok()
        .and_then(system_time_millis)
}

fn application_icon_path(
    display_icon: Option<&str>,
    install_location: Option<&Path>,
    msi_product_code: Option<&str>,
    msi_icon_lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> Option<PathBuf> {
    display_icon
        .and_then(parse_display_icon_path)
        .map(|path| PathBuf::from(expand_environment_path(&path)))
        .or_else(|| msi_product_code.and_then(msi_icon_lookup))
        .or_else(|| install_location.map(Path::to_path_buf))
}

fn msi_product_icon_path(product_code: &str) -> Option<PathBuf> {
    let product_code = product_code
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut value_len = 0_u32;
    // SAFETY: both input strings are terminated UTF-16 buffers. The initial
    // call follows the MSI sizing contract and does not provide an output
    // buffer.
    let status = unsafe {
        MsiGetProductInfoW(
            product_code.as_ptr(),
            INSTALLPROPERTY_PRODUCTICON,
            ptr::null_mut(),
            &mut value_len,
        )
    };
    if !msi_property_size_is_available(status, value_len) {
        return None;
    }

    let capacity = usize::try_from(value_len).ok()?.checked_add(1)?;
    let mut value = vec![0_u16; capacity];
    let mut buffer_len = u32::try_from(capacity).ok()?;
    // SAFETY: the output buffer contains `buffer_len` UTF-16 code units and
    // remains valid for the duration of the call.
    let status = unsafe {
        MsiGetProductInfoW(
            product_code.as_ptr(),
            INSTALLPROPERTY_PRODUCTICON,
            value.as_mut_ptr(),
            &mut buffer_len,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    value.truncate(usize::try_from(buffer_len).ok()?);
    let path = PathBuf::from(OsString::from_wide(&value));
    path.is_file().then_some(path)
}

fn msi_property_size_is_available(status: u32, value_len: u32) -> bool {
    matches!(status, ERROR_SUCCESS | ERROR_MORE_DATA) && value_len > 0
}

fn current_user_install_location_bytes(
    install_location: Option<&Path>,
    icon_path: Option<&Path>,
) -> Option<u64> {
    let install_location = fs::canonicalize(install_location?).ok()?;
    let icon_path = fs::canonicalize(icon_path?).ok()?;
    let user_directories = super::directories::user_directories().ok()?;
    let user_profile = fs::canonicalize(user_directories.home_directory()).ok()?;
    if windows_path_eq(&install_location, &user_profile)
        || !windows_path_is_same_or_child(&install_location, &user_profile)
        || !windows_path_is_same_or_child(&icon_path, &install_location)
    {
        return None;
    }
    for directory in user_directories
        .application_storage_directories()
        .iter()
        .map(PathBuf::as_path)
        .chain(std::iter::once(user_directories.temporary_directory()))
    {
        let Some(root) = fs::canonicalize(directory).ok() else {
            continue;
        };
        if windows_path_eq(&install_location, &root) {
            return None;
        }
    }
    exclusive_directory_logical_size(&install_location)
}

fn windows_path_eq(left: &Path, right: &Path) -> bool {
    path_identity::equal(left, right)
}

fn windows_path_is_same_or_child(path: &Path, root: &Path) -> bool {
    path_identity::is_same_or_child(path, root)
}

fn registrations_are_compatible(
    left: Option<&ApplicationUninstallRegistration>,
    right: Option<&ApplicationUninstallRegistration>,
) -> bool {
    if left == right {
        return true;
    }
    matches!(
        (left, right),
        (
            Some(ApplicationUninstallRegistration::WindowsRegistered {
                key_name: left_key,
                scope: left_scope,
                command_kind: left_kind,
                command_digest: left_digest,
                ..
            }),
            Some(ApplicationUninstallRegistration::WindowsRegistered {
                key_name: right_key,
                scope: right_scope,
                command_kind: right_kind,
                command_digest: right_digest,
                ..
            })
        ) if left_key.eq_ignore_ascii_case(right_key)
            && left_scope == right_scope
            && left_kind == right_kind
            && left_digest == right_digest
    )
}

fn uninstall_entry_exclusion_reason(entry: &RegKey) -> Option<&'static str> {
    if entry.get_value::<u32, _>("SystemComponent").ok() == Some(1) {
        Some("hidden_component")
    } else if string_value(entry, "ParentKeyName").is_some() {
        Some("child_registration")
    } else if matches!(
        string_value(entry, "ReleaseType").as_deref(),
        Some("Update" | "Hotfix" | "Security Update")
    ) {
        Some("system_update")
    } else {
        None
    }
}

fn expand_environment_path(value: &str) -> String {
    let mut expanded = value.to_string();
    for (name, replacement) in env::vars() {
        let token = format!("%{name}%");
        if expanded
            .to_ascii_lowercase()
            .contains(&token.to_ascii_lowercase())
        {
            expanded = replace_ascii_case_insensitive(&expanded, &token, &replacement);
        }
    }
    expanded
}

fn parse_registry_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let value = if let Some(quoted) = value.strip_prefix('"') {
        quoted.strip_suffix('"')?.trim()
    } else {
        value
    };
    (!value.is_empty()).then(|| PathBuf::from(expand_environment_path(value)))
}

pub(super) fn parse_display_icon_path(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(quoted) = value.strip_prefix('"') {
        let closing_quote = quoted.find('"')?;
        let path = quoted[..closing_quote].trim();
        return (!path.is_empty()).then(|| path.to_string());
    }

    let path = value
        .rsplit_once(',')
        .filter(|(_, suffix)| suffix.trim().parse::<i32>().is_ok())
        .map_or(value, |(path, _)| path)
        .trim();
    (!path.is_empty()).then(|| path.to_string())
}

fn replace_ascii_case_insensitive(value: &str, pattern: &str, replacement: &str) -> String {
    let lower_value = value.to_ascii_lowercase();
    let lower_pattern = pattern.to_ascii_lowercase();
    let mut output = String::with_capacity(value.len());
    let mut start = 0;
    while let Some(relative) = lower_value[start..].find(&lower_pattern) {
        let index = start + relative;
        output.push_str(&value[start..index]);
        output.push_str(replacement);
        start = index + pattern.len();
    }
    output.push_str(&value[start..]);
    output
}

fn msi_registration(entry: &RegKey, key_name: &str) -> Option<ApplicationUninstallRegistration> {
    let product_code =
        msi_product_code(entry.get_value::<u32, _>("WindowsInstaller").ok(), key_name)?;
    let scope = match native_uninstall::msi_install_scope(&product_code) {
        Ok(Some(scope)) => scope,
        Ok(None) => return None,
        Err(error) => {
            let error = error.to_string();
            log::warn!(
                "windows_msi_context_query_failed error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
            return None;
        }
    };
    Some(msi_registration_from_evidence(
        product_code,
        scope,
        entry.get_value::<u32, _>("EstimatedSize").ok(),
    ))
}

fn registered_uninstall_registration(
    entry: &RegKey,
    key_name: &str,
    scope: RegistryScope,
    registry_view: WindowsRegistryView,
) -> Result<ApplicationUninstallRegistration, ApplicationUninstallDiagnostic> {
    let application_id = application_uninstall_diagnostic_id(&format!(
        "windows-registry:{}:{}",
        scope.stable_code(),
        key_name.to_ascii_lowercase()
    ));
    let evidence = entry
        .get_value::<String, _>("UninstallString")
        .map_err(|error| native_uninstall::RegisteredCommandRejection {
            reason: if error.kind() == std::io::ErrorKind::NotFound {
                ApplicationUninstallDiagnostic::CommandMissing
            } else {
                ApplicationUninstallDiagnostic::CommandUnreadable
            },
            native_code: error.raw_os_error(),
            detail: "read_registry_value",
            target_kind: "unknown",
        })
        .and_then(|command| {
            native_uninstall::registered_uninstall_command_evidence_with_diagnostic(
                &command,
                key_name,
                scope.install_scope(),
            )
        });
    let (command_kind, command_digest) = evidence.map_err(|rejection| {
        // Log the exact rejection at the evidence boundary, not just a catalog blocked count.
        // The UI interaction logs use this same redacted ID so support can locate an affected application
        // without recording its name, registry key, command line, or installation directory.
        log::warn!(
            "windows_uninstall_registration_rejected application_id={} scope={} registry_view={:?} source=uninstall_string reason={} detail={} target_kind={} native_code={:?} quiet_command_present={}",
            application_id, scope.stable_code(), registry_view, rejection.reason.stable_code(), rejection.detail, rejection.target_kind,
            rejection.native_code, entry.get_raw_value("QuietUninstallString").is_ok()
        );
        rejection.reason
    })?;
    if command_kind == crate::WindowsRegisteredUninstallKind::BatchScript {
        log::info!("windows_registered_batch_uninstaller_recognized application_id={} scope={} registry_view={:?} command_kind=batch-script", application_id, scope.stable_code(), registry_view);
    }
    if command_kind == crate::WindowsRegisteredUninstallKind::Rundll32 {
        log::info!("windows_registered_rundll32_uninstaller_recognized application_id={} scope={} registry_view={:?} command_kind=rundll32", application_id, scope.stable_code(), registry_view);
    }
    Ok(ApplicationUninstallRegistration::WindowsRegistered {
        key_name: key_name.to_string(),
        scope: scope.install_scope(),
        registry_view,
        command_kind,
        command_digest,
        estimated_bytes: estimated_bytes_from_kib(entry.get_value::<u32, _>("EstimatedSize").ok()),
    })
}

fn msi_product_code(windows_installer: Option<u32>, key_name: &str) -> Option<String> {
    (windows_installer == Some(1))
        .then(|| normalize_product_code(key_name))
        .flatten()
}

fn msi_registration_from_evidence(
    product_code: String,
    scope: ApplicationInstallScope,
    estimated_size_kib: Option<u32>,
) -> ApplicationUninstallRegistration {
    let estimated_bytes = estimated_bytes_from_kib(estimated_size_kib);
    ApplicationUninstallRegistration::WindowsMsi {
        product_code,
        scope,
        estimated_bytes,
    }
}

fn estimated_bytes_from_kib(estimated_size_kib: Option<u32>) -> u64 {
    estimated_size_kib
        .map(u64::from)
        .unwrap_or_default()
        .saturating_mul(1024)
}

fn parse_install_date(value: &str) -> Option<u64> {
    let digits = value.trim();
    if digits.len() != 8 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = digits[0..4].parse::<i32>().ok()?;
    let month = digits[4..6].parse::<u32>().ok()?;
    let day = digits[6..8].parse::<u32>().ok()?;
    let days = days_from_civil(year, month, day)?;
    u64::try_from(days).ok()?.checked_mul(86_400_000)
}

/// Converts a Gregorian calendar date to days since the Unix epoch without
/// adding a date-time dependency to the platform inventory.
fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31_u32,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day == 0 || day > month_days[usize::try_from(month - 1).ok()?] {
        return None;
    }
    let adjusted_year = year - i32::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = i32::try_from(month).ok()? + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + i32::try_from(day).ok()? - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(i64::from(era * 146_097 + day_of_era - 719_468))
}

fn system_time_millis(value: std::time::SystemTime) -> Option<u64> {
    value
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

fn normalize_product_code(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() != 38
        || bytes.first() != Some(&b'{')
        || bytes.last() != Some(&b'}')
        || [9, 14, 19, 24]
            .into_iter()
            .any(|index| bytes.get(index) != Some(&b'-'))
        || bytes[1..37].iter().enumerate().any(|(index, byte)| {
            !matches!(index + 1, 9 | 14 | 19 | 24) && !byte.is_ascii_hexdigit()
        })
    {
        return None;
    }
    Some(value.to_ascii_uppercase())
}

fn string_value(key: &RegKey, name: &str) -> Option<String> {
    key.get_value::<String, _>(name).ok()
}

fn windows_version() -> String {
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            KEY_READ | KEY_WOW64_64KEY,
        )
        .ok();
    let product = key
        .as_ref()
        .and_then(|key| string_value(key, "ProductName"));
    let release = key
        .as_ref()
        .and_then(|key| string_value(key, "DisplayVersion"));
    let build = key
        .as_ref()
        .and_then(|key| string_value(key, "CurrentBuildNumber"));
    let version = [product, release, build]
        .into_iter()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if version.is_empty() {
        "unknown".to_string()
    } else {
        version
    }
}

fn filesystem_name() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetVolumeInformationW(
            root_path_name: *const u16,
            volume_name_buffer: *mut u16,
            volume_name_size: u32,
            volume_serial_number: *mut u32,
            maximum_component_length: *mut u32,
            file_system_flags: *mut u32,
            file_system_name_buffer: *mut u16,
            file_system_name_size: u32,
        ) -> i32;
    }
    let root = super::volumes::system_drive_path();
    let root = root
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut filesystem = [0_u16; 64];
    let succeeded = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    };
    if succeeded == 0 {
        return None;
    }
    let length = filesystem
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(filesystem.len());
    Some(
        OsString::from_wide(&filesystem[..length])
            .to_string_lossy()
            .into_owned(),
    )
}

fn application_inventory_complete(opened_views: usize, complete_views: usize) -> bool {
    opened_views > 0 && complete_views == opened_views
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::PathBuf};

    use crate::{
        ApplicationInstallScope, ApplicationInventorySource, ApplicationSourceIdentity,
        ApplicationUninstallRegistration, ApplicationUninstallRegistrationState,
        InstalledApplication, WindowsRegistryView,
    };

    use super::{
        application_icon_path, application_inventory_complete, appx_repository_revision,
        current_user_install_location_bytes, estimated_bytes_from_kib, merge_packaged_application,
        msi_product_code, msi_property_size_is_available, msi_registration_from_evidence,
        normalize_product_code, package_icon_path, package_sources, parse_display_icon_path,
        parse_install_date, parse_registry_path, registrations_are_compatible,
        running_process_names, system_inventory, system_inventory_revision,
        system_inventory_revision_with_cancellation, PackagedApplicationRecord,
    };
    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};

    #[test]
    fn cancelled_process_snapshot_does_not_start_tasklist() {
        let cancellation = crate::PlatformCancellation::new(|| true);

        let error = running_process_names(&cancellation)
            .expect_err("a pre-cancelled process snapshot must stop");

        assert!(error.contains("cancelled"));
    }

    #[test]
    fn cancelled_inventory_revision_stops_before_registry_access() {
        let cancellation = crate::PlatformCancellation::new(|| true);

        let error = system_inventory_revision_with_cancellation(&cancellation)
            .expect_err("a pre-cancelled revision capture must stop");

        assert!(error.contains("cancelled"));
    }

    #[test]
    fn registry_install_date_parser_validates_calendar_dates() {
        assert_eq!(parse_install_date("19700101"), Some(0));
        assert_eq!(parse_install_date("19700102"), Some(86_400_000));
        assert_eq!(parse_install_date("20240229"), Some(1_709_164_800_000));
        assert_eq!(parse_install_date("20230229"), None);
        assert_eq!(parse_install_date("20241301"), None);
        assert_eq!(parse_install_date("2024-01-01"), None);
    }

    #[test]
    fn absent_uninstall_views_do_not_make_existing_views_incomplete() {
        assert!(application_inventory_complete(2, 2));
        assert!(!application_inventory_complete(2, 1));
        assert!(!application_inventory_complete(0, 0));
    }

    #[test]
    fn product_codes_require_the_canonical_braced_guid_shape() {
        assert_eq!(
            normalize_product_code("{01234567-89ab-cdef-0123-456789abcdef}"),
            Some("{01234567-89AB-CDEF-0123-456789ABCDEF}".to_string())
        );
        assert!(normalize_product_code("01234567-89ab-cdef-0123-456789abcdef").is_none());
        assert!(normalize_product_code("{01234567-89ab-cdef-0123-456789abcdeg}").is_none());
        assert!(normalize_product_code("{0123456789ab-cdef-0123-456789abcdef}").is_none());
    }

    #[test]
    fn msi_product_icon_fills_only_missing_registry_icon_evidence() {
        let install_location = std::path::Path::new(r"C:\Program Files\Example");
        let registry_icon = application_icon_path(
            Some(r#""C:\Program Files\Example\example.exe",0"#),
            Some(install_location),
            Some("{01234567-89AB-CDEF-0123-456789ABCDEF}"),
            |_| panic!("the MSI icon must not replace explicit registry evidence"),
        );
        assert_eq!(
            registry_icon,
            Some(std::path::PathBuf::from(
                r"C:\Program Files\Example\example.exe"
            ))
        );

        let msi_icon = application_icon_path(
            None,
            Some(install_location),
            Some("{01234567-89AB-CDEF-0123-456789ABCDEF}"),
            |product_code| {
                assert_eq!(product_code, "{01234567-89AB-CDEF-0123-456789ABCDEF}");
                Some(std::path::PathBuf::from(
                    r"C:\Windows\Installer\ExampleIcon",
                ))
            },
        );
        assert_eq!(
            msi_icon,
            Some(std::path::PathBuf::from(
                r"C:\Windows\Installer\ExampleIcon"
            ))
        );

        let install_fallback = application_icon_path(None, Some(install_location), None, |_| None);
        assert_eq!(install_fallback, Some(install_location.to_path_buf()));
    }

    #[test]
    fn msi_property_sizing_accepts_both_documented_windows_results() {
        assert!(msi_property_size_is_available(ERROR_SUCCESS, 24));
        assert!(msi_property_size_is_available(ERROR_MORE_DATA, 24));
        assert!(!msi_property_size_is_available(ERROR_SUCCESS, 0));
        assert!(!msi_property_size_is_available(1605, 24));
    }

    #[test]
    fn native_msi_evidence_requires_the_registry_marker_and_product_code() {
        assert_eq!(estimated_bytes_from_kib(Some(512)), 512 * 1024);
        assert_eq!(estimated_bytes_from_kib(None), 0);
        assert_eq!(
            msi_product_code(Some(1), "{01234567-89ab-cdef-0123-456789abcdef}",),
            Some("{01234567-89AB-CDEF-0123-456789ABCDEF}".to_string())
        );
        assert!(msi_product_code(None, "{01234567-89ab-cdef-0123-456789abcdef}").is_none());
        assert!(msi_product_code(Some(1), "vendor-uninstaller").is_none());
        assert_eq!(
            msi_registration_from_evidence(
                "{01234567-89AB-CDEF-0123-456789ABCDEF}".to_string(),
                ApplicationInstallScope::CurrentUser,
                Some(512),
            ),
            ApplicationUninstallRegistration::WindowsMsi {
                product_code: "{01234567-89AB-CDEF-0123-456789ABCDEF}".to_string(),
                scope: ApplicationInstallScope::CurrentUser,
                estimated_bytes: 512 * 1024,
            }
        );
        assert_eq!(
            msi_registration_from_evidence(
                "{01234567-89AB-CDEF-0123-456789ABCDEF}".to_string(),
                ApplicationInstallScope::Machine,
                None,
            ),
            ApplicationUninstallRegistration::WindowsMsi {
                product_code: "{01234567-89AB-CDEF-0123-456789ABCDEF}".to_string(),
                scope: ApplicationInstallScope::Machine,
                estimated_bytes: 0,
            }
        );
    }

    #[test]
    fn display_icon_paths_preserve_quoted_commas_and_remove_resource_indexes() {
        assert_eq!(
            parse_display_icon_path(r#""C:\Program Files\Example, Inc\app.exe",-1"#),
            Some(r"C:\Program Files\Example, Inc\app.exe".to_string())
        );
        assert_eq!(
            parse_display_icon_path(r"C:\Tools\app.exe,0"),
            Some(r"C:\Tools\app.exe".to_string())
        );
        assert_eq!(
            parse_display_icon_path(r"C:\Tools\comma,name.ico"),
            Some(r"C:\Tools\comma,name.ico".to_string())
        );
        assert_eq!(parse_display_icon_path("  "), None);
    }

    #[test]
    fn registry_paths_strip_only_a_balanced_outer_quote_pair() {
        assert_eq!(
            parse_registry_path(r#" "C:\Users\developer\AppData\Local\Project Graph" "#),
            Some(PathBuf::from(
                r"C:\Users\developer\AppData\Local\Project Graph"
            ))
        );
        assert_eq!(
            parse_registry_path(r"C:\Program Files\Example"),
            Some(PathBuf::from(r"C:\Program Files\Example"))
        );
        assert_eq!(parse_registry_path(r#""C:\Program Files\Example"#), None);
        assert_eq!(parse_registry_path(""), None);
    }

    #[test]
    fn duplicate_registry_views_keep_only_identical_registered_uninstall_evidence() {
        let registration =
            |view, digest: &str| ApplicationUninstallRegistration::WindowsRegistered {
                key_name: "Vendor.App".to_string(),
                scope: ApplicationInstallScope::CurrentUser,
                registry_view: view,
                command_kind: crate::WindowsRegisteredUninstallKind::Executable,
                command_digest: digest.to_string(),
                estimated_bytes: 1_024,
            };

        let registry_32 = registration(WindowsRegistryView::Registry32, "verified-command-digest");
        let registry_64 = registration(WindowsRegistryView::Registry64, "verified-command-digest");
        assert!(registrations_are_compatible(
            Some(&registry_32),
            Some(&registry_64)
        ));

        let conflicting = registration(WindowsRegistryView::Registry64, "different-command-digest");
        assert!(!registrations_are_compatible(
            Some(&registry_32),
            Some(&conflicting)
        ));
    }

    #[test]
    fn packaged_application_does_not_merge_on_display_name_alone() {
        let mut applications = HashMap::from([(
            "machine:example".to_string(),
            InstalledApplication {
                #[cfg(windows)]
                system_signed: false,
                uninstall_diagnostic: None,
                catalog_identifier: "windows-registry:machine:example".to_string(),
                source_identities: vec![ApplicationSourceIdentity {
                    source: ApplicationInventorySource::WindowsRegistry,
                    identifier: "machine:example".to_string(),
                }],
                primary_identifier: "Example.Registry".to_string(),
                identifiers: vec!["Example.Registry".to_string()],
                name: "Example".to_string(),
                version: None,
                publisher: None,
                estimated_bytes: 0,
                last_used_at_ms: None,
                installed_at_ms: None,
                icon_path: None,
                bundle_path: None,
                executable_paths: Vec::new(),
                uninstall_registration: None,
            },
        )]);

        merge_packaged_application(
            &mut applications,
            PackagedApplicationRecord {
                system_signed: false,
                package_family_name: "Example_123".to_string(),
                package_full_name: "Example_1.0.0.0_x64__123".to_string(),
                name: "Example".to_string(),
                version: "1.0.0.0".to_string(),
                publisher: "Example Publisher".to_string(),
                install_location: r"C:\Program Files\WindowsApps\Example".to_string(),
                executable: String::new(),
                icon: r"Assets\Square150x150Logo.png".to_string(),
            },
        );

        assert_eq!(applications.len(), 2);
        assert!(applications["machine:example"]
            .uninstall_registration
            .is_none());
        assert!(applications.values().any(|application| {
            matches!(
                application.uninstall_registration.as_ref(),
                Some(ApplicationUninstallRegistration::WindowsAppx { .. })
            )
        }));
    }
    #[test]
    fn packaged_application_icon_must_remain_inside_the_package() {
        let root = std::path::Path::new(r"C:\Program Files\WindowsApps\Example");
        assert_eq!(
            package_icon_path(root, r"Assets\Square150x150Logo.png"),
            Some(root.join(r"Assets\Square150x150Logo.png"))
        );
        assert_eq!(package_icon_path(root, r"..\outside.png"), None);
        assert_eq!(package_icon_path(root, r"C:\outside.png"), None);
        assert_eq!(package_icon_path(root, ""), None);
    }

    #[test]
    fn current_user_install_size_requires_icon_identity_inside_the_root() {
        let unique = format!(
            "mangodisk-user-install-size-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should follow the Unix epoch")
                .as_nanos()
        );
        let fixture_root = std::env::temp_dir().join(unique);
        let install_root = fixture_root.join("application");
        let outside_root = fixture_root.join("outside");
        fs::create_dir_all(install_root.join("bin")).expect("install fixture should be created");
        fs::create_dir_all(&outside_root).expect("outside fixture should be created");
        let icon = install_root.join("bin").join("application.exe");
        fs::write(&icon, vec![1_u8; 1_024]).expect("icon fixture should be written");
        fs::write(install_root.join("data.bin"), vec![2_u8; 2_048])
            .expect("data fixture should be written");
        let outside_icon = outside_root.join("application.exe");
        fs::write(&outside_icon, vec![3_u8; 512]).expect("outside fixture should be written");

        assert_eq!(
            current_user_install_location_bytes(Some(&install_root), Some(&icon)),
            Some(3_072)
        );
        assert_eq!(
            current_user_install_location_bytes(Some(&install_root), Some(&outside_icon)),
            None
        );
        assert_eq!(
            current_user_install_location_bytes(
                Some(std::path::Path::new(
                    &std::env::var("USERPROFILE").expect("USERPROFILE should exist")
                )),
                Some(&icon),
            ),
            None
        );
        fs::remove_dir_all(fixture_root).expect("fixture should be removed");
    }

    #[test]
    #[ignore = "reads the live Windows application sources for revision diagnostics"]
    fn real_inventory_sources_keep_the_revision_stable() {
        let package_sources_before = package_sources::revision_fingerprint();
        let appx_before = appx_repository_revision();
        let before = system_inventory_revision()
            .expect("the initial inventory revision should be available");
        let inventory = system_inventory(&crate::PlatformCancellation::new(|| false))
            .expect("the live inventory should be available");
        let after =
            system_inventory_revision().expect("the final inventory revision should be available");
        let appx_after = appx_repository_revision();
        let package_sources_after = package_sources::revision_fingerprint();

        assert!(
            inventory.installed_applications_complete,
            "the live inventory should be complete"
        );
        assert_eq!(
            package_sources_before, package_sources_after,
            "reading package-manager sources must not mutate their revision evidence"
        );
        assert_eq!(
            appx_before, appx_after,
            "reading AppX sources must not mutate their revision evidence"
        );
        assert_eq!(
            before, after,
            "reading application sources must not mutate the revision evidence"
        );
    }

    #[test]
    fn packaged_application_size_counts_internal_hard_links_once_and_excludes_shared_links() {
        let unique = format!(
            "mangodisk-appx-size-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should follow the Unix epoch")
                .as_nanos()
        );
        let fixture_root = std::env::temp_dir().join(unique);
        let package_root = fixture_root.join("package");
        let external_root = fixture_root.join("external");
        fs::create_dir_all(&package_root).expect("package fixture should be created");
        fs::create_dir_all(&external_root).expect("external fixture should be created");

        let internal_source = package_root.join("internal.bin");
        fs::write(&internal_source, vec![1_u8; 1_024]).expect("internal fixture should be written");
        fs::hard_link(&internal_source, package_root.join("internal-copy.bin"))
            .expect("internal hard link should be created");

        let shared_source = package_root.join("shared.bin");
        fs::write(&shared_source, vec![2_u8; 2_048]).expect("shared fixture should be written");
        fs::hard_link(&shared_source, external_root.join("shared-copy.bin"))
            .expect("external hard link should be created");
        fs::write(package_root.join("regular.bin"), vec![3_u8; 512])
            .expect("regular fixture should be written");

        assert_eq!(
            super::exclusive_directory_logical_size(&package_root),
            Some(1_536)
        );
        fs::remove_dir_all(fixture_root).expect("fixture should be removed");
    }

    #[test]
    #[ignore = "reads the host uninstall registry for an explicit Windows validation report"]
    fn real_inventory_reports_structured_msi_evidence() {
        let inventory = super::system_inventory(&crate::PlatformCancellation::new(|| false))
            .expect("the Windows inventory should be readable");
        let mut current_user = 0_usize;
        let mut machine = 0_usize;
        for application in &inventory.installed_applications {
            let Some(ApplicationUninstallRegistration::WindowsMsi {
                product_code,
                scope,
                ..
            }) = &application.uninstall_registration
            else {
                continue;
            };
            assert_eq!(
                normalize_product_code(product_code).as_deref(),
                Some(product_code.as_str())
            );
            match scope {
                ApplicationInstallScope::CurrentUser => current_user += 1,
                ApplicationInstallScope::Machine => machine += 1,
            }
        }
        eprintln!(
            "windows_msi_inventory total={} current_user={} machine={}",
            current_user + machine,
            current_user,
            machine
        );
    }

    #[test]
    #[ignore = "reads the current Windows user's package registrations"]
    fn real_inventory_validates_discovered_appx_evidence() {
        let inventory = super::system_inventory(&crate::PlatformCancellation::new(|| false))
            .expect("the Windows inventory should be readable");
        let registrations = inventory
            .installed_applications
            .iter()
            .filter_map(|application| application.uninstall_registration.as_ref())
            .filter(|registration| {
                matches!(
                    registration,
                    ApplicationUninstallRegistration::WindowsAppx { .. }
                )
            })
            .collect::<Vec<_>>();

        for (index, registration) in registrations.iter().enumerate() {
            if let ApplicationUninstallRegistration::WindowsAppx {
                package_full_name, ..
            } = registration
            {
                eprintln!("windows_appx_validation index={index} package={package_full_name}");
            }
            assert_eq!(
                super::native_uninstall::registration_state(registration)
                    .expect("the AppX registration should be queryable"),
                ApplicationUninstallRegistrationState::Installed
            );
        }
        eprintln!("windows_appx_inventory actionable={}", registrations.len());
    }

    #[test]
    #[ignore = "reads the host application catalog for an explicit Windows validation report"]
    fn real_inventory_reports_visible_application_metadata() {
        let inventory = super::system_inventory(&crate::PlatformCancellation::new(|| false))
            .expect("the Windows inventory should be readable");
        let applications = &inventory.installed_applications;
        let total_bytes = applications.iter().fold(0_u64, |total, application| {
            total.saturating_add(application.estimated_bytes)
        });
        let sized = applications
            .iter()
            .filter(|application| application.estimated_bytes > 0)
            .count();
        let icon_sources = applications
            .iter()
            .filter(|application| application.icon_path.is_some())
            .count();
        eprintln!(
            "windows_application_inventory total={} sized={} icon_sources={} total_bytes={}",
            applications.len(),
            sized,
            icon_sources,
            total_bytes
        );
        assert!(!applications.is_empty());
        assert!(sized > 0);
        assert!(icon_sources > 0);
    }
}
