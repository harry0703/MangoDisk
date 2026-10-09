use std::ffi::OsString;
use std::fs;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::{
    FOLDERID_CommonStartup, FOLDERID_Startup, IShellLinkW, SHGetKnownFolderPath, ShellLink,
    KF_FLAG_DEFAULT, SLGP_RAWPATH,
};
use windows_core::{Interface, HSTRING, PWSTR};
use winreg::{
    enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, REG_BINARY},
    RegKey, RegValue,
};

use crate::{
    PlatformCancellation, PlatformError, PlatformErrorCode, PlatformResult,
    PlatformStartupArtifact, PlatformStartupChangeRequest, PlatformStartupChangeResult,
    PlatformStartupConfiguredState, PlatformStartupControlCapability,
    PlatformStartupCoverageReason, PlatformStartupCoverageStatus, PlatformStartupDesiredState,
    PlatformStartupDiagnosticCode, PlatformStartupIdentityConfidence, PlatformStartupOwner,
    PlatformStartupRuntimeState, PlatformStartupScope, PlatformStartupSourceKind,
    PlatformStartupSourceResult, PlatformStartupSummarySource, PlatformStartupTarget,
    PlatformStartupTrigger,
};

use super::metadata::{
    file_version_metadata, filesystem_target_state, startup_trust, FilesystemTargetState,
};
use super::registry::{
    changed_startup_approved_bytes, desired_configured_state, modified_at_ms, normalized_path,
    split_command_line, startup_approved_state, target_kind,
};

const STARTUP_APPROVED_FOLDER_PATH: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder";
const BUFFER_LENGTH: usize = 32_768;

struct ComGuard;

impl ComGuard {
    fn initialize() -> windows_core::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

#[derive(Clone, Copy)]
struct FolderSource {
    source_id: &'static str,
    folder_id: &'static windows_core::GUID,
    scope: PlatformStartupScope,
    approval_root: winreg::HKEY,
    control_capability: PlatformStartupControlCapability,
}

struct LinkTarget {
    path: PathBuf,
    arguments: Vec<String>,
    description: Option<String>,
}

pub(super) fn scan(cancellation: &PlatformCancellation) -> Vec<PlatformStartupSourceResult> {
    let sources = folder_sources();
    let Ok(_com) = ComGuard::initialize() else {
        return sources
            .into_iter()
            .map(|source| {
                PlatformStartupSourceResult::unavailable(
                    source.source_id,
                    true,
                    PlatformStartupCoverageReason::ApiUnavailable,
                )
            })
            .collect();
    };
    sources
        .into_iter()
        .map(|source| scan_folder(source, cancellation))
        .collect()
}

fn scan_folder(
    source: FolderSource,
    cancellation: &PlatformCancellation,
) -> PlatformStartupSourceResult {
    let started = Instant::now();
    let folder = match known_folder_path(source.folder_id) {
        Ok(folder) => folder,
        Err(_) => {
            return result(
                source.source_id,
                Vec::new(),
                PlatformStartupCoverageStatus::Unavailable,
                Some(PlatformStartupCoverageReason::ApiUnavailable),
                started,
            );
        }
    };
    let approval = RegKey::predef(source.approval_root)
        .open_subkey_with_flags(STARTUP_APPROVED_FOLDER_PATH, KEY_READ)
        .ok();
    let entries = match fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return result(
                source.source_id,
                Vec::new(),
                PlatformStartupCoverageStatus::Complete,
                None,
                started,
            );
        }
        Err(error) => {
            let reason = if error.kind() == std::io::ErrorKind::PermissionDenied {
                PlatformStartupCoverageReason::AccessDenied
            } else {
                PlatformStartupCoverageReason::InvalidData
            };
            return result(
                source.source_id,
                Vec::new(),
                PlatformStartupCoverageStatus::Unavailable,
                Some(reason),
                started,
            );
        }
    };
    let mut items = Vec::new();
    let mut partial_reason = None;
    for entry in entries {
        if cancellation.is_cancelled() {
            return result(
                source.source_id,
                items,
                PlatformStartupCoverageStatus::Cancelled,
                Some(PlatformStartupCoverageReason::Cancelled),
                started,
            );
        }
        let Ok(entry) = entry else {
            partial_reason.get_or_insert(PlatformStartupCoverageReason::InvalidData);
            continue;
        };
        let path = entry.path();
        if is_startup_folder_metadata(&path) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => {
                partial_reason.get_or_insert(PlatformStartupCoverageReason::InvalidData);
                continue;
            }
        };
        if file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        items.push(folder_artifact(source, &path, approval.as_ref()));
    }
    result(
        source.source_id,
        items,
        if partial_reason.is_some() {
            PlatformStartupCoverageStatus::Partial
        } else {
            PlatformStartupCoverageStatus::Complete
        },
        partial_reason,
        started,
    )
}

