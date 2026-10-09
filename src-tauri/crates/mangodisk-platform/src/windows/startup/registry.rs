use std::env;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::slice;
use std::time::{Instant, UNIX_EPOCH};

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
use winreg::{
    enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_32KEY,
        KEY_WOW64_64KEY, REG_BINARY,
    },
    types::FromRegValue,
    RegKey, RegValue, HKEY,
};

use crate::{
    PlatformCancellation, PlatformError, PlatformErrorCode, PlatformResult,
    PlatformStartupArtifact, PlatformStartupChangeRequest, PlatformStartupChangeResult,
    PlatformStartupConfiguredState, PlatformStartupControlCapability,
    PlatformStartupCoverageReason, PlatformStartupCoverageStatus, PlatformStartupDesiredState,
    PlatformStartupDiagnosticCode, PlatformStartupIdentityConfidence, PlatformStartupOwner,
    PlatformStartupRuntimeState, PlatformStartupScope, PlatformStartupSourceKind,
    PlatformStartupSourceResult, PlatformStartupSummarySource, PlatformStartupTarget,
    PlatformStartupTargetKind, PlatformStartupTrigger,
};

use super::super::path_identity;
use super::metadata::{
    file_version_metadata, filesystem_target_state, startup_trust, FilesystemTargetState,
};

const RUN_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_ONCE_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
const POLICY_RUN_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run";
const STARTUP_APPROVED_PATH: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

#[derive(Clone, Copy)]
struct RegistrySource {
    root: HKEY,
    root_name: &'static str,
    key_path: &'static str,
    view: u32,
    view_name: &'static str,
    scope: PlatformStartupScope,
    approval_bucket: Option<&'static str>,
    control_capability: PlatformStartupControlCapability,
}

