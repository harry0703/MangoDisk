use std::path::PathBuf;
use std::{error::Error, fmt};

use crate::command::ControlledExecutable;

/// Exact logical-size aggregate required by the application uninstall catalog.
///
/// This deliberately excludes path snapshots and fingerprints. Catalog scans need current bytes,
/// ordinary-file counts, and completeness, while uninstall planning rebuilds the stronger safety
/// snapshot separately. Keeping the products distinct prevents list rendering from paying the
/// hashing and deterministic-sort cost required only at the mutation boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationComponentAggregate {
    pub bytes: u64,
    pub file_count: u64,
    pub skipped_count: u64,
    pub strategy: &'static str,
}

/// Cancellation is distinct from native enumeration failure so Core never starts a portable
/// second pass after the user has already cancelled the application scan.
#[derive(Debug)]
pub enum ApplicationComponentAggregateError {
    Cancelled,
    Platform(String),
}

/// Stable system facts used to build the cross-platform application catalog.
/// This contract contains no cleanup selection, recommendation, or UI text.
#[derive(Debug, Clone, Default)]
pub struct SystemInventory {
    pub installed_applications: Vec<InstalledApplication>,
    pub installed_applications_complete: bool,
    pub developer_tools: Vec<DetectedTool>,
    pub developer_tools_complete: bool,
    pub filesystem_kinds: Vec<String>,
    pub filesystem_complete: bool,
    pub capabilities: Vec<String>,
    pub capabilities_complete: bool,
    pub os_version: String,
}

#[derive(Debug, Clone)]
pub struct InstalledApplication {
    /// Stable catalog identity assigned by the platform inventory.
    ///
    /// This identity represents the catalog record, not one of its package
    /// sources. Enriching an application with another package source must
    /// never change selection state or invalidate an unchanged uninstall plan.
    pub catalog_identifier: String,
    /// Primary platform identity for the application.
    ///
    /// macOS uses the main bundle identifier. Windows uses the uninstall
    /// registry key because display names and publishers are not unique.
    pub primary_identifier: String,
    /// Stable application identities owned by the main bundle and its nested
    /// helpers, login items, app extensions, and XPC services.
    ///
    /// Core must compare leftovers against every component identity. A main
    /// bundle identifier alone is insufficient because a nested component may
    /// remain the valid owner of a sandbox container.
    pub identifiers: Vec<String>,
    /// Exact identities reported by every inventory source that owns this
    /// catalog record.
    ///
    /// These facts support cross-source deduplication and presentation. They
    /// never authorize uninstall execution or filesystem deletion by
    /// themselves.
    pub source_identities: Vec<ApplicationSourceIdentity>,
    /// Whether Windows reports a package signed as part of the operating system.
    /// This is display evidence only; it does not grant or restrict uninstall authority.
    #[cfg(windows)]
    pub system_signed: bool,
    pub name: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
    /// Platform-derived estimate of removable application bytes.
    ///
    /// macOS uses Spotlight's indexed bundle size. Traditional Windows
    /// applications use the uninstall registration's EstimatedSize value.
    /// Packaged Windows applications count package-local hard links once and
    /// exclude allocations that are still linked outside the package. A zero
    /// value means that the platform did not provide a trustworthy estimate;
    /// Core must not recursively walk every application merely to populate the
    /// catalog.
    pub estimated_bytes: u64,
    /// Last time the operating system observed the application being used.
    ///
    /// This fact is optional because Windows uninstall registrations do not
    /// expose a reliable cross-installer equivalent.
    pub last_used_at_ms: Option<u64>,
    /// Installation or last-update date reported by the operating system.
    ///
    /// Windows uninstall registrations commonly expose an `InstallDate`
    /// without a reliable last-used timestamp. Keeping the facts separate
    /// lets adapters present the platform-appropriate date without changing
    /// the meaning of either value.
    pub installed_at_ms: Option<u64>,
    /// Native icon source resolved by the platform inventory.
    ///
    /// macOS keeps the bundle as the source. Windows may expose an executable,
    /// ICO/PNG resource, or packaged-application directory. Adapters decode
    /// this source lazily and must not put image bytes in the catalog.
    pub icon_path: Option<PathBuf>,
    /// Platform-native application container when one exists.
    ///
    /// macOS uses the bundle path to keep binary maintenance inside the
    /// discovered `.app`. Windows uses a verified package or registered
    /// installation directory when one is available. This path is display
    /// metadata only and never authorizes deletion or native uninstall.
    pub bundle_path: Option<PathBuf>,
    pub executable_paths: Vec<PathBuf>,
    /// Structured native uninstall registration discovered by the platform.
    ///
    /// Free-form command strings are intentionally excluded. Core may only
    /// create an executable uninstall plan from typed evidence whose identity
    /// and install scope can be revalidated by the platform.
    pub uninstall_registration: Option<ApplicationUninstallRegistration>,
    /// Explains why no executable registration was accepted. The same typed reason is shown in
    /// the catalog and logged with its redacted application ID; raw commands never leave Windows.
    pub uninstall_diagnostic: Option<ApplicationUninstallDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallDiagnostic {
    CommandMissing,
    CommandUnreadable,
    InvalidCommand,
    RelativeExecutable,
    UnresolvedEnvironment,
    ExecutableMissing,
    ExecutableAccessDenied,
    ExecutableProbeFailed,
    InvalidExecutable,
    UnsupportedCommandHost,
    RegistrationConflict,
}

impl ApplicationUninstallDiagnostic {
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::CommandMissing => "command_missing",
            Self::CommandUnreadable => "command_unreadable",
            Self::InvalidCommand => "invalid_command",
            Self::RelativeExecutable => "relative_executable",
            Self::UnresolvedEnvironment => "unresolved_environment",
            Self::ExecutableMissing => "executable_missing",
            Self::ExecutableAccessDenied => "executable_access_denied",
            Self::ExecutableProbeFailed => "executable_probe_failed",
            Self::InvalidExecutable => "invalid_executable",
            Self::UnsupportedCommandHost => "unsupported_command_host",
            Self::RegistrationConflict => "registration_conflict",
        }
    }
}