pub(super) fn change(
    request: &PlatformStartupChangeRequest,
) -> PlatformResult<PlatformStartupChangeResult> {
    let _com = ComGuard::initialize().map_err(|error| {
        PlatformError::new(
            PlatformErrorCode::OperationFailed,
            format!("initialize startup folder COM access: {error}"),
        )
    })?;
    let (source, source_path, current) = find_item(&request.provider_item_id)?;
    if current != request.expected_artifact {
        return Err(PlatformError::item_changed(
            "startup folder item changed after preflight",
        ));
    }
    if request.desired_state == PlatformStartupDesiredState::Removed {
        return remove_startup_shortcut(source, &source_path, &current);
    }
    if !matches!(
        current.control_capability,
        PlatformStartupControlCapability::Toggleable
            | PlatformStartupControlCapability::ElevationRequired
    ) {
        return Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "startup folder item is not toggleable",
        ));
    }
    let file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| PlatformError::item_changed("startup folder item name is unavailable"))?;
    let root = RegKey::predef(source.approval_root);
    let (approval, _) = root
        .create_subkey_with_flags(STARTUP_APPROVED_FOLDER_PATH, KEY_READ | KEY_SET_VALUE)
        .map_err(|error| PlatformError::io("open startup folder approval state", &error))?;
    let desired = desired_configured_state(request.desired_state);
    let existing = approval.get_raw_value(file_name).ok();
    if current.configured_state != desired {
        let bytes = changed_startup_approved_bytes(existing.as_ref(), request.desired_state)?;
        approval
            .set_raw_value(
                file_name,
                &RegValue {
                    bytes: bytes.into(),
                    vtype: REG_BINARY,
                },
            )
            .map_err(|error| PlatformError::io("write startup folder approval state", &error))?;
    }
    let verified = folder_artifact(source, &source_path, Some(&approval));
    Ok(PlatformStartupChangeResult {
        previous_state: current.configured_state,
        configured_state: verified.configured_state,
        verified: verified.configured_state == desired,
    })
}