pub(super) fn scan(cancellation: &PlatformCancellation) -> PlatformStartupSourceResult {
    let started = Instant::now();
    let mut items = Vec::new();
    let mut partial_reason = None;
    for source in registry_sources() {
        if cancellation.is_cancelled() {
            return source_result(
                items,
                PlatformStartupCoverageStatus::Cancelled,
                Some(PlatformStartupCoverageReason::Cancelled),
                started,
            );
        }
        if let Err(reason) = scan_key(source, &mut items) {
            partial_reason.get_or_insert(reason);
        }
    }
    source_result(
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

fn source_result(
    items: Vec<PlatformStartupArtifact>,
    status: PlatformStartupCoverageStatus,
    reason: Option<PlatformStartupCoverageReason>,
    started: Instant,
) -> PlatformStartupSourceResult {
    PlatformStartupSourceResult {
        source_id: "windows.registry.run".to_string(),
        required: true,
        status,
        reason,
        items,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

fn registry_sources() -> Vec<RegistrySource> {
    let roots_and_views = [
        (
            HKEY_CURRENT_USER,
            "hkcu",
            PlatformStartupScope::CurrentUser,
            PlatformStartupControlCapability::Toggleable,
            KEY_WOW64_64KEY,
            "64",
            "Run",
        ),
        (
            HKEY_LOCAL_MACHINE,
            "hklm",
            PlatformStartupScope::Machine,
            PlatformStartupControlCapability::ElevationRequired,
            KEY_WOW64_64KEY,
            "64",
            "Run",
        ),
        (
            HKEY_LOCAL_MACHINE,
            "hklm",
            PlatformStartupScope::Machine,
            PlatformStartupControlCapability::ElevationRequired,
            KEY_WOW64_32KEY,
            "32",
            "Run32",
        ),
    ];
    roots_and_views
        .into_iter()
        .flat_map(
            |(root, root_name, scope, control_capability, view, view_name, approval_bucket)| {
                [
                    RegistrySource {
                        root,
                        root_name,
                        key_path: RUN_PATH,
                        view,
                        view_name,
                        scope,
                        approval_bucket: Some(approval_bucket),
                        control_capability,
                    },
                    RegistrySource {
                        root,
                        root_name,
                        key_path: RUN_ONCE_PATH,
                        view,
                        view_name,
                        scope,
                        approval_bucket: None,
                        control_capability: PlatformStartupControlCapability::RemoveOnly,
                    },
                    RegistrySource {
                        root,
                        root_name,
                        key_path: POLICY_RUN_PATH,
                        view,
                        view_name,
                        scope,
                        approval_bucket: None,
                        control_capability: PlatformStartupControlCapability::PolicyManaged,
                    },
                ]
            },
        )
        .collect()
}

fn scan_key(
    source: RegistrySource,
    items: &mut Vec<PlatformStartupArtifact>,
) -> Result<(), PlatformStartupCoverageReason> {
    let root = RegKey::predef(source.root);
    let key = match root.open_subkey_with_flags(source.key_path, KEY_READ | source.view) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(PlatformStartupCoverageReason::AccessDenied);
        }
        Err(_) => return Err(PlatformStartupCoverageReason::InvalidData),
    };
    let approval_key = source.approval_bucket.and_then(|bucket| {
        root.open_subkey_with_flags(
            format!(r"{STARTUP_APPROVED_PATH}\{bucket}"),
            KEY_READ | source.view,
        )
        .ok()
    });
    for value in key.enum_values() {
        let Ok((value_name, value)) = value else {
            return Err(PlatformStartupCoverageReason::InvalidData);
        };
        let Ok(command) = String::from_reg_value(&value) else {
            continue;
        };
        items.push(artifact_from_value(
            source,
            &value_name,
            &command,
            approval_key.as_ref(),
        ));
    }
    Ok(())
}

pub(super) fn change(
    request: &PlatformStartupChangeRequest,
) -> PlatformResult<PlatformStartupChangeResult> {
    let (source, value_name, current) = find_item(&request.provider_item_id)?;
    if current != request.expected_artifact {
        return Err(PlatformError::item_changed(
            "registry startup item changed after preflight",
        ));
    }
    if request.desired_state == PlatformStartupDesiredState::Removed {
        return remove_startup_value(source, &value_name, &current);
    }
    if !matches!(
        current.control_capability,
        PlatformStartupControlCapability::Toggleable
            | PlatformStartupControlCapability::ElevationRequired
    ) {
        return Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "registry startup item is not toggleable",
        ));
    }
    let bucket = source.approval_bucket.ok_or_else(|| {
        PlatformError::new(
            PlatformErrorCode::Unsupported,
            "registry startup item has no compatible state store",
        )
    })?;
    let root = RegKey::predef(source.root);
    let approval_path = format!(r"{STARTUP_APPROVED_PATH}\{bucket}");
    let (approval_key, _) = root
        .create_subkey_with_flags(approval_path, KEY_READ | KEY_SET_VALUE | source.view)
        .map_err(|error| PlatformError::io("open startup approval state", &error))?;
    let existing = approval_key.get_raw_value(&value_name).ok();
    if existing
        .as_ref()
        .is_some_and(|value| value.vtype != REG_BINARY)
    {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "startup approval state has an unsupported registry type",
        ));
    }
    let desired = desired_configured_state(request.desired_state);
    if current.configured_state != desired {
        match request.desired_state {
            PlatformStartupDesiredState::Enabled => {
                // Absence is Windows' native enabled state for Run entries. Removing the disabled
                // override restores that state without leaving MangoDisk-owned registry residue.
                if let Err(error) = approval_key.delete_value(&value_name) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(PlatformError::io("remove startup approval state", &error));
                    }
                }
            }
            PlatformStartupDesiredState::Disabled => {
                let bytes =
                    changed_startup_approved_bytes(existing.as_ref(), request.desired_state)?;
                approval_key
                    .set_raw_value(
                        &value_name,
                        &RegValue {
                            bytes: bytes.into(),
                            vtype: REG_BINARY,
                        },
                    )
                    .map_err(|error| PlatformError::io("write startup approval state", &error))?;
            }
            PlatformStartupDesiredState::Removed => {
                return Err(PlatformError::new(
                    PlatformErrorCode::Unsupported,
                    "removed startup items do not use approval state bytes",
                ));
            }
        }
    }
    let command_key = root
        .open_subkey_with_flags(source.key_path, KEY_READ | source.view)
        .map_err(|error| PlatformError::io("reopen registry startup item", &error))?;
    let command = command_key
        .get_raw_value(&value_name)
        .ok()
        .and_then(|value| String::from_reg_value(&value).ok())
        .ok_or_else(|| PlatformError::item_changed("registry startup item disappeared"))?;
    let verified = artifact_from_value(source, &value_name, &command, Some(&approval_key));
    Ok(PlatformStartupChangeResult {
        previous_state: current.configured_state,
        configured_state: verified.configured_state,
        verified: verified.configured_state == desired,
    })
}