/// Shared by inventory diagnostics, Core plans, and UI interaction logs. Hashing keeps
/// private registry/package identifiers out of logs without losing cross-stage correlation.
pub fn application_uninstall_diagnostic_id(catalog_identifier: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-application-uninstall-v2");
    hasher.update(catalog_identifier.as_bytes());
    format!("application-{}", &hasher.finalize().to_hex()[..24])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApplicationInventorySource {
    MacosBundle,
    WindowsRegistry,
    WindowsMsi,
    WindowsAppx,
    Winget,
    Steam,
    Scoop,
    Chocolatey,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ApplicationSourceIdentity {
    pub source: ApplicationInventorySource,
    pub identifier: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApplicationInstallScope {
    CurrentUser,
    Machine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsRegistryView {
    Registry32,
    Registry64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsRegisteredUninstallKind {
    Executable,
    BatchScript,
    Rundll32,
    UserPowerShellScript,
    WingetProduct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationUninstallRegistration {
    WindowsMsi {
        product_code: String,
        scope: ApplicationInstallScope,
        estimated_bytes: u64,
    },
    WindowsAppx {
        package_family_name: String,
        package_full_name: String,
        estimated_bytes: u64,
    },
    /// A Scoop package whose package marker and Scoop command script were
    /// captured from the same verified installation root.
    ///
    /// Execution revalidates both digests before invoking Scoop. Package
    /// source identity alone is not sufficient uninstall authority.
    WindowsScoop {
        package_name: String,
        scope: ApplicationInstallScope,
        install_root: PathBuf,
        package_marker_digest: String,
        scoop_script_digest: String,
        estimated_bytes: u64,
    },
    /// A Chocolatey package whose package marker and client executable were
    /// captured from the same verified installation root.
    ///
    /// Chocolatey-installed MSI applications must be removed through
    /// Chocolatey rather than invoking MSI directly. Otherwise the program
    /// disappears while Chocolatey's package database still reports it as
    /// installed. Execution revalidates the package marker and executable
    /// identity before launching the exact package ID.
    WindowsChocolatey {
        package_name: String,
        install_root: PathBuf,
        package_marker_digest: String,
        chocolatey_executable: ControlledExecutable,
        estimated_bytes: u64,
    },
    /// A traditional Win32 uninstaller registered with Windows.
    ///
    /// The descriptor stores only registry identity and a digest of the
    /// validated command. Execution reopens the same key and verifies that the
    /// command is unchanged. Executables launch through the default Windows
    /// Shell policy so their own manifest decides whether UAC is required;
    /// MangoDisk never infers elevation from registry scope, publisher, or
    /// installation path. No command interpreter is used, and raw command text
    /// never crosses the platform boundary.
    WindowsRegistered {
        key_name: String,
        scope: ApplicationInstallScope,
        registry_view: WindowsRegistryView,
        command_kind: WindowsRegisteredUninstallKind,
        command_digest: String,
        estimated_bytes: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationUninstallRegistrationState {
    Installed,
    Absent,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationUninstallExecutionOutcome {
    Completed,
    RestartRequired,
}

/// Result of one macOS administrator-authorized bundle removal.
///
/// The privileged boundary is intentionally limited to validated bundles in
/// the system or current user's Applications directory. Product selection,
/// bundle discovery, and associated-data cleanup remain outside this contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacosPrivilegedApplicationRemovalOutcome {
    Completed,
    UserCancelled,
    ItemChanged,
    /// The bundle may have been moved or partially removed and requires a
    /// deliberate recovery check instead of an ordinary retry.
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationUninstallPlatformError {
    Unsupported,
    RequiresElevation,
    UserCancelled,
    RegistrationChanged,
    NativeFailure(u32),
    /// The vendor failed, but postflight verified that this exact registration is absent.
    NativeFailureAfterRemoval(u32),
}

impl ApplicationUninstallPlatformError {
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::RequiresElevation => "requires_elevation",
            Self::UserCancelled => "user_cancelled",
            Self::RegistrationChanged => "registration_changed",
            Self::NativeFailure(_) => "native_failure",
            Self::NativeFailureAfterRemoval(_) => "native_failure_after_removal",
        }
    }

    pub const fn native_code(self) -> Option<u32> {
        match self {
            Self::NativeFailure(code) | Self::NativeFailureAfterRemoval(code) => Some(code),
            _ => None,
        }
    }
}

impl fmt::Display for ApplicationUninstallPlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_code())
    }
}

impl Error for ApplicationUninstallPlatformError {}

#[derive(Debug, Clone)]
pub struct DetectedTool {
    pub name: String,
    pub executable: ControlledExecutable,
}

/// Missing registered paths are evidence only when every existing ancestor is a plain directory
/// on an accessible volume. Share this fact between catalog classification and removal preflight
/// so permission failures, offline roots, and reparse points never become deletion authority.
pub fn registered_application_path_is_missing(path: &std::path::Path) -> bool {
    use std::io::ErrorKind;
    if !path.is_absolute()
        || !std::fs::symlink_metadata(path).is_err_and(|error| error.kind() == ErrorKind::NotFound)
    {
        return false;
    }
    let mut root_is_accessible = false;
    for ancestor in path.ancestors().skip(1) {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                #[cfg(windows)]
                let link_like = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes()
                        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                        != 0
                };
                #[cfg(not(windows))]
                let link_like = metadata.file_type().is_symlink();
                if !metadata.is_dir() || link_like {
                    return false;
                }
                root_is_accessible = ancestor.parent().is_none();
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => return false,
        }
    }
    root_is_accessible
}