fn remove_startup_shortcut(
    source: FolderSource,
    source_path: &Path,
    current: &PlatformStartupArtifact,
) -> PlatformResult<PlatformStartupChangeResult> {
    let expected_folder = known_folder_path(source.folder_id).map_err(|error| {
        PlatformError::new(
            PlatformErrorCode::OperationFailed,
            format!("resolve startup folder: {error}"),
        )
    })?;
    let is_shortcut = source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"));
    let metadata = fs::symlink_metadata(source_path)
        .map_err(|error| PlatformError::io("inspect startup shortcut for removal", &error))?;
    if source_path.parent() != Some(expected_folder.as_path())
        || !is_shortcut
        || !metadata.is_file()
        || metadata.file_type().is_symlink()
    {
        return Err(PlatformError::invalid_path(
            "startup shortcut is outside the removable source boundary",
        ));
    }

    fs::remove_file(source_path)
        .map_err(|error| PlatformError::io("remove startup shortcut", &error))?;
    if let Some(file_name) = source_path.file_name().and_then(|name| name.to_str()) {
        let root = RegKey::predef(source.approval_root);
        if let Ok(approval) =
            root.open_subkey_with_flags(STARTUP_APPROVED_FOLDER_PATH, KEY_SET_VALUE)
        {
            let _ = approval.delete_value(file_name);
        }
    }
    let verified = matches!(
        fs::symlink_metadata(source_path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    );
    Ok(PlatformStartupChangeResult {
        previous_state: current.configured_state,
        configured_state: PlatformStartupConfiguredState::NotApplicable,
        verified,
    })
}

fn find_item(
    provider_item_id: &str,
) -> PlatformResult<(FolderSource, PathBuf, PlatformStartupArtifact)> {
    for source in folder_sources() {
        let folder = known_folder_path(source.folder_id).map_err(|error| {
            PlatformError::new(
                PlatformErrorCode::OperationFailed,
                format!("resolve startup folder: {error}"),
            )
        })?;
        let root = RegKey::predef(source.approval_root);
        let approval = root
            .open_subkey_with_flags(STARTUP_APPROVED_FOLDER_PATH, KEY_READ)
            .ok();
        let entries = match fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(PlatformError::io("read startup folder", &error)),
        };
        for entry in entries {
            let entry =
                entry.map_err(|error| PlatformError::io("read startup folder item", &error))?;
            let path = entry.path();
            if is_startup_folder_metadata(&path) {
                continue;
            }
            let file_type = entry
                .file_type()
                .map_err(|error| PlatformError::io("inspect startup folder item", &error))?;
            if file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let artifact = folder_artifact(source, &path, approval.as_ref());
            if artifact.provider_item_id == provider_item_id {
                return Ok((source, path, artifact));
            }
        }
    }
    Err(PlatformError::item_changed(
        "startup folder item no longer exists",
    ))
}

fn folder_sources() -> [FolderSource; 2] {
    [
        FolderSource {
            source_id: "windows.startup_folder.user",
            folder_id: &FOLDERID_Startup,
            scope: PlatformStartupScope::CurrentUser,
            approval_root: HKEY_CURRENT_USER,
            control_capability: PlatformStartupControlCapability::Toggleable,
        },
        FolderSource {
            source_id: "windows.startup_folder.common",
            folder_id: &FOLDERID_CommonStartup,
            scope: PlatformStartupScope::AllUsers,
            approval_root: HKEY_LOCAL_MACHINE,
            control_capability: PlatformStartupControlCapability::ElevationRequired,
        },
    ]
}

fn is_startup_folder_metadata(path: &Path) -> bool {
    // Explorer keeps desktop.ini in both known startup folders to localize their display names.
    // The file is shell metadata rather than a logon payload, even though it is a regular file.
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("desktop.ini"))
}