fn remove_startup_value(
    source: RegistrySource,
    value_name: &str,
    current: &PlatformStartupArtifact,
) -> PlatformResult<PlatformStartupChangeResult> {
    if !matches!(
        current.control_capability,
        PlatformStartupControlCapability::Toggleable
            | PlatformStartupControlCapability::ElevationRequired
            | PlatformStartupControlCapability::RemoveOnly
    ) {
        return Err(PlatformError::item_changed(
            "registry startup item is no longer removable",
        ));
    }

    let root = RegKey::predef(source.root);
    let key = root
        .open_subkey_with_flags(source.key_path, KEY_READ | KEY_SET_VALUE | source.view)
        .map_err(|error| PlatformError::io("open registry startup item for removal", &error))?;
    key.delete_value(value_name)
        .map_err(|error| PlatformError::io("remove registry startup item", &error))?;
    if let Some(bucket) = source.approval_bucket {
        if let Ok(approval) = root.open_subkey_with_flags(
            format!(r"{STARTUP_APPROVED_PATH}\{bucket}"),
            KEY_SET_VALUE | source.view,
        ) {
            let _ = approval.delete_value(value_name);
        }
    }
    let verified = matches!(
        key.get_raw_value(value_name),
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
) -> PlatformResult<(RegistrySource, String, PlatformStartupArtifact)> {
    for source in registry_sources() {
        let root = RegKey::predef(source.root);
        let key = match root.open_subkey_with_flags(source.key_path, KEY_READ | source.view) {
            Ok(key) => key,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(PlatformError::io("read registry startup item", &error)),
        };
        let approval_key = source.approval_bucket.and_then(|bucket| {
            root.open_subkey_with_flags(
                format!(r"{STARTUP_APPROVED_PATH}\{bucket}"),
                KEY_READ | source.view,
            )
            .ok()
        });
        for value in key.enum_values() {
            let (value_name, value) = value
                .map_err(|error| PlatformError::io("enumerate registry startup item", &error))?;
            let Ok(command) = String::from_reg_value(&value) else {
                continue;
            };
            let artifact =
                artifact_from_value(source, &value_name, &command, approval_key.as_ref());
            if artifact.provider_item_id == provider_item_id {
                return Ok((source, value_name, artifact));
            }
        }
    }
    Err(PlatformError::item_changed(
        "registry startup item no longer exists",
    ))
}

pub(super) fn desired_configured_state(
    desired: PlatformStartupDesiredState,
) -> PlatformStartupConfiguredState {
    match desired {
        PlatformStartupDesiredState::Enabled => PlatformStartupConfiguredState::Enabled,
        PlatformStartupDesiredState::Disabled => PlatformStartupConfiguredState::Disabled,
        PlatformStartupDesiredState::Removed => PlatformStartupConfiguredState::NotApplicable,
    }
}

pub(super) fn changed_startup_approved_bytes(
    existing: Option<&RegValue<'_>>,
    desired: PlatformStartupDesiredState,
) -> PlatformResult<Vec<u8>> {
    let mut bytes = existing
        .map(|value| value.bytes.to_vec())
        .unwrap_or_else(|| vec![0; 12]);
    if bytes.len() < 12 {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "startup approval state has an unsupported format",
        ));
    }
    bytes[0] = match (bytes[0], desired) {
        (0x06 | 0x07, PlatformStartupDesiredState::Enabled) => 0x06,
        (0x06 | 0x07, PlatformStartupDesiredState::Disabled) => 0x07,
        (_, PlatformStartupDesiredState::Enabled) => 0x02,
        (_, PlatformStartupDesiredState::Disabled) => 0x03,
        (_, PlatformStartupDesiredState::Removed) => {
            return Err(PlatformError::new(
                PlatformErrorCode::Unsupported,
                "removed startup items do not use approval state bytes",
            ));
        }
    };
    Ok(bytes)
}

fn artifact_from_value(
    source: RegistrySource,
    value_name: &str,
    command: &str,
    approval_key: Option<&RegKey>,
) -> PlatformStartupArtifact {
    let command_parts = split_command_line(command);
    let raw_target = command_parts.first().cloned();
    let target_path = raw_target
        .as_deref()
        .map(expand_environment_variables)
        .map(PathBuf::from);
    let diagnostics = if target_path.as_deref().is_some_and(|path| {
        is_definitive_filesystem_target(path)
            && filesystem_target_state(path) == FilesystemTargetState::Missing
    }) {
        vec![PlatformStartupDiagnosticCode::MissingTarget]
    } else if target_path.is_none() {
        vec![PlatformStartupDiagnosticCode::InvalidData]
    } else {
        Vec::new()
    };
    let configured_state = approval_key
        .and_then(|key| key.get_raw_value(value_name).ok())
        .map(|value| startup_approved_state(&value.bytes))
        .unwrap_or(PlatformStartupConfiguredState::Enabled);
    let executable_name = target_path
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .or_else(|| raw_target.as_deref().and_then(command_file_name));
    let normalized_target = target_path
        .as_deref()
        .map(normalized_path)
        .unwrap_or_else(|| command.trim().to_lowercase());
    let version_metadata = target_path
        .as_deref()
        .and_then(file_version_metadata)
        .unwrap_or_default();
    let metadata_description_available = version_metadata.description.is_some();
    PlatformStartupArtifact {
        provider_item_id: format!(
            "registry:{}:{}:{}:{}",
            source.root_name,
            source.view_name,
            source.key_path.to_lowercase(),
            value_name.to_lowercase()
        ),
        source_kind: PlatformStartupSourceKind::RegistryRun,
        scope: source.scope,
        triggers: vec![PlatformStartupTrigger::UserLogon],
        display_name: if value_name.trim().is_empty() {
            executable_name
                .clone()
                .unwrap_or_else(|| "Unnamed startup item".to_string())
        } else {
            value_name.to_string()
        },
        configuration_path: None,
        target: PlatformStartupTarget {
            kind: target_kind(target_path.as_deref()),
            identity_key: format!("path:{normalized_target}"),
            path: target_path.clone(),
            executable_name,
            arguments: command_parts.into_iter().skip(1).collect(),
        },
        owner: PlatformStartupOwner {
            identity_key: target_path
                .as_deref()
                .map(|path| format!("path:{}", normalized_path(path))),
            name: version_metadata
                .product_name
                .or_else(|| Some(value_name.to_string()).filter(|name| !name.trim().is_empty())),
            publisher: version_metadata.company_name,
            summary: version_metadata.description,
            summary_source: if metadata_description_available {
                PlatformStartupSummarySource::VersionInfo
            } else {
                PlatformStartupSummarySource::SourceLabel
            },
            version: version_metadata.product_version,
            icon_path: target_path.clone().filter(|path| path.exists()),
            confidence: if target_path.is_some() {
                PlatformStartupIdentityConfidence::Strong
            } else {
                PlatformStartupIdentityConfidence::Unresolved
            },
        },
        configured_state,
        runtime_state: PlatformStartupRuntimeState::Unknown,
        control_capability: source.control_capability,
        trust: startup_trust(
            target_path.as_deref(),
            target_path
                .as_deref()
                .is_some_and(|path| normalized_path(path).starts_with("c:\\windows\\")),
        ),
        modified_at_ms: target_path.as_deref().and_then(modified_at_ms),
        diagnostics,
    }
}

fn is_definitive_filesystem_target(path: &Path) -> bool {
    path.is_absolute()
        && matches!(
            target_kind(Some(path)),
            PlatformStartupTargetKind::Executable | PlatformStartupTargetKind::Script
        )
}

pub(super) fn startup_approved_state(bytes: &[u8]) -> PlatformStartupConfiguredState {
    match bytes.first().copied() {
        Some(0x02) | Some(0x06) => PlatformStartupConfiguredState::Enabled,
        Some(0x03) | Some(0x07) => PlatformStartupConfiguredState::Disabled,
        _ => PlatformStartupConfiguredState::Unknown,
    }
}

pub(super) fn split_command_line(command: &str) -> Vec<String> {
    let wide: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
    let mut count = 0;
    // Windows startup commands often depend on subtle quote and backslash escaping rules. The
    // native parser preserves those semantics and avoids a second, incomplete command grammar.
    let argv = unsafe { CommandLineToArgvW(wide.as_ptr(), &mut count) };
    if argv.is_null() || count <= 0 {
        return Vec::new();
    }
    let values = unsafe { slice::from_raw_parts(argv, count as usize) }
        .iter()
        .map(|argument| {
            let argument = *argument;
            let mut length = 0;
            unsafe {
                while *argument.add(length) != 0 {
                    length += 1;
                }
                OsString::from_wide(slice::from_raw_parts(argument, length))
                    .to_string_lossy()
                    .into_owned()
            }
        })
        .collect();
    unsafe {
        LocalFree(argv.cast());
    }
    values
}

pub(super) fn expand_environment_variables(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find('%') {
        output.push_str(&remaining[..start]);
        let after_start = &remaining[start + 1..];
        let Some(end) = after_start.find('%') else {
            output.push_str(&remaining[start..]);
            return output;
        };
        let name = &after_start[..end];
        if let Some(replacement) = env::var_os(name) {
            output.push_str(&replacement.to_string_lossy());
        } else {
            output.push('%');
            output.push_str(name);
            output.push('%');
        }
        remaining = &after_start[end + 1..];
    }
    output.push_str(remaining);
    output
}