fn folder_artifact(
    source: FolderSource,
    source_path: &Path,
    approval: Option<&RegKey>,
) -> PlatformStartupArtifact {
    let file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Unnamed startup item");
    let display_name = source_path
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(file_name)
        .to_string();
    let is_link = source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"));
    let link_target = is_link.then(|| resolve_link(source_path)).transpose();
    let (target_path, arguments, description, mut diagnostics) = match link_target {
        Ok(Some(target)) => (
            target.path,
            target.arguments,
            target.description,
            Vec::new(),
        ),
        Ok(None) => (source_path.to_path_buf(), Vec::new(), None, Vec::new()),
        Err(_) => (
            source_path.to_path_buf(),
            Vec::new(),
            None,
            vec![PlatformStartupDiagnosticCode::UnsupportedFormat],
        ),
    };
    if filesystem_target_state(&target_path) == FilesystemTargetState::Missing {
        diagnostics.push(PlatformStartupDiagnosticCode::MissingTarget);
    }
    let configured_state = approval
        .and_then(|key| key.get_raw_value(file_name).ok())
        .map(|value| startup_approved_state(&value.bytes))
        .unwrap_or(PlatformStartupConfiguredState::Enabled);
    let identity = normalized_path(&target_path);
    let version_metadata = file_version_metadata(&target_path).unwrap_or_default();
    let summary = description.or(version_metadata.description);
    let summary_source = if summary.is_some() {
        if version_metadata.product_name.is_some()
            || version_metadata.company_name.is_some()
            || version_metadata.product_version.is_some()
        {
            PlatformStartupSummarySource::VersionInfo
        } else {
            PlatformStartupSummarySource::SourceLabel
        }
    } else {
        PlatformStartupSummarySource::Unavailable
    };
    PlatformStartupArtifact {
        provider_item_id: format!(
            "startup-folder:{}:{}",
            source.source_id,
            normalized_path(source_path)
        ),
        source_kind: PlatformStartupSourceKind::StartupFolder,
        scope: source.scope,
        triggers: vec![PlatformStartupTrigger::UserLogon],
        display_name: display_name.clone(),
        configuration_path: Some(source_path.to_path_buf()),
        target: PlatformStartupTarget {
            kind: target_kind(Some(&target_path)),
            identity_key: format!("path:{identity}"),
            path: Some(target_path.clone()),
            executable_name: target_path
                .file_name()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned),
            arguments,
        },
        owner: PlatformStartupOwner {
            identity_key: Some(format!("path:{identity}")),
            name: version_metadata.product_name.or(Some(display_name)),
            publisher: version_metadata.company_name,
            summary,
            summary_source,
            version: version_metadata.product_version,
            icon_path: Some(target_path.clone()).filter(|path| path.exists()),
            confidence: PlatformStartupIdentityConfidence::Strong,
        },
        configured_state,
        runtime_state: PlatformStartupRuntimeState::Unknown,
        control_capability: source.control_capability,
        trust: startup_trust(Some(&target_path), identity.starts_with("c:\\windows\\")),
        modified_at_ms: modified_at_ms(source_path),
        diagnostics,
    }
}

fn resolve_link(path: &Path) -> windows_core::Result<LinkTarget> {
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)? };
    let persist: IPersistFile = link.cast()?;
    unsafe {
        persist.Load(&HSTRING::from(path.as_os_str()), STGM_READ)?;
    }
    let mut target = vec![0u16; BUFFER_LENGTH];
    let mut find_data = WIN32_FIND_DATAW::default();
    unsafe {
        link.GetPath(&mut target, &mut find_data, SLGP_RAWPATH.0 as u32)?;
    }
    let mut argument_buffer = vec![0u16; BUFFER_LENGTH];
    unsafe {
        link.GetArguments(&mut argument_buffer)?;
    }
    let mut description_buffer = vec![0u16; 1024];
    let description = unsafe { link.GetDescription(&mut description_buffer) }
        .ok()
        .and_then(|()| wide_buffer_string(&description_buffer));
    let target = wide_buffer_string(&target).ok_or_else(windows_core::Error::from_thread)?;
    let arguments = wide_buffer_string(&argument_buffer)
        .map(|arguments| split_command_line(&arguments))
        .unwrap_or_default();
    Ok(LinkTarget {
        path: PathBuf::from(target),
        arguments,
        description,
    })
}

fn known_folder_path(folder_id: &windows_core::GUID) -> windows_core::Result<PathBuf> {
    let path: PWSTR = unsafe { SHGetKnownFolderPath(folder_id, KF_FLAG_DEFAULT, None)? };
    let mut length = 0;
    unsafe {
        while *path.0.add(length) != 0 {
            length += 1;
        }
    }
    let value = unsafe { OsString::from_wide(std::slice::from_raw_parts(path.0, length)) };
    unsafe {
        CoTaskMemFree(Some(path.0.cast()));
    }
    Ok(PathBuf::from(value))
}

fn wide_buffer_string(buffer: &[u16]) -> Option<String> {
    let end = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..end]);
    (!value.trim().is_empty()).then_some(value)
}