pub(super) fn normalized_path(path: &Path) -> String {
    path_identity::comparison_key(path)
}

fn command_file_name(value: &str) -> Option<String> {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
}

pub(super) fn target_kind(path: Option<&Path>) -> PlatformStartupTargetKind {
    match path
        .and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("exe") | Some("com") => PlatformStartupTargetKind::Executable,
        Some("cmd") | Some("bat") | Some("ps1") | Some("vbs") | Some("js") => {
            PlatformStartupTargetKind::Script
        }
        Some(_) => PlatformStartupTargetKind::Other,
        None => PlatformStartupTargetKind::Unknown,
    }
}

pub(super) fn modified_at_ms(path: &Path) -> Option<u64> {
    path.metadata()
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::{
        artifact_from_value, change, changed_startup_approved_bytes, expand_environment_variables,
        find_item, registry_sources, scan, split_command_line, startup_approved_state, RUN_PATH,
        STARTUP_APPROVED_PATH,
    };
    use crate::{
        PlatformStartupConfiguredState, PlatformStartupDesiredState, PlatformStartupDiagnosticCode,
    };
    use winreg::{enums::REG_BINARY, RegKey, RegValue};

    #[test]
    fn quoted_command_preserves_executable_and_arguments() {
        let values =
            split_command_line(r#""C:\Program Files\Example\agent.exe" --silent "two words""#);

        assert_eq!(values[0], r"C:\Program Files\Example\agent.exe");
        assert_eq!(values[1..], ["--silent", "two words"]);
    }

    #[test]
    fn startup_approved_unknown_data_remains_unknown() {
        assert_eq!(
            startup_approved_state(&[0xff, 0, 0, 0]),
            PlatformStartupConfiguredState::Unknown
        );
        assert_eq!(
            startup_approved_state(&[]),
            PlatformStartupConfiguredState::Unknown
        );
    }

    #[test]
    fn unknown_environment_variable_is_preserved() {
        let value = expand_environment_variables(r"%MANGODISK_TEST_UNKNOWN%\agent.exe");

        assert_eq!(value, r"%MANGODISK_TEST_UNKNOWN%\agent.exe");
    }

    #[test]
    fn bare_executable_is_not_reported_as_a_missing_target() {
        let source = registry_sources()
            .into_iter()
            .next()
            .expect("at least one registry source must exist");
        let artifact = artifact_from_value(source, "Fixture", "cmd.exe /c exit", None);

        assert!(!artifact
            .diagnostics
            .contains(&PlatformStartupDiagnosticCode::MissingTarget));
    }

    #[test]
    fn relative_and_unresolved_targets_fail_closed() {
        let source = registry_sources()
            .into_iter()
            .next()
            .expect("at least one registry source must exist");
        for command in [
            r"tools\agent.exe --silent",
            r"%MANGODISK_UNKNOWN_STARTUP_ROOT%\agent.exe --silent",
            r"C:\Program Files\Example\agent.exe --silent",
        ] {
            let artifact = artifact_from_value(source, "Fixture", command, None);
            assert!(!artifact
                .diagnostics
                .contains(&PlatformStartupDiagnosticCode::MissingTarget));
        }
    }

    #[test]
    fn absent_absolute_target_is_reported_as_missing() {
        let source = registry_sources()
            .into_iter()
            .next()
            .expect("at least one registry source must exist");
        let artifact = artifact_from_value(
            source,
            "Fixture",
            r#""C:\MangoDiskMissingFixture\agent.exe" --silent"#,
            None,
        );

        assert!(artifact
            .diagnostics
            .contains(&PlatformStartupDiagnosticCode::MissingTarget));
    }

    #[test]
    fn startup_approved_change_preserves_native_payload() {
        let original = RegValue {
            bytes: vec![0x02, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 11].into(),
            vtype: REG_BINARY,
        };

        let disabled =
            changed_startup_approved_bytes(Some(&original), PlatformStartupDesiredState::Disabled)
                .expect("a recognized startup approval payload must be mutable");

        assert_eq!(disabled[0], 0x03);
        assert_eq!(disabled[1..], original.bytes[1..]);
    }

    #[test]
    fn startup_approved_change_rejects_truncated_payload() {
        let original = RegValue {
            bytes: vec![0x02, 0].into(),
            vtype: REG_BINARY,
        };

        assert!(changed_startup_approved_bytes(
            Some(&original),
            PlatformStartupDesiredState::Disabled,
        )
        .is_err());
    }

    #[test]
    #[ignore = "modifies and removes an isolated HKCU startup fixture, then restores it"]
    fn actual_hkcu_run_change_and_removal_are_verified() {
        use std::process;

        use crate::{
            PlatformStartupChangeRequest, PlatformStartupControlCapability,
            PlatformStartupDesiredState,
        };
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY};
        let value_name = format!("MangoDiskStartupFixture{}", process::id());
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (run_key, _) = root
            .create_subkey_with_flags(RUN_PATH, KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY)
            .expect("the HKCU Run fixture key must be writable");
        let (approval_key, _) = root
            .create_subkey_with_flags(
                format!(r"{STARTUP_APPROVED_PATH}\Run"),
                KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY,
            )
            .expect("the HKCU approval fixture key must be writable");
        let previous_run = run_key.get_raw_value(&value_name).ok();
        let previous_approval = approval_key.get_raw_value(&value_name).ok();
        let _cleanup = RegistryFixtureGuard {
            value_name: value_name.clone(),
            previous_run,
            previous_approval,
        };
        let command = r#""C:\Windows\System32\notepad.exe" --mangodisk-startup-fixture"#;
        run_key
            .set_value(&value_name, &command)
            .expect("the HKCU Run fixture must be created");
        let _ = approval_key.delete_value(&value_name);

        let cancellation = crate::PlatformCancellation::new(|| false);
        let scanned = scan(&cancellation);
        let enabled = scanned
            .items
            .into_iter()
            .find(|item| item.display_name == value_name)
            .expect("the HKCU Run fixture must be discovered");
        assert_eq!(
            enabled.control_capability,
            PlatformStartupControlCapability::Toggleable
        );
        let disabled = change(&PlatformStartupChangeRequest {
            provider_item_id: enabled.provider_item_id.clone(),
            source_id: "windows.registry.run".to_owned(),
            expected_artifact: enabled,
            desired_state: PlatformStartupDesiredState::Disabled,
        })
        .expect("the HKCU Run fixture must be disabled");
        assert!(disabled.verified);
        assert_eq!(
            disabled.configured_state,
            PlatformStartupConfiguredState::Disabled
        );

        let (_, _, current) = find_item(&format!(
            "registry:hkcu:64:{}:{}",
            RUN_PATH.to_lowercase(),
            value_name.to_lowercase()
        ))
        .expect("the disabled fixture must remain discoverable");
        let enabled_again = change(&PlatformStartupChangeRequest {
            provider_item_id: current.provider_item_id.clone(),
            source_id: "windows.registry.run".to_owned(),
            expected_artifact: current,
            desired_state: PlatformStartupDesiredState::Enabled,
        })
        .expect("the HKCU Run fixture must be enabled again");
        assert!(enabled_again.verified);
        assert_eq!(
            enabled_again.configured_state,
            PlatformStartupConfiguredState::Enabled
        );
        assert!(approval_key.get_raw_value(&value_name).is_err());

        let (_, _, current) = find_item(&format!(
            "registry:hkcu:64:{}:{}",
            RUN_PATH.to_lowercase(),
            value_name.to_lowercase()
        ))
        .expect("the enabled fixture must remain discoverable");
        assert!(!current
            .diagnostics
            .contains(&PlatformStartupDiagnosticCode::MissingTarget));
        let removed = change(&PlatformStartupChangeRequest {
            provider_item_id: current.provider_item_id.clone(),
            source_id: "windows.registry.run".to_owned(),
            expected_artifact: current,
            desired_state: PlatformStartupDesiredState::Removed,
        })
        .expect("the HKCU Run fixture must be removed");
        assert!(removed.verified);
        assert_eq!(
            removed.configured_state,
            PlatformStartupConfiguredState::NotApplicable
        );
        assert!(run_key.get_raw_value(&value_name).is_err());
        assert!(approval_key.get_raw_value(&value_name).is_err());
    }

    #[test]
    #[ignore = "removes an isolated orphaned HKCU startup fixture and restores prior values"]
    fn actual_hkcu_orphan_removal_deletes_only_the_fixture() {
        use std::process;

        use crate::{PlatformStartupChangeRequest, PlatformStartupControlCapability};
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY};

        let value_name = format!("MangoDiskOrphanFixture{}", process::id());
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (run_key, _) = root
            .create_subkey_with_flags(RUN_PATH, KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY)
            .expect("the HKCU Run fixture key must be writable");
        let (approval_key, _) = root
            .create_subkey_with_flags(
                format!(r"{STARTUP_APPROVED_PATH}\Run"),
                KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY,
            )
            .expect("the HKCU approval fixture key must be writable");
        let _cleanup = RegistryFixtureGuard {
            value_name: value_name.clone(),
            previous_run: run_key.get_raw_value(&value_name).ok(),
            previous_approval: approval_key.get_raw_value(&value_name).ok(),
        };
        run_key
            .set_value(
                &value_name,
                &r#""C:\MangoDiskMissingFixture\agent.exe" --silent"#,
            )
            .expect("the orphaned Run fixture must be created");
        approval_key
            .set_raw_value(
                &value_name,
                &RegValue {
                    bytes: {
                        let mut bytes = vec![0x02; 12];
                        bytes[0] = 0x03;
                        bytes.into()
                    },
                    vtype: REG_BINARY,
                },
            )
            .expect("the orphaned approval fixture must be created");

        let cancellation = crate::PlatformCancellation::new(|| false);
        let orphan = scan(&cancellation)
            .items
            .into_iter()
            .find(|item| item.display_name == value_name)
            .expect("the orphaned Run fixture must be discovered");
        assert_eq!(
            orphan.control_capability,
            PlatformStartupControlCapability::Toggleable
        );
        assert!(orphan
            .diagnostics
            .contains(&PlatformStartupDiagnosticCode::MissingTarget));

        let removed = change(&PlatformStartupChangeRequest {
            provider_item_id: orphan.provider_item_id.clone(),
            source_id: "windows.registry.run".to_owned(),
            expected_artifact: orphan,
            desired_state: PlatformStartupDesiredState::Removed,
        })
        .expect("the orphaned Run fixture must be removed");

        assert!(removed.verified);
        assert_eq!(
            removed.configured_state,
            PlatformStartupConfiguredState::NotApplicable
        );
        assert!(run_key.get_raw_value(&value_name).is_err());
        assert!(approval_key.get_raw_value(&value_name).is_err());
    }

    #[test]
    #[ignore = "uses a three-phase isolated fixture across two explicit Windows restarts"]
    fn actual_hkcu_run_change_survives_restart_and_login() {
        use std::{env, fs, time::Duration};

        use crate::{
            PlatformStartupChangeRequest, PlatformStartupControlCapability,
            PlatformStartupDesiredState,
        };
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY};

        const VALUE_NAME: &str = "MangoDiskStartupRebootFixture";
        let phase = env::var("MANGODISK_STARTUP_REBOOT_PHASE")
            .expect("MANGODISK_STARTUP_REBOOT_PHASE must select prepare, verify-disabled, verify-enabled, or cleanup");
        let marker_path = env::temp_dir().join("MangoDiskStartupRebootFixture.marker");
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (run_key, _) = root
            .create_subkey_with_flags(RUN_PATH, KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY)
            .expect("the HKCU Run fixture key must be writable");
        let (approval_key, _) = root
            .create_subkey_with_flags(
                format!(r"{STARTUP_APPROVED_PATH}\Run"),
                KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY,
            )
            .expect("the HKCU approval fixture key must be writable");

        if phase == "cleanup" {
            cleanup_restart_fixture(&run_key, &approval_key, &marker_path);
            return;
        }
        let mut cleanup = RestartFixtureGuard {
            run_key: &run_key,
            approval_key: &approval_key,
            marker_path: &marker_path,
            preserve_for_restart: false,
        };

        match phase.as_str() {
            "prepare" => {
                assert!(
                    run_key.get_raw_value(VALUE_NAME).is_err()
                        && approval_key.get_raw_value(VALUE_NAME).is_err(),
                    "the reboot fixture namespace must be empty before preparation"
                );
                let _ = fs::remove_file(&marker_path);
                let command = format!(
                    r#""C:\Windows\System32\cmd.exe" /d /c type nul > "{}""#,
                    marker_path.display()
                );
                run_key
                    .set_value(VALUE_NAME, &command)
                    .expect("the reboot fixture Run value must be created");
                let enabled = restart_fixture_artifact(VALUE_NAME);
                assert_eq!(
                    enabled.control_capability,
                    PlatformStartupControlCapability::Toggleable
                );
                let disabled = change(&PlatformStartupChangeRequest {
                    provider_item_id: enabled.provider_item_id.clone(),
                    source_id: "windows.registry.run".to_owned(),
                    expected_artifact: enabled,
                    desired_state: PlatformStartupDesiredState::Disabled,
                })
                .expect("the reboot fixture must be disabled before restart");
                assert!(disabled.verified);
                cleanup.preserve_for_restart = true;
            }
            "verify-disabled" => {
                assert!(
                    restart_fixture_marker_remains_absent(&marker_path, Duration::from_secs(30)),
                    "a disabled Run fixture must not execute after restart and login"
                );
                let disabled = restart_fixture_artifact(VALUE_NAME);
                assert_eq!(
                    disabled.configured_state,
                    PlatformStartupConfiguredState::Disabled
                );
                let enabled = change(&PlatformStartupChangeRequest {
                    provider_item_id: disabled.provider_item_id.clone(),
                    source_id: "windows.registry.run".to_owned(),
                    expected_artifact: disabled,
                    desired_state: PlatformStartupDesiredState::Enabled,
                })
                .expect("the reboot fixture must be enabled before the second restart");
                assert!(enabled.verified);
                cleanup.preserve_for_restart = true;
            }
            "verify-enabled" => {
                assert!(
                    wait_for_restart_fixture_marker(&marker_path, Duration::from_secs(30)),
                    "a restored Run fixture must execute after restart and login"
                );
                let enabled = restart_fixture_artifact(VALUE_NAME);
                assert_eq!(
                    enabled.configured_state,
                    PlatformStartupConfiguredState::Enabled
                );
            }
            _ => panic!("unsupported reboot fixture phase"),
        }
    }

    fn restart_fixture_artifact(value_name: &str) -> crate::PlatformStartupArtifact {
        let cancellation = crate::PlatformCancellation::new(|| false);
        scan(&cancellation)
            .items
            .into_iter()
            .find(|item| item.display_name == value_name)
            .expect("the reboot fixture must be discovered")
    }

    struct RestartFixtureGuard<'a> {
        run_key: &'a RegKey,
        approval_key: &'a RegKey,
        marker_path: &'a std::path::Path,
        preserve_for_restart: bool,
    }

    impl Drop for RestartFixtureGuard<'_> {
        fn drop(&mut self) {
            if !self.preserve_for_restart {
                cleanup_restart_fixture(self.run_key, self.approval_key, self.marker_path);
            }
        }
    }

    fn cleanup_restart_fixture(
        run_key: &RegKey,
        approval_key: &RegKey,
        marker_path: &std::path::Path,
    ) {
        const VALUE_NAME: &str = "MangoDiskStartupRebootFixture";
        let _ = run_key.delete_value(VALUE_NAME);
        let _ = approval_key.delete_value(VALUE_NAME);
        let _ = std::fs::remove_file(marker_path);
    }

    fn restart_fixture_marker_remains_absent(
        marker_path: &std::path::Path,
        observation: std::time::Duration,
    ) -> bool {
        let deadline = std::time::Instant::now() + observation;
        while std::time::Instant::now() < deadline {
            if marker_path.exists() {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        !marker_path.exists()
    }

    fn wait_for_restart_fixture_marker(
        marker_path: &std::path::Path,
        timeout: std::time::Duration,
    ) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if marker_path.exists() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        marker_path.exists()
    }

    struct RegistryFixtureGuard {
        value_name: String,
        previous_run: Option<RegValue<'static>>,
        previous_approval: Option<RegValue<'static>>,
    }

    impl Drop for RegistryFixtureGuard {
        fn drop(&mut self) {
            let root = RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
            if let Ok(run_key) = root.open_subkey_with_flags(
                RUN_PATH,
                winreg::enums::KEY_SET_VALUE | winreg::enums::KEY_WOW64_64KEY,
            ) {
                restore_registry_value(&run_key, &self.value_name, self.previous_run.as_ref());
            }
            if let Ok(approval_key) = root.open_subkey_with_flags(
                format!(r"{STARTUP_APPROVED_PATH}\Run"),
                winreg::enums::KEY_SET_VALUE | winreg::enums::KEY_WOW64_64KEY,
            ) {
                restore_registry_value(
                    &approval_key,
                    &self.value_name,
                    self.previous_approval.as_ref(),
                );
            }
        }
    }

    fn restore_registry_value(key: &RegKey, name: &str, previous: Option<&RegValue<'_>>) {
        if let Some(previous) = previous {
            let _ = key.set_raw_value(name, previous);
        } else {
            let _ = key.delete_value(name);
        }
    }
}