fn result(
    source_id: &str,
    items: Vec<PlatformStartupArtifact>,
    status: PlatformStartupCoverageStatus,
    reason: Option<PlatformStartupCoverageReason>,
    started: Instant,
) -> PlatformStartupSourceResult {
    PlatformStartupSourceResult {
        source_id: source_id.to_string(),
        required: true,
        status,
        reason,
        items,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::{
        change, folder_artifact, folder_sources, is_startup_folder_metadata, known_folder_path,
        wide_buffer_string, ComGuard, STARTUP_APPROVED_FOLDER_PATH,
    };
    use crate::{
        PlatformStartupChangeRequest, PlatformStartupConfiguredState,
        PlatformStartupControlCapability, PlatformStartupDesiredState,
        PlatformStartupDiagnosticCode,
    };
    use windows::Win32::{
        System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER},
        UI::Shell::{IShellLinkW, ShellLink},
    };
    use windows_core::{Interface, HSTRING};
    use winreg::{
        enums::{KEY_READ, KEY_SET_VALUE},
        RegKey, RegValue,
    };

    #[test]
    fn desktop_ini_is_excluded_case_insensitively() {
        assert!(is_startup_folder_metadata(Path::new(
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Startup\desktop.ini"
        )));
        assert!(is_startup_folder_metadata(Path::new(
            r"C:\Users\fixture\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\DESKTOP.INI"
        )));
        assert!(!is_startup_folder_metadata(Path::new(
            r"C:\Users\fixture\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\Desktop.lnk"
        )));
    }

    #[test]
    fn wide_buffer_ignores_unused_capacity() {
        let buffer = [b'a' as u16, b'b' as u16, 0, b'c' as u16];

        assert_eq!(wide_buffer_string(&buffer).as_deref(), Some("ab"));
    }

    #[test]
    #[ignore = "toggles and removes an isolated shortcut in the current user's Startup folder"]
    fn actual_shortcut_toggle_and_removal_are_verified() {
        let _com = ComGuard::initialize().expect("COM must initialize for the shortcut fixture");
        let source = folder_sources()[0];
        let folder = known_folder_path(source.folder_id)
            .expect("the current user's Startup folder must be available");
        let file_name = format!("MangoDiskToggleFixture{}.lnk", std::process::id());
        let link_path = folder.join(&file_name);
        let root = RegKey::predef(source.approval_root);
        let (approval, _) = root
            .create_subkey_with_flags(STARTUP_APPROVED_FOLDER_PATH, KEY_READ | KEY_SET_VALUE)
            .expect("the StartupApproved fixture key must be writable");
        let _cleanup = ShortcutToggleFixtureGuard {
            path: link_path.clone(),
            approval,
            file_name: file_name.clone(),
            previous_approval: root
                .open_subkey_with_flags(STARTUP_APPROVED_FOLDER_PATH, KEY_READ)
                .ok()
                .and_then(|key| key.get_raw_value(&file_name).ok()),
        };
        let _ = _cleanup.approval.delete_value(&file_name);

        let link: IShellLinkW = unsafe {
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .expect("the ShellLink fixture must be created")
        };
        unsafe {
            link.SetPath(&HSTRING::from(r"C:\Windows\System32\notepad.exe"))
                .expect("the shortcut target must be configured");
        }
        let persist: IPersistFile = link
            .cast()
            .expect("the shortcut fixture must support persistence");
        unsafe {
            persist
                .Save(&HSTRING::from(link_path.as_os_str()), true)
                .expect("the shortcut fixture must be written");
        }

        let enabled = folder_artifact(source, &link_path, Some(&_cleanup.approval));
        assert_eq!(
            enabled.control_capability,
            PlatformStartupControlCapability::Toggleable
        );
        let disabled = change(&PlatformStartupChangeRequest {
            provider_item_id: enabled.provider_item_id.clone(),
            source_id: source.source_id.to_owned(),
            expected_artifact: enabled,
            desired_state: PlatformStartupDesiredState::Disabled,
        })
        .expect("the shortcut fixture must be disabled");
        assert!(disabled.verified);
        assert_eq!(
            disabled.configured_state,
            PlatformStartupConfiguredState::Disabled
        );

        let disabled = folder_artifact(source, &link_path, Some(&_cleanup.approval));
        let enabled = change(&PlatformStartupChangeRequest {
            provider_item_id: disabled.provider_item_id.clone(),
            source_id: source.source_id.to_owned(),
            expected_artifact: disabled,
            desired_state: PlatformStartupDesiredState::Enabled,
        })
        .expect("the shortcut fixture must be enabled again");
        assert!(enabled.verified);
        assert_eq!(
            enabled.configured_state,
            PlatformStartupConfiguredState::Enabled
        );

        let enabled = folder_artifact(source, &link_path, Some(&_cleanup.approval));
        let disabled = change(&PlatformStartupChangeRequest {
            provider_item_id: enabled.provider_item_id.clone(),
            source_id: source.source_id.to_owned(),
            expected_artifact: enabled,
            desired_state: PlatformStartupDesiredState::Disabled,
        })
        .expect("the shortcut fixture must support disabling again before removal");
        assert!(disabled.verified);

        let current = folder_artifact(source, &link_path, Some(&_cleanup.approval));
        let removed = change(&PlatformStartupChangeRequest {
            provider_item_id: current.provider_item_id.clone(),
            source_id: source.source_id.to_owned(),
            expected_artifact: current,
            desired_state: PlatformStartupDesiredState::Removed,
        })
        .expect("the shortcut fixture must be removed");
        assert!(removed.verified);
        assert_eq!(
            removed.configured_state,
            PlatformStartupConfiguredState::NotApplicable
        );
        assert!(!link_path.exists());
        assert!(_cleanup.approval.get_raw_value(&file_name).is_err());
    }

    #[test]
    #[ignore = "creates and removes an isolated shortcut in the current user's Startup folder"]
    fn actual_orphan_shortcut_removal_deletes_only_the_fixture() {
        let _com = ComGuard::initialize().expect("COM must initialize for the shortcut fixture");
        let source = folder_sources()[0];
        let folder = known_folder_path(source.folder_id)
            .expect("the current user's Startup folder must be available");
        let suffix = format!("{}", std::process::id());
        let link_path = folder.join(format!("MangoDiskOrphanFixture{suffix}.lnk"));
        let target_path = folder.join(format!("MangoDiskMissingTarget{suffix}.exe"));
        assert!(!target_path.exists());
        let _cleanup = ShortcutFixtureGuard(link_path.clone());

        let link: IShellLinkW = unsafe {
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .expect("the ShellLink fixture must be created")
        };
        unsafe {
            link.SetPath(&HSTRING::from(target_path.as_os_str()))
                .expect("the shortcut target must be configured");
        }
        let persist: IPersistFile = link
            .cast()
            .expect("the shortcut fixture must support persistence");
        unsafe {
            persist
                .Save(&HSTRING::from(link_path.as_os_str()), true)
                .expect("the shortcut fixture must be written");
        }

        let orphan = folder_artifact(source, &link_path, None);
        assert!(orphan
            .diagnostics
            .contains(&PlatformStartupDiagnosticCode::MissingTarget));
        let removed = change(&PlatformStartupChangeRequest {
            provider_item_id: orphan.provider_item_id.clone(),
            source_id: source.source_id.to_owned(),
            expected_artifact: orphan,
            desired_state: PlatformStartupDesiredState::Removed,
        })
        .expect("the orphaned shortcut fixture must be removed");

        assert!(removed.verified);
        assert_eq!(
            removed.configured_state,
            PlatformStartupConfiguredState::NotApplicable
        );
        assert!(!link_path.exists());
    }

    struct ShortcutFixtureGuard(std::path::PathBuf);

    struct ShortcutToggleFixtureGuard {
        path: std::path::PathBuf,
        approval: RegKey,
        file_name: String,
        previous_approval: Option<RegValue<'static>>,
    }

    impl Drop for ShortcutToggleFixtureGuard {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
            if let Some(previous) = self.previous_approval.as_ref() {
                let _ = self.approval.set_raw_value(&self.file_name, previous);
            } else {
                let _ = self.approval.delete_value(&self.file_name);
            }
        }
    }

    impl Drop for ShortcutFixtureGuard {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
}
