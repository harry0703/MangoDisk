mod command;
#[cfg(test)]
use command::split_registered_command;

use std::{
    collections::HashSet,
    env,
    ffi::{c_void, OsString},
    fs, iter,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        process::{CommandExt, ExitStatusExt},
    },
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    ptr, thread,
    time::{Duration, Instant},
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_CANCELLED, ERROR_GEN_FAILURE, ERROR_NO_MORE_FILES,
        ERROR_SUCCESS, ERROR_SUCCESS_REBOOT_INITIATED, ERROR_SUCCESS_REBOOT_REQUIRED, HANDLE,
        INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::{
        GetTokenInformation, TokenElevation, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE,
        TOKEN_ELEVATION, TOKEN_QUERY,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock},
        Ioctl::FSCTL_GET_REPARSE_POINT,
        SystemInformation::GetSystemDirectoryW,
        SystemServices::IO_REPARSE_TAG_APPEXECLINK,
        Threading::{
            CreateProcessWithTokenW, GetCurrentProcess, GetExitCodeProcess, GetProcessId,
            OpenProcess, OpenProcessToken, WaitForSingleObject, CREATE_NO_WINDOW,
            CREATE_UNICODE_ENVIRONMENT, LOGON_WITH_PROFILE, PROCESS_INFORMATION,
            PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
        },
        IO::DeviceIoControl,
    },
    UI::{
        Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
        WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId, SW_HIDE, SW_SHOWNORMAL},
    },
};
use winreg::{
    enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY},
    RegKey,
};

use crate::{
    configure_background_process, ApplicationInstallScope, ApplicationUninstallExecutionOutcome,
    ApplicationUninstallPlatformError, ApplicationUninstallRegistration,
    ApplicationUninstallRegistrationState, WindowsRegisteredUninstallKind, WindowsRegistryView,
};

use super::{package_evidence, package_locations, path_identity};

const UNINSTALL_PATH: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const MAXIMUM_REPARSE_DATA_BUFFER_SIZE: usize = 16 * 1024;
const PROCESS_TREE_POLL_INTERVAL: Duration = Duration::from_millis(200);
const PROCESS_TREE_SETTLED_POLLS: u8 = 3;
const ELEVATED_EXECUTOR_CHOCOLATEY: &str = "windows_chocolatey";
const EXECUTOR_REGISTERED_POWERSHELL: &str = "windows_registered_powershell";
const EXECUTOR_REGISTERED: &str = "windows_registered";
const EXECUTOR_SCOOP: &str = "windows_scoop";
const EXECUTOR_WINGET: &str = "windows_winget";
const WINGET_PACKAGE_FAMILY: &str = "Microsoft.DesktopAppInstaller_8wekyb3d8bbwe";
const WINGET_AUMID: &str = "Microsoft.DesktopAppInstaller_8wekyb3d8bbwe!winget";

mod appx;
mod msi;

/// Resolves the Windows Installer context instead of inferring it from the
/// uninstall registry root. A per-user MSI may publish its display entry under
/// HKLM, so registry provenance is not sufficient authorization evidence.
pub(super) fn msi_install_scope(
    product_code: &str,
) -> Result<Option<ApplicationInstallScope>, ApplicationUninstallPlatformError> {
    msi::install_scope(product_code)
}

pub(super) fn registration_state(
    registration: &ApplicationUninstallRegistration,
) -> Result<ApplicationUninstallRegistrationState, ApplicationUninstallPlatformError> {
    match registration {
        ApplicationUninstallRegistration::WindowsMsi {
            product_code,
            scope,
            ..
        } => msi::registration_state(product_code, *scope),
        ApplicationUninstallRegistration::WindowsAppx {
            package_full_name, ..
        } => appx::package_state(package_full_name),
        ApplicationUninstallRegistration::WindowsScoop {
            package_name,
            scope,
            install_root,
            package_marker_digest,
            scoop_script_digest,
            ..
        } => scoop_package_state(
            package_name,
            *scope,
            install_root,
            package_marker_digest,
            scoop_script_digest,
        ),
        ApplicationUninstallRegistration::WindowsChocolatey {
            package_name,
            install_root,
            package_marker_digest,
            chocolatey_executable,
            ..
        } => chocolatey_package_state(
            package_name,
            install_root,
            package_marker_digest,
            chocolatey_executable,
        ),
        ApplicationUninstallRegistration::WindowsRegistered {
            key_name,
            scope,
            registry_view,
            command_kind,
            command_digest,
            ..
        } => registered_uninstall_state(
            key_name,
            *scope,
            *registry_view,
            *command_kind,
            command_digest,
        ),
    }
}

pub(super) fn execute_registration(
    registration: &ApplicationUninstallRegistration,
) -> Result<ApplicationUninstallExecutionOutcome, ApplicationUninstallPlatformError> {
    match registration {
        ApplicationUninstallRegistration::WindowsMsi {
            product_code,
            scope,
            ..
        } => msi::execute(product_code, *scope),
        ApplicationUninstallRegistration::WindowsAppx {
            package_full_name, ..
        } => appx::execute(package_full_name),
        ApplicationUninstallRegistration::WindowsScoop {
            package_name,
            scope,
            install_root,
            package_marker_digest,
            scoop_script_digest,
            ..
        } => execute_scoop(
            package_name,
            *scope,
            install_root,
            package_marker_digest,
            scoop_script_digest,
            registration,
        ),
        ApplicationUninstallRegistration::WindowsChocolatey {
            package_name,
            install_root,
            package_marker_digest,
            chocolatey_executable,
            ..
        } => execute_chocolatey(
            package_name,
            install_root,
            package_marker_digest,
            chocolatey_executable,
            registration,
        ),
        ApplicationUninstallRegistration::WindowsRegistered {
            key_name,
            scope,
            registry_view,
            command_kind,
            command_digest,
            ..
        } => execute_registered_uninstaller(
            key_name,
            *scope,
            *registry_view,
            *command_kind,
            command_digest,
            registration,
        ),
    }
}

fn scoop_package_state(
    package_name: &str,
    scope: ApplicationInstallScope,
    install_root: &Path,
    expected_package_digest: &str,
    expected_script_digest: &str,
) -> Result<ApplicationUninstallRegistrationState, ApplicationUninstallPlatformError> {
    validate_scoop_registration(package_name, scope, install_root)?;
    let package_current = install_root.join("apps").join(package_name).join("current");
    if !package_current.is_dir() {
        return Ok(ApplicationUninstallRegistrationState::Absent);
    }
    let package_digest = package_evidence::file_set_digest(&[
        &package_current.join("install.json"),
        &package_current.join("manifest.json"),
    ])
    .ok_or(ApplicationUninstallPlatformError::RegistrationChanged)?;
    let scoop_script = scoop_script_path(install_root);
    let script_digest = package_evidence::file_set_digest(&[&scoop_script])
        .ok_or(ApplicationUninstallPlatformError::RegistrationChanged)?;
    if package_digest != expected_package_digest || script_digest != expected_script_digest {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    Ok(ApplicationUninstallRegistrationState::Installed)
}

fn execute_scoop(
    package_name: &str,
    scope: ApplicationInstallScope,
    install_root: &Path,
    expected_package_digest: &str,
    expected_script_digest: &str,
    registration: &ApplicationUninstallRegistration,
) -> Result<ApplicationUninstallExecutionOutcome, ApplicationUninstallPlatformError> {
    if scoop_package_state(
        package_name,
        scope,
        install_root,
        expected_package_digest,
        expected_script_digest,
    )? != ApplicationUninstallRegistrationState::Installed
    {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }

    // Machine-scoped Scoop packages require an elevated package-manager
    // boundary. MangoDisk intentionally does not execute a user-owned Scoop
    // script with administrator rights, so global packages remain view-only.
    if scope != ApplicationInstallScope::CurrentUser {
        return Err(ApplicationUninstallPlatformError::Unsupported);
    }

    let status = execute_current_user_command(
        &system_powershell_path()?,
        &[
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            scoop_script_path(install_root)
                .to_string_lossy()
                .into_owned(),
            "uninstall".to_string(),
            package_name.to_string(),
        ],
        EXECUTOR_SCOOP,
    )?;
    if !status.success() {
        return Err(ApplicationUninstallPlatformError::NativeFailure(
            exit_code_or_fallback(status.code()),
        ));
    }
    if registration_state(registration)? != ApplicationUninstallRegistrationState::Absent {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    Ok(ApplicationUninstallExecutionOutcome::Completed)
}

fn validate_scoop_registration(
    package_name: &str,
    scope: ApplicationInstallScope,
    install_root: &Path,
) -> Result<(), ApplicationUninstallPlatformError> {
    if package_name.is_empty()
        || package_name.len() > 128
        || !package_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        || package_name.eq_ignore_ascii_case("scoop")
    {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    let allowed = scoop_install_roots()
        .into_iter()
        .any(|allowed| allowed.scope == scope && windows_paths_match(&allowed.path, install_root));
    if !allowed {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    Ok(())
}

fn chocolatey_package_state(
    package_name: &str,
    install_root: &Path,
    expected_package_digest: &str,
    chocolatey_executable: &crate::command::ControlledExecutable,
) -> Result<ApplicationUninstallRegistrationState, ApplicationUninstallPlatformError> {
    validate_chocolatey_registration(package_name, install_root, chocolatey_executable)?;
    let marker = chocolatey_package_marker_path(install_root, package_name);
    if !marker.is_file() {
        return Ok(ApplicationUninstallRegistrationState::Absent);
    }
    let package_digest = package_evidence::file_set_digest(&[&marker])
        .ok_or(ApplicationUninstallPlatformError::RegistrationChanged)?;
    if package_digest != expected_package_digest {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    Ok(ApplicationUninstallRegistrationState::Installed)
}

fn execute_chocolatey(
    package_name: &str,
    install_root: &Path,
    expected_package_digest: &str,
    chocolatey_executable: &crate::command::ControlledExecutable,
    registration: &ApplicationUninstallRegistration,
) -> Result<ApplicationUninstallExecutionOutcome, ApplicationUninstallPlatformError> {
    if chocolatey_package_state(
        package_name,
        install_root,
        expected_package_digest,
        chocolatey_executable,
    )? != ApplicationUninstallRegistrationState::Installed
    {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }

    let executable = chocolatey_executable
        .validated_path()
        .map_err(|_| ApplicationUninstallPlatformError::RegistrationChanged)?;
    let arguments = format!("uninstall {package_name} --yes --no-progress --limit-output");
    let exit_code =
        execute_elevated_executable(executable, &arguments, ELEVATED_EXECUTOR_CHOCOLATEY)?;
    let outcome = match exit_code {
        ERROR_SUCCESS => ApplicationUninstallExecutionOutcome::Completed,
        ERROR_SUCCESS_REBOOT_REQUIRED | ERROR_SUCCESS_REBOOT_INITIATED => {
            ApplicationUninstallExecutionOutcome::RestartRequired
        }
        code => return Err(ApplicationUninstallPlatformError::NativeFailure(code)),
    };
    if registration_state(registration)? != ApplicationUninstallRegistrationState::Absent {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    Ok(outcome)
}

fn validate_chocolatey_registration(
    package_name: &str,
    install_root: &Path,
    chocolatey_executable: &crate::command::ControlledExecutable,
) -> Result<(), ApplicationUninstallPlatformError> {
    if !valid_package_name(package_name) || package_name.eq_ignore_ascii_case("chocolatey") {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    let expected_root = package_locations::chocolatey_root()
        .ok_or(ApplicationUninstallPlatformError::RegistrationChanged)?;
    if !windows_paths_match(&expected_root, install_root) {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    let executable = chocolatey_executable
        .validated_path()
        .map_err(|_| ApplicationUninstallPlatformError::RegistrationChanged)?;
    if !windows_paths_match(executable, &install_root.join("bin").join("choco.exe"))
        && !windows_paths_match(executable, &install_root.join("choco.exe"))
    {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    Ok(())
}

fn chocolatey_package_marker_path(install_root: &Path, package_name: &str) -> PathBuf {
    install_root
        .join("lib")
        .join(package_name)
        .join(format!("{package_name}.nuspec"))
}

fn valid_package_name(package_name: &str) -> bool {
    !package_name.is_empty()
        && package_name.len() <= 128
        && package_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn scoop_install_roots() -> Vec<package_locations::ScoopRoot> {
    package_locations::scoop_roots()
}

fn scoop_script_path(install_root: &Path) -> PathBuf {
    install_root
        .join("apps")
        .join("scoop")
        .join("current")
        .join("bin")
        .join("scoop.ps1")
}

fn windows_paths_match(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    path_identity::equal(&left, &right)
}

pub(super) fn registered_uninstall_command_evidence(
    command: &str,
    key_name: &str,
    scope: ApplicationInstallScope,
) -> Option<(WindowsRegisteredUninstallKind, String)> {
    registered_uninstall_command_evidence_with_diagnostic(command, key_name, scope).ok()
}

pub(super) fn registered_uninstall_command_evidence_with_diagnostic(
    command: &str,
    key_name: &str,
    scope: ApplicationInstallScope,
) -> Result<(WindowsRegisteredUninstallKind, String), RegisteredCommandRejection> {
    let validated = diagnose_registered_command(command, key_name, scope)?;
    let command_kind = validated.kind();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-windows-registered-uninstaller-v2");
    hasher.update(command_kind.stable_code().as_bytes());
    match validated {
        ValidatedRegisteredCommand::Executable {
            executable,
            arguments,
            ..
        } => {
            hasher.update(path_identity::comparison_key(&executable).as_bytes());
            hasher.update(arguments.as_bytes());
        }
        ValidatedRegisteredCommand::UserPowerShellScript { script, arguments } => {
            hasher.update(path_identity::comparison_key(&script).as_bytes());
            for argument in arguments {
                hasher.update(&[0]);
                hasher.update(argument.as_bytes());
            }
        }
        ValidatedRegisteredCommand::WingetProduct { product_code } => {
            hasher.update(product_code.to_ascii_lowercase().as_bytes());
        }
    }
    Ok((command_kind, hasher.finalize().to_hex().to_string()))
}

fn registered_uninstall_state(
    key_name: &str,
    scope: ApplicationInstallScope,
    registry_view: WindowsRegistryView,
    expected_kind: WindowsRegisteredUninstallKind,
    expected_digest: &str,
) -> Result<ApplicationUninstallRegistrationState, ApplicationUninstallPlatformError> {
    let Some(values) = read_registered_uninstall_values(key_name, scope, registry_view)? else {
        return Ok(ApplicationUninstallRegistrationState::Absent);
    };
    match registered_uninstall_command_evidence_with_diagnostic(&values.command, key_name, scope) {
        Ok((kind, digest)) if kind == expected_kind && digest == expected_digest => {
            Ok(ApplicationUninstallRegistrationState::Installed)
        }
        Ok(_) => {
            log::warn!("windows_uninstall_registration_changed application_id={} registry_view={:?} stage=revalidate reason=command_identity_changed", registered_diagnostic_id(key_name, scope), registry_view);
            Err(ApplicationUninstallPlatformError::RegistrationChanged)
        }
        Err(rejection) => {
            log::warn!("windows_uninstall_registration_changed application_id={} registry_view={:?} stage=revalidate reason={} detail={} native_code={:?}", registered_diagnostic_id(key_name, scope), registry_view, rejection.reason.stable_code(), rejection.detail, rejection.native_code);
            Err(ApplicationUninstallPlatformError::RegistrationChanged)
        }
    }
}

fn execute_registered_uninstaller(
    key_name: &str,
    scope: ApplicationInstallScope,
    registry_view: WindowsRegistryView,
    expected_kind: WindowsRegisteredUninstallKind,
    expected_digest: &str,
    registration: &ApplicationUninstallRegistration,
) -> Result<ApplicationUninstallExecutionOutcome, ApplicationUninstallPlatformError> {
    if registration_state(registration)? != ApplicationUninstallRegistrationState::Installed {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    let values = read_registered_uninstall_values(key_name, scope, registry_view)?
        .ok_or(ApplicationUninstallPlatformError::RegistrationChanged)?;
    let (kind, digest) = registered_uninstall_command_evidence(&values.command, key_name, scope)
        .ok_or(ApplicationUninstallPlatformError::RegistrationChanged)?;
    if kind != expected_kind || digest != expected_digest {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    let validated = validated_registered_command(&values.command, key_name, scope)
        .ok_or(ApplicationUninstallPlatformError::RegistrationChanged)?;
    let status = match validated {
        ValidatedRegisteredCommand::UserPowerShellScript { script, arguments } => {
            execute_user_powershell_script(&script, &arguments)?
        }
        ValidatedRegisteredCommand::WingetProduct { product_code }
            if scope == ApplicationInstallScope::CurrentUser =>
        {
            execute_user_winget_product(&product_code)?
        }
        ValidatedRegisteredCommand::Executable {
            executable,
            arguments,
        } => ExitStatus::from_raw(execute_shell_executable(
            &executable,
            &arguments,
            EXECUTOR_REGISTERED,
            ShellLaunchMode::Default,
        )?),
        validated => {
            let mut command = command_for_registered_uninstaller(validated)?;
            execute_command_process_tree(&mut command)?
        }
    };
    // Some vendors return a failure code after removing their registration. Record both facts
    // even for nonzero exits so support can distinguish a remaining installation from partial
    // removal. The observation is diagnostic evidence, not permission to reinterpret vendor
    // failures as success or to delete files that the installer left behind.
    let observed_state = registered_uninstall_state(
        key_name,
        scope,
        registry_view,
        expected_kind,
        expected_digest,
    );
    log::info!(
        "windows_uninstall_postflight application_id={} registry_view={:?} exit_code={:?} registration_state={} state_error={} state_native_code={:?}",
        registered_diagnostic_id(key_name, scope), registry_view, status.code(),
        match &observed_state {
            Ok(ApplicationUninstallRegistrationState::Installed) => "installed",
            Ok(ApplicationUninstallRegistrationState::Absent) => "absent",
            Ok(ApplicationUninstallRegistrationState::Incomplete) => "incomplete",
            Err(_) => "unknown",
        },
        observed_state.as_ref().err().map_or("none", |error| error.stable_code()),
        observed_state.as_ref().err().and_then(|error| error.native_code())
    );
    let outcome = registered_execution_outcome(status.code())?;
    if observed_state? != ApplicationUninstallRegistrationState::Absent {
        return Err(ApplicationUninstallPlatformError::RegistrationChanged);
    }
    Ok(outcome)
}

fn registered_execution_outcome(
    exit_code: Option<i32>,
) -> Result<ApplicationUninstallExecutionOutcome, ApplicationUninstallPlatformError> {
    let outcome = match exit_code {
        Some(0) => ApplicationUninstallExecutionOutcome::Completed,
        Some(code)
            if matches!(
                code as u32,
                ERROR_SUCCESS_REBOOT_REQUIRED | ERROR_SUCCESS_REBOOT_INITIATED
            ) =>
        {
            ApplicationUninstallExecutionOutcome::RestartRequired
        }
        code => {
            return Err(ApplicationUninstallPlatformError::NativeFailure(
                exit_code_or_fallback(code),
            ));
        }
    };
    Ok(outcome)
}

/// Launches a verified machine-level uninstaller through UAC and tracks its process tree.
///
/// The caller must revalidate the registration digest first. This boundary
/// rejects environment lookup, command interpreters, relative paths, missing
/// executables, and blocked hosts. Logs omit the executable path and arguments.
fn execute_elevated_executable(
    executable: &Path,
    arguments: &str,
    executor_kind: &'static str,
) -> Result<u32, ApplicationUninstallPlatformError> {
    execute_shell_executable(
        executable,
        arguments,
        executor_kind,
        ShellLaunchMode::RequestElevation,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellLaunchMode {
    Default,
    RequestElevation,
}

/// Launches a verified executable through Windows Shell and tracks its process tree.
///
/// Registered third-party uninstallers use the default Shell verb. Windows can
/// then honor the executable manifest and installer-detection policy, prompting
/// for UAC only when the uninstaller itself requires it. MangoDisk explicitly
/// requests elevation only for executors it controls, such as `msiexec`.
fn execute_shell_executable(
    executable: &Path,
    arguments: &str,
    executor_kind: &'static str,
    launch_mode: ShellLaunchMode,
) -> Result<u32, ApplicationUninstallPlatformError> {
    let started = Instant::now();
    let executable = wide_path(executable);
    let verb = match launch_mode {
        ShellLaunchMode::Default => None,
        ShellLaunchMode::RequestElevation => Some(wide_string("runas")),
    };
    let arguments = wide_string(arguments);
    let mut execution = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: verb.as_ref().map_or(ptr::null(), |value| value.as_ptr()),
        lpFile: executable.as_ptr(),
        lpParameters: arguments.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    let launch_mode_code = match launch_mode {
        ShellLaunchMode::Default => "shell_default",
        ShellLaunchMode::RequestElevation => "runas",
    };
    log::info!(
        "application_uninstall_shell_launch_requested executor_kind={executor_kind} launch_mode={launch_mode_code}"
    );
    if unsafe { ShellExecuteExW(&mut execution) } == 0 {
        let native_code = unsafe { GetLastError() };
        let error = elevation_request_error(native_code);
        if error == ApplicationUninstallPlatformError::UserCancelled {
            log::info!(
                "application_uninstall_shell_launch_cancelled executor_kind={executor_kind} launch_mode={launch_mode_code} native_code={native_code} elapsed_ms={}",
                started.elapsed().as_millis()
            );
        } else {
            log::warn!(
                "application_uninstall_shell_launch_failed executor_kind={executor_kind} launch_mode={launch_mode_code} stage=request native_code={native_code} elapsed_ms={}",
                started.elapsed().as_millis()
            );
        }
        return Err(error);
    }
    if execution.hProcess.is_null() {
        log::warn!(
            "application_uninstall_shell_launch_failed executor_kind={executor_kind} launch_mode={launch_mode_code} stage=process_handle native_code={ERROR_GEN_FAILURE} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        return Err(ApplicationUninstallPlatformError::NativeFailure(
            ERROR_GEN_FAILURE,
        ));
    }
    let process_handle = OwnedHandle(execution.hProcess);
    let process_id = unsafe { GetProcessId(process_handle.0) };
    if process_id == 0 {
        log::warn!(
            "application_uninstall_shell_launch_failed executor_kind={executor_kind} launch_mode={launch_mode_code} stage=process_identity native_code={ERROR_GEN_FAILURE} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        return Err(ApplicationUninstallPlatformError::NativeFailure(
            ERROR_GEN_FAILURE,
        ));
    }
    log::info!(
        "application_uninstall_shell_process_started executor_kind={executor_kind} launch_mode={launch_mode_code} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    let exit_code = wait_for_native_process_tree(process_handle.0, process_id).inspect_err(|error| {
        log::warn!(
            "application_uninstall_shell_process_wait_failed executor_kind={executor_kind} launch_mode={launch_mode_code} platform_error={} native_code={} elapsed_ms={}",
            error.stable_code(),
            error
                .native_code()
                .map_or_else(|| "none".to_string(), |code| code.to_string()),
            started.elapsed().as_millis()
        );
    })?;
    log::info!(
        "application_uninstall_shell_process_finished executor_kind={executor_kind} launch_mode={launch_mode_code} exit_code={exit_code} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    Ok(exit_code)
}

fn elevation_request_error(native_code: u32) -> ApplicationUninstallPlatformError {
    if native_code == ERROR_CANCELLED {
        ApplicationUninstallPlatformError::UserCancelled
    } else {
        ApplicationUninstallPlatformError::NativeFailure(native_code)
    }
}

fn registered_diagnostic_id(key_name: &str, scope: ApplicationInstallScope) -> String {
    let scope = match scope {
        ApplicationInstallScope::CurrentUser => "current-user",
        ApplicationInstallScope::Machine => "machine",
    };
    crate::application_uninstall_diagnostic_id(&format!(
        "windows-registry:{scope}:{}",
        key_name.to_ascii_lowercase()
    ))
}

struct RegisteredUninstallValues {
    command: String,
}

fn read_registered_uninstall_values(
    key_name: &str,
    scope: ApplicationInstallScope,
    registry_view: WindowsRegistryView,
) -> Result<Option<RegisteredUninstallValues>, ApplicationUninstallPlatformError> {
    let root = match scope {
        ApplicationInstallScope::CurrentUser => RegKey::predef(HKEY_CURRENT_USER),
        ApplicationInstallScope::Machine => RegKey::predef(HKEY_LOCAL_MACHINE),
    };
    let view = match registry_view {
        WindowsRegistryView::Registry32 => KEY_WOW64_32KEY,
        WindowsRegistryView::Registry64 => KEY_WOW64_64KEY,
    };
    let uninstall = match root.open_subkey_with_flags(UNINSTALL_PATH, KEY_READ | view) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            log::warn!("windows_uninstall_registration_changed application_id={} registry_view={:?} stage=open_registry native_code={:?} error_kind={:?}", registered_diagnostic_id(key_name, scope), registry_view, error.raw_os_error(), error.kind());
            return Err(io_error(error));
        }
    };
    let entry = match uninstall.open_subkey_with_flags(key_name, KEY_READ | view) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            log::warn!("windows_uninstall_registration_changed application_id={} registry_view={:?} stage=open_registry native_code={:?} error_kind={:?}", registered_diagnostic_id(key_name, scope), registry_view, error.raw_os_error(), error.kind());
            return Err(io_error(error));
        }
    };
    let command = entry
        .get_value::<String, _>("UninstallString")
        // An existing key without the expected command is a changed
        // registration, not proof that uninstall completed.
        .map_err(|error| {
            log::warn!("windows_uninstall_registration_changed application_id={} registry_view={:?} stage=read_command native_code={:?} error_kind={:?}", registered_diagnostic_id(key_name, scope), registry_view, error.raw_os_error(), error.kind());
            ApplicationUninstallPlatformError::RegistrationChanged
        })?;
    Ok(Some(RegisteredUninstallValues { command }))
}

enum ValidatedRegisteredCommand {
    Executable {
        executable: PathBuf,
        arguments: String,
    },
    UserPowerShellScript {
        script: PathBuf,
        arguments: Vec<String>,
    },
    WingetProduct {
        product_code: String,
    },
}

impl ValidatedRegisteredCommand {
    const fn kind(&self) -> WindowsRegisteredUninstallKind {
        match self {
            Self::Executable { .. } => WindowsRegisteredUninstallKind::Executable,
            Self::UserPowerShellScript { .. } => {
                WindowsRegisteredUninstallKind::UserPowerShellScript
            }
            Self::WingetProduct { .. } => WindowsRegisteredUninstallKind::WingetProduct,
        }
    }
}

impl WindowsRegisteredUninstallKind {
    const fn stable_code(self) -> &'static str {
        match self {
            Self::Executable => "executable",
            Self::UserPowerShellScript => "user-powershell-script",
            Self::WingetProduct => "winget-product",
        }
    }
}

/// Carries only a typed reason and OS code. Error messages can contain private paths, so they
/// must never be interpolated into inventory diagnostics.
#[derive(Debug)]
pub(super) struct RegisteredCommandRejection {
    pub(super) reason: crate::ApplicationUninstallDiagnostic,
    pub(super) native_code: Option<i32>,
    pub(super) detail: &'static str,
}

impl From<crate::ApplicationUninstallDiagnostic> for RegisteredCommandRejection {
    fn from(reason: crate::ApplicationUninstallDiagnostic) -> Self {
        Self {
            reason,
            native_code: None,
            detail: "none",
        }
    }
}

fn validated_registered_command(
    command: &str,
    key_name: &str,
    scope: ApplicationInstallScope,
) -> Option<ValidatedRegisteredCommand> {
    diagnose_registered_command(command, key_name, scope).ok()
}

fn diagnose_registered_command(
    command: &str,
    key_name: &str,
    scope: ApplicationInstallScope,
) -> Result<ValidatedRegisteredCommand, RegisteredCommandRejection> {
    use crate::ApplicationUninstallDiagnostic as Reason;
    if command.trim().is_empty() {
        return Err(Reason::CommandMissing.into());
    }
    if let Some(product_code) = parse_winget_product_code(command) {
        if product_code.eq_ignore_ascii_case(key_name) && trusted_winget_path().is_some() {
            return Ok(ValidatedRegisteredCommand::WingetProduct { product_code });
        }
        return Err(Reason::UnsupportedCommandHost.into());
    }
    if scope == ApplicationInstallScope::CurrentUser {
        if let Some((script, arguments)) = parse_user_powershell_script(command) {
            return Ok(ValidatedRegisteredCommand::UserPowerShellScript { script, arguments });
        }
    }
    let (executable, arguments) = command::parse_registered_command(command).map_err(|detail| {
        RegisteredCommandRejection {
            reason: Reason::InvalidCommand,
            native_code: None,
            detail,
        }
    })?;
    let expanded = expand_environment_path(&executable);
    if expanded.split('%').count() >= 3 {
        return Err(Reason::UnresolvedEnvironment.into());
    }
    let executable = PathBuf::from(expanded);
    if !executable.is_absolute() {
        return Err(Reason::RelativeExecutable.into());
    }
    if blocked_uninstaller_host(&executable) {
        return Err(Reason::UnsupportedCommandHost.into());
    }
    if !executable
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err(Reason::InvalidExecutable.into());
    }
    let metadata = fs::metadata(&executable).map_err(|error| RegisteredCommandRejection {
        reason: match error.kind() {
            std::io::ErrorKind::NotFound => Reason::ExecutableMissing,
            std::io::ErrorKind::PermissionDenied => Reason::ExecutableAccessDenied,
            _ => Reason::ExecutableProbeFailed,
        },
        native_code: error.raw_os_error(),
        detail: "metadata",
    })?;
    if !metadata.is_file() {
        return Err(Reason::InvalidExecutable.into());
    }
    Ok(ValidatedRegisteredCommand::Executable {
        executable,
        arguments,
    })
}

fn command_for_registered_uninstaller(
    validated: ValidatedRegisteredCommand,
) -> Result<Command, ApplicationUninstallPlatformError> {
    let mut command = match validated {
        ValidatedRegisteredCommand::Executable {
            executable,
            arguments,
            ..
        } => {
            let mut command = Command::new(executable);
            if !arguments.is_empty() {
                // Arguments are passed directly to the verified executable.
                // No command shell or environment-based executable lookup is
                // involved.
                command.raw_arg(arguments);
            }
            command
        }
        ValidatedRegisteredCommand::WingetProduct { product_code } => {
            let mut command = Command::new(
                trusted_winget_path()
                    .ok_or(ApplicationUninstallPlatformError::RegistrationChanged)?,
            );
            command.args([
                "uninstall",
                "--product-code",
                product_code.as_str(),
                "--disable-interactivity",
            ]);
            command.stdin(Stdio::null());
            command.stdout(Stdio::null());
            command.stderr(Stdio::null());
            command
        }
        ValidatedRegisteredCommand::UserPowerShellScript { .. } => {
            return Err(ApplicationUninstallPlatformError::Unsupported);
        }
    };
    configure_background_process(&mut command);
    Ok(command)
}

fn execute_user_powershell_script(
    script: &Path,
    arguments: &[String],
) -> Result<ExitStatus, ApplicationUninstallPlatformError> {
    let powershell = system_powershell_path()?;
    let mut command_arguments = vec![
        "-NoProfile".to_string(),
        "-ExecutionPolicy".to_string(),
        "Bypass".to_string(),
        "-File".to_string(),
        script.to_string_lossy().into_owned(),
    ];
    command_arguments.extend(arguments.iter().cloned());
    execute_current_user_command(
        &powershell,
        &command_arguments,
        EXECUTOR_REGISTERED_POWERSHELL,
    )
}

fn execute_user_winget_product(
    product_code: &str,
) -> Result<ExitStatus, ApplicationUninstallPlatformError> {
    let winget =
        trusted_winget_path().ok_or(ApplicationUninstallPlatformError::RegistrationChanged)?;
    execute_current_user_command(
        &winget,
        &[
            "uninstall".to_string(),
            "--product-code".to_string(),
            product_code.to_string(),
            "--disable-interactivity".to_string(),
        ],
        EXECUTOR_WINGET,
    )
}

/// Keeps a current-user uninstaller in the package owner's security context.
///
/// A normal MangoDisk process already owns the correct token and can launch
/// the verified executable directly. Calling `CreateProcessWithTokenW` from a
/// non-elevated process would fail with `ERROR_PRIVILEGE_NOT_HELD`. Only an
/// elevated process needs to drop back to the interactive Explorer token.
fn execute_current_user_command(
    executable: &Path,
    arguments: &[String],
    executor_kind: &'static str,
) -> Result<ExitStatus, ApplicationUninstallPlatformError> {
    let started = Instant::now();
    let parent_is_elevated = current_process_is_elevated()?;
    let launch_context = if parent_is_elevated {
        "interactive_shell"
    } else {
        "current_process"
    };
    log::info!(
        "application_uninstall_user_command_requested executor_kind={executor_kind} launch_context={launch_context}"
    );
    let result = if parent_is_elevated {
        execute_as_shell_user(executable, arguments)
    } else {
        let mut command = Command::new(executable);
        command.args(arguments);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        configure_background_process(&mut command);
        execute_command_process_tree(&mut command)
    };
    match &result {
        Ok(status) => log::info!(
            "application_uninstall_user_command_finished executor_kind={executor_kind} launch_context={launch_context} exit_code={} elapsed_ms={}",
            status
                .code()
                .map_or_else(|| "none".to_string(), |code| (code as u32).to_string()),
            started.elapsed().as_millis()
        ),
        Err(error) => log::warn!(
            "application_uninstall_user_command_failed executor_kind={executor_kind} launch_context={launch_context} platform_error={} native_code={} elapsed_ms={}",
            error.stable_code(),
            error
                .native_code()
                .map_or_else(|| "none".to_string(), |code| code.to_string()),
            started.elapsed().as_millis()
        ),
    }
    result
}

fn execute_command_process_tree(
    command: &mut Command,
) -> Result<ExitStatus, ApplicationUninstallPlatformError> {
    let mut child = command.spawn().map_err(io_error)?;
    let root_process_id = child.id();
    wait_for_process_tree(root_process_id, || child.try_wait().map_err(io_error))
}

fn wait_for_process_tree<T>(
    root_process_id: u32,
    mut poll_root: impl FnMut() -> Result<Option<T>, ApplicationUninstallPlatformError>,
) -> Result<T, ApplicationUninstallPlatformError> {
    let started = Instant::now();
    let mut tracked_process_ids = HashSet::from([root_process_id]);
    let mut root_result = None;
    let mut settled_polls = 0_u8;

    loop {
        let processes = process_parent_snapshot()?;
        extend_process_tree(&mut tracked_process_ids, &processes);
        if root_result.is_none() {
            root_result = poll_root()?;
        }
        let descendants_active = processes.iter().any(|(process_id, _)| {
            *process_id != root_process_id && tracked_process_ids.contains(process_id)
        });
        if root_result.is_some() && !descendants_active {
            settled_polls = settled_polls.saturating_add(1);
            if settled_polls >= PROCESS_TREE_SETTLED_POLLS {
                log::debug!(
                    "windows_uninstaller_process_tree_finished descendant_count={} elapsed_ms={}",
                    tracked_process_ids.len().saturating_sub(1),
                    started.elapsed().as_millis()
                );
                return root_result.ok_or(ApplicationUninstallPlatformError::NativeFailure(
                    ERROR_GEN_FAILURE,
                ));
            }
        } else {
            settled_polls = 0;
        }
        thread::sleep(PROCESS_TREE_POLL_INTERVAL);
    }
}

fn wait_for_native_process_tree(
    process_handle: HANDLE,
    process_id: u32,
) -> Result<u32, ApplicationUninstallPlatformError> {
    wait_for_process_tree(process_id, || {
        match unsafe { WaitForSingleObject(process_handle, 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut exit_code = ERROR_GEN_FAILURE;
                if unsafe { GetExitCodeProcess(process_handle, &mut exit_code) } == 0 {
                    return Err(last_native_error());
                }
                Ok(Some(exit_code))
            }
            _ => Err(last_native_error()),
        }
    })
}

fn extend_process_tree(tracked_process_ids: &mut HashSet<u32>, processes: &[(u32, u32)]) {
    loop {
        let previous_count = tracked_process_ids.len();
        for (process_id, parent_process_id) in processes {
            if process_id != parent_process_id && tracked_process_ids.contains(parent_process_id) {
                tracked_process_ids.insert(*process_id);
            }
        }
        if tracked_process_ids.len() == previous_count {
            return;
        }
    }
}

fn process_parent_snapshot() -> Result<Vec<(u32, u32)>, ApplicationUninstallPlatformError> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(last_native_error());
    }
    let _snapshot = OwnedHandle(snapshot);
    let mut entry = unsafe { std::mem::zeroed::<PROCESSENTRY32W>() };
    entry.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>())
        .map_err(|_| ApplicationUninstallPlatformError::Unsupported)?;
    if unsafe { Process32FirstW(snapshot, &mut entry) } == 0 {
        let error = unsafe { GetLastError() };
        return if error == ERROR_NO_MORE_FILES {
            Ok(Vec::new())
        } else {
            Err(ApplicationUninstallPlatformError::NativeFailure(error))
        };
    }

    let mut processes = Vec::new();
    loop {
        processes.push((entry.th32ProcessID, entry.th32ParentProcessID));
        if unsafe { Process32NextW(snapshot, &mut entry) } != 0 {
            continue;
        }
        let error = unsafe { GetLastError() };
        if error != ERROR_NO_MORE_FILES {
            return Err(ApplicationUninstallPlatformError::NativeFailure(error));
        }
        return Ok(processes);
    }
}

fn current_process_is_elevated() -> Result<bool, ApplicationUninstallPlatformError> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_native_error());
    }
    let token = OwnedHandle(token);
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned_bytes = 0_u32;
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>())
                .map_err(|_| ApplicationUninstallPlatformError::Unsupported)?,
            &mut returned_bytes,
        )
    } == 0
    {
        return Err(last_native_error());
    }
    if returned_bytes
        < u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>())
            .map_err(|_| ApplicationUninstallPlatformError::Unsupported)?
    {
        return Err(ApplicationUninstallPlatformError::NativeFailure(
            ERROR_GEN_FAILURE,
        ));
    }
    Ok(elevation.TokenIsElevated != 0)
}

/// Runs a verified current-user uninstaller with the interactive shell token.
///
/// MangoDisk may be elevated to remove machine-scoped applications. Reusing
/// that elevated token for a per-user WinGet package fails because WinGet
/// deliberately separates user and administrator package registrations. The
/// Explorer token preserves the package owner's identity without invoking a
/// command shell or trusting an environment-based executable lookup.
fn execute_as_shell_user(
    executable: &Path,
    arguments: &[String],
) -> Result<ExitStatus, ApplicationUninstallPlatformError> {
    let mut command_line = quote_windows_argument(&executable.to_string_lossy());
    for argument in arguments {
        command_line.push(' ');
        command_line.push_str(&quote_windows_argument(argument));
    }
    execute_as_shell_user_command_line(executable, command_line, true)
}

fn execute_as_shell_user_command_line(
    executable: &Path,
    command_line: String,
    hide_window: bool,
) -> Result<ExitStatus, ApplicationUninstallPlatformError> {
    let shell_window = unsafe { GetShellWindow() };
    if shell_window.is_null() {
        return Err(shell_user_context_failure(
            "shell_window",
            ERROR_GEN_FAILURE,
        ));
    }
    let mut shell_process_id = 0_u32;
    unsafe {
        GetWindowThreadProcessId(shell_window, &mut shell_process_id);
    }
    if shell_process_id == 0 {
        return Err(shell_user_context_failure(
            "shell_process_identity",
            ERROR_GEN_FAILURE,
        ));
    }
    let shell_process =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, shell_process_id) };
    if shell_process.is_null() {
        return Err(shell_user_context_last_error("shell_process_open"));
    }
    let shell_process = OwnedHandle(shell_process);
    let mut shell_token = ptr::null_mut();
    if unsafe {
        OpenProcessToken(
            shell_process.0,
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY,
            &mut shell_token,
        )
    } == 0
    {
        return Err(shell_user_context_last_error("shell_token_open"));
    }
    let shell_token = OwnedHandle(shell_token);
    let mut environment = ptr::null_mut();
    if unsafe { CreateEnvironmentBlock(&mut environment, shell_token.0, 0) } == 0 {
        return Err(shell_user_context_last_error("environment_create"));
    }
    let environment = OwnedEnvironmentBlock(environment);

    if !executable.is_absolute() || !executable.is_file() {
        return Err(ApplicationUninstallPlatformError::Unsupported);
    }
    let executable = wide_path(executable);
    let mut command_line = wide_string(&command_line);
    let startup = if hide_window {
        STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            dwFlags: windows_sys::Win32::System::Threading::STARTF_USESHOWWINDOW,
            wShowWindow: SW_HIDE as u16,
            ..Default::default()
        }
    } else {
        STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        }
    };
    let creation_flags =
        CREATE_UNICODE_ENVIRONMENT | if hide_window { CREATE_NO_WINDOW } else { 0 };
    let mut process = PROCESS_INFORMATION::default();
    let created = unsafe {
        CreateProcessWithTokenW(
            shell_token.0,
            LOGON_WITH_PROFILE,
            executable.as_ptr(),
            command_line.as_mut_ptr(),
            creation_flags,
            environment.0,
            ptr::null(),
            &startup,
            &mut process,
        )
    };
    if created == 0 {
        return Err(shell_user_context_last_error("process_create"));
    }
    let process_handle = OwnedHandle(process.hProcess);
    let _thread_handle = OwnedHandle(process.hThread);
    let exit_code = wait_for_native_process_tree(process_handle.0, process.dwProcessId)?;
    Ok(ExitStatus::from_raw(exit_code))
}

fn shell_user_context_last_error(stage: &'static str) -> ApplicationUninstallPlatformError {
    shell_user_context_failure(stage, unsafe { GetLastError() })
}

fn shell_user_context_failure(
    stage: &'static str,
    native_code: u32,
) -> ApplicationUninstallPlatformError {
    log::warn!(
        "application_uninstall_shell_user_context_failed stage={stage} native_code={native_code}"
    );
    ApplicationUninstallPlatformError::NativeFailure(native_code)
}

fn quote_windows_argument(value: &str) -> String {
    let escaped = value.replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn last_native_error() -> ApplicationUninstallPlatformError {
    ApplicationUninstallPlatformError::NativeFailure(unsafe { GetLastError() })
}

fn parse_user_powershell_script(command: &str) -> Option<(PathBuf, Vec<String>)> {
    let command = command.trim();
    if command.contains(['\r', '\n', ';', '|', '>', '<', '`']) {
        return None;
    }
    let (host, remainder) = split_powershell_host(command)?;
    if !matches!(
        host.to_ascii_lowercase().as_str(),
        "powershell" | "powershell.exe"
    ) && !Path::new(&host)
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("powershell.exe"))
    {
        return None;
    }

    let tokens = tokenize_registered_arguments(remainder)?;
    let (script, arguments) = parse_powershell_file_tokens(&tokens)
        .or_else(|| parse_powershell_command_tokens(&tokens))?;
    let script = trusted_current_user_script(&script)?;
    Some((script, arguments))
}

fn split_powershell_host(command: &str) -> Option<(String, &str)> {
    if let Some(quoted) = command.strip_prefix('"') {
        let closing_quote = quoted.find('"')?;
        let host = quoted[..closing_quote].trim();
        let remainder = quoted[closing_quote + 1..].trim();
        return (!host.is_empty()).then(|| (host.to_string(), remainder));
    }
    let end = command.find(char::is_whitespace).unwrap_or(command.len());
    let host = command[..end].trim();
    (!host.is_empty()).then(|| (host.to_string(), command[end..].trim()))
}

fn parse_powershell_file_tokens(tokens: &[String]) -> Option<(String, Vec<String>)> {
    let file_index = tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case("-file"))?;
    if !valid_powershell_host_options(tokens[..file_index].iter()) {
        return None;
    }
    let script = tokens.get(file_index + 1)?.clone();
    let arguments = tokens[file_index + 2..].to_vec();
    safe_script_arguments(&arguments).then_some((script, arguments))
}

fn parse_powershell_command_tokens(tokens: &[String]) -> Option<(String, Vec<String>)> {
    let command_index = tokens.iter().position(|token| {
        token.eq_ignore_ascii_case("-c") || token.eq_ignore_ascii_case("-command")
    })?;
    let body = tokens.get(command_index + 1)?.trim();
    if !valid_powershell_host_options(
        tokens[..command_index]
            .iter()
            .chain(tokens[command_index + 2..].iter()),
    ) {
        return None;
    }
    let body = body.strip_prefix('&')?.trim();
    let (script, remainder) = take_quoted_value(body)?;
    let arguments = tokenize_registered_arguments(remainder)?;
    if !safe_script_arguments(&arguments) {
        return None;
    }
    Some((script, arguments))
}

fn valid_powershell_host_options<'a>(tokens: impl IntoIterator<Item = &'a String>) -> bool {
    let mut tokens = tokens.into_iter();
    let mut no_profile = false;
    let mut no_logo = false;
    let mut non_interactive = false;
    let mut execution_policy = false;
    while let Some(token) = tokens.next() {
        if token.eq_ignore_ascii_case("-noprofile") && !no_profile {
            no_profile = true;
        } else if token.eq_ignore_ascii_case("-nologo") && !no_logo {
            no_logo = true;
        } else if token.eq_ignore_ascii_case("-noninteractive") && !non_interactive {
            non_interactive = true;
        } else if token.eq_ignore_ascii_case("-executionpolicy") && !execution_policy {
            if !tokens
                .next()
                .is_some_and(|policy| policy.eq_ignore_ascii_case("bypass"))
            {
                return false;
            }
            execution_policy = true;
        } else {
            return false;
        }
    }
    true
}

fn take_quoted_value(value: &str) -> Option<(String, &str)> {
    let quote = value.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let remainder = &value[quote.len_utf8()..];
    let end = remainder.find(quote)?;
    let selected = remainder[..end].to_string();
    Some((selected, remainder[end + quote.len_utf8()..].trim()))
}

fn tokenize_registered_arguments(value: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for character in value.chars() {
        match quote {
            Some(active) if character == active => quote = None,
            Some(_) => current.push(character),
            None if character == '"' || character == '\'' => quote = Some(character),
            None if character.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None => current.push(character),
        }
    }
    if quote.is_some() {
        return None;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Some(tokens)
}

fn safe_script_arguments(arguments: &[String]) -> bool {
    arguments.iter().all(|argument| {
        !argument.is_empty()
            && argument.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, '-' | '_' | '/' | '\\' | '.' | ':' | '=')
            })
    })
}

fn trusted_current_user_script(value: &str) -> Option<PathBuf> {
    let script = PathBuf::from(expand_environment_path(value));
    let profile = super::directories::user_directories()
        .ok()?
        .home_directory()
        .to_path_buf();
    let script = script.canonicalize().ok()?;
    let profile = profile.canonicalize().ok()?;
    if !script.is_file()
        || !script
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ps1"))
        || !path_identity::is_same_or_child(&script, &profile)
    {
        return None;
    }
    Some(script)
}

fn parse_winget_product_code(command: &str) -> Option<String> {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    (parts.len() == 4
        && matches!(
            parts[0].to_ascii_lowercase().as_str(),
            "winget" | "winget.exe"
        )
        && parts[1].eq_ignore_ascii_case("uninstall")
        && parts[2].eq_ignore_ascii_case("--product-code")
        && !parts[3].is_empty())
    .then(|| parts[3].to_string())
}

pub(super) fn trusted_winget_path() -> Option<PathBuf> {
    let alias_path = super::directories::local_data_directory()
        .ok()?
        .join("Microsoft")
        .join("WindowsApps")
        .join("winget.exe");
    let alias = read_app_execution_alias(&alias_path)?;
    let program_files = super::directories::program_files_directories().ok()?;
    // The per-user alias is the authoritative link between the current user
    // and Microsoft's App Installer package, so it remains the trust anchor.
    // Execute the validated package target instead of the alias itself:
    // CreateProcessWithTokenW cannot reliably activate App Execution Alias
    // reparse points after an elevated MangoDisk process switches back to the
    // interactive user's token (Windows may return ERROR_GEN_FAILURE).
    (alias.package_family == WINGET_PACKAGE_FAMILY
        && alias.application_user_model_id == WINGET_AUMID
        && trusted_winget_target(&alias.target, program_files.first()?))
    .then_some(alias.target)
}

struct AppExecutionAlias {
    package_family: String,
    application_user_model_id: String,
    target: PathBuf,
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct OwnedEnvironmentBlock(*mut c_void);

impl Drop for OwnedEnvironmentBlock {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                DestroyEnvironmentBlock(self.0);
            }
        }
    }
}

fn read_app_execution_alias(path: &Path) -> Option<AppExecutionAlias> {
    let path = wide_path(path);
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let handle = OwnedHandle(handle);
    let mut buffer = vec![0_u8; MAXIMUM_REPARSE_DATA_BUFFER_SIZE];
    let mut bytes_returned = 0_u32;
    let success = unsafe {
        DeviceIoControl(
            handle.0,
            FSCTL_GET_REPARSE_POINT,
            ptr::null(),
            0,
            buffer.as_mut_ptr().cast(),
            u32::try_from(buffer.len()).ok()?,
            &mut bytes_returned,
            ptr::null_mut(),
        )
    };
    if success == 0 {
        return None;
    }
    parse_app_execution_alias(&buffer[..usize::try_from(bytes_returned).ok()?])
}

fn parse_app_execution_alias(buffer: &[u8]) -> Option<AppExecutionAlias> {
    let header = buffer.get(..8)?;
    let tag = u32::from_le_bytes(header[0..4].try_into().ok()?);
    if tag != IO_REPARSE_TAG_APPEXECLINK {
        return None;
    }
    let data_length = usize::from(u16::from_le_bytes(header[4..6].try_into().ok()?));
    let data = buffer.get(8..8_usize.checked_add(data_length)?)?;
    let version = u32::from_le_bytes(data.get(..4)?.try_into().ok()?);
    if version != 3 {
        return None;
    }
    let fields_data = data.get(4..)?;
    if fields_data.len() % 2 != 0 {
        return None;
    }
    let units = fields_data
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect::<Vec<_>>();
    let fields = units
        .split(|unit| *unit == 0)
        .map(String::from_utf16)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(AppExecutionAlias {
        package_family: fields.first()?.clone(),
        application_user_model_id: fields.get(1)?.clone(),
        target: PathBuf::from(fields.get(2)?),
    })
}

fn trusted_winget_target(target: &Path, program_files: &Path) -> bool {
    let Some(relative) =
        path_identity::relative_child_key(target, &program_files.join("WindowsApps"))
    else {
        return false;
    };
    let mut components = relative.split('\\');
    let Some(package_directory) = components.next() else {
        return false;
    };
    package_directory.starts_with("microsoft.desktopappinstaller_")
        && package_directory.ends_with("__8wekyb3d8bbwe")
        && components.next() == Some("winget.exe")
        && components.next().is_none()
}

fn blocked_uninstaller_host(executable: &Path) -> bool {
    let Some(name) = executable
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
    else {
        return true;
    };
    matches!(
        name.as_str(),
        "cmd.exe"
            | "cscript.exe"
            | "mshta.exe"
            | "powershell.exe"
            | "pwsh.exe"
            | "regsvr32.exe"
            | "rundll32.exe"
            | "wscript.exe"
    )
}

pub(super) fn expand_environment_path(value: &str) -> String {
    command::expand_environment_variables(value, |name| env::var(name).ok())
}

fn io_error(error: std::io::Error) -> ApplicationUninstallPlatformError {
    ApplicationUninstallPlatformError::NativeFailure(
        error
            .raw_os_error()
            .and_then(|code| u32::try_from(code).ok())
            .unwrap_or(ERROR_GEN_FAILURE),
    )
}

/// Resolves the inbox Windows PowerShell executable from the trusted system
/// directory. Package metadata and the process environment must not be able to
/// redirect privileged uninstall or inventory operations through `PATH`.
pub(super) fn system_powershell_path() -> Result<PathBuf, ApplicationUninstallPlatformError> {
    let powershell = system_directory_path()?
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    powershell
        .is_file()
        .then_some(powershell)
        .ok_or(ApplicationUninstallPlatformError::Unsupported)
}

fn system_directory_path() -> Result<PathBuf, ApplicationUninstallPlatformError> {
    let mut buffer = vec![0_u16; 32_768];
    let length = unsafe {
        GetSystemDirectoryW(
            buffer.as_mut_ptr(),
            u32::try_from(buffer.len()).unwrap_or(u32::MAX),
        )
    };
    if length == 0 || usize::try_from(length).map_or(true, |length| length >= buffer.len()) {
        return Err(ApplicationUninstallPlatformError::NativeFailure(unsafe {
            GetLastError()
        }));
    }
    buffer.truncate(length as usize);
    Ok(PathBuf::from(OsString::from_wide(&buffer)))
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect()
}

fn exit_code_or_fallback(code: Option<i32>) -> u32 {
    match code {
        Some(code) if code != 0 => code as u32,
        _ => ERROR_GEN_FAILURE,
    }
}

fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_prompt_cancellation_is_typed_separately_from_native_failure() {
        assert_eq!(
            elevation_request_error(ERROR_CANCELLED),
            ApplicationUninstallPlatformError::UserCancelled
        );
        assert_eq!(
            elevation_request_error(5),
            ApplicationUninstallPlatformError::NativeFailure(5)
        );
    }

    #[test]
    fn hresult_style_process_exit_code_is_preserved() {
        assert_eq!(exit_code_or_fallback(Some(-1_978_335_107)), 0x8A15_007D);
        assert_eq!(exit_code_or_fallback(None), ERROR_GEN_FAILURE);
        assert_eq!(exit_code_or_fallback(Some(0)), ERROR_GEN_FAILURE);
    }

    #[test]
    fn registered_uninstaller_accepts_both_windows_restart_success_codes() {
        assert_eq!(
            registered_execution_outcome(Some(ERROR_SUCCESS_REBOOT_REQUIRED as i32)),
            Ok(ApplicationUninstallExecutionOutcome::RestartRequired)
        );
        assert_eq!(
            registered_execution_outcome(Some(ERROR_SUCCESS_REBOOT_INITIATED as i32)),
            Ok(ApplicationUninstallExecutionOutcome::RestartRequired)
        );
    }

    #[test]
    fn machine_registered_executable_validation_does_not_infer_elevation_from_path() {
        let directory = env::temp_dir().join(format!(
            "mangodisk-registered-uninstaller-shell-policy-{}",
            std::process::id()
        ));
        let executable = directory.join("unins000.exe");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("fixture directory should be created");
        fs::write(&executable, b"fixture").expect("fixture executable should be created");

        let command = format!(r#""{}""#, executable.display());
        let validated =
            validated_registered_command(&command, "Fixture", ApplicationInstallScope::Machine);
        assert!(matches!(
            validated,
            Some(ValidatedRegisteredCommand::Executable { .. })
        ));

        fs::remove_dir_all(directory).expect("fixture directory should be removed");
    }

    #[test]
    fn battle_net_registered_command_preserves_vendor_arguments() {
        assert_eq!(
            split_registered_command(
                r#""C:\ProgramData\Battle.net\Agent\Blizzard Uninstaller.exe" --lang=enUS --uid=battle.net --displayname="Battle.net""#
            ),
            Some((
                r"C:\ProgramData\Battle.net\Agent\Blizzard Uninstaller.exe".to_string(),
                r#"--lang=enUS --uid=battle.net --displayname="Battle.net""#.to_string()
            ))
        );
    }

    #[test]
    #[ignore = "reads the installed Battle.net machine uninstaller"]
    fn real_battle_net_registration_uses_the_default_shell_policy() {
        let executable = super::super::directories::program_data_directory()
            .expect("ProgramData should be available")
            .join("Battle.net")
            .join("Agent")
            .join("Blizzard Uninstaller.exe");
        assert!(executable.is_file(), "Battle.net uninstaller should exist");
        let command = format!(
            r#""{}" --lang=enUS --uid=battle.net --displayname="Battle.net""#,
            executable.display()
        );
        assert!(registered_uninstall_command_evidence(
            &command,
            "Battle.net",
            ApplicationInstallScope::Machine
        )
        .is_some());
    }

    #[test]
    #[ignore = "reads the installed China Merchants Bank machine uninstaller"]
    fn real_cmb_registration_uses_the_default_shell_policy() {
        let executable = PathBuf::from(env::var_os("WINDIR").expect("WINDIR should be available"))
            .join("SysWOW64")
            .join("CMBPBUninstall.exe");
        assert!(executable.is_file(), "CMB uninstaller should exist");
        assert!(registered_uninstall_command_evidence(
            executable.to_string_lossy().as_ref(),
            "CMBPB40",
            ApplicationInstallScope::Machine
        )
        .is_some());
    }

    #[test]
    fn process_tree_tracker_keeps_descendants_after_intermediate_exit() {
        let mut tracked = HashSet::from([10]);
        extend_process_tree(&mut tracked, &[(20, 10), (30, 20), (40, 99)]);

        assert_eq!(tracked, HashSet::from([10, 20, 30]));
        extend_process_tree(&mut tracked, &[(30, 20), (50, 30)]);
        assert!(tracked.contains(&50));
        assert!(!tracked.contains(&40));
    }

    #[test]
    fn registered_command_parser_preserves_vendor_arguments_without_a_shell() {
        assert_eq!(
            split_registered_command(r#""C:\Program Files\Example\uninstall.exe" /remove /user"#),
            Some((
                r"C:\Program Files\Example\uninstall.exe".to_string(),
                "/remove /user".to_string()
            ))
        );
        assert_eq!(
            split_registered_command(r"C:\Tools\uninstall.exe /remove"),
            Some((r"C:\Tools\uninstall.exe".to_string(), "/remove".to_string()))
        );
    }

    #[test]
    fn registered_command_parser_rejects_missing_executable_boundaries() {
        assert_eq!(split_registered_command(""), None);
        assert_eq!(
            split_registered_command(r#""C:\Tools\uninstall.exe /remove"#),
            None
        );
        assert_eq!(
            split_registered_command(r"C:\Tools\uninstall /remove"),
            None
        );
    }

    #[test]
    fn powershell_file_parser_accepts_only_structured_host_options() {
        let tokens = tokenize_registered_arguments(
            r#"-NoProfile -ExecutionPolicy Bypass -File "C:\Users\developer\tool\uninstall.ps1" --remove user"#,
        )
        .expect("the command should tokenize");
        assert_eq!(
            parse_powershell_file_tokens(&tokens),
            Some((
                r"C:\Users\developer\tool\uninstall.ps1".to_string(),
                vec!["--remove".to_string(), "user".to_string()]
            ))
        );

        let unsafe_tokens = tokenize_registered_arguments(
            r#"-EncodedCommand ZQB2AGkAbAA= -File "C:\Users\developer\tool\uninstall.ps1""#,
        )
        .expect("the command should tokenize");
        assert_eq!(parse_powershell_file_tokens(&unsafe_tokens), None);
    }

    #[test]
    fn powershell_command_parser_rejects_composition_and_trailing_tokens() {
        let tokens = tokenize_registered_arguments(
            r#"-NoProfile -Command "& 'C:\Users\developer\tool\uninstall.ps1' --remove""#,
        )
        .expect("the command should tokenize");
        assert_eq!(
            parse_powershell_command_tokens(&tokens),
            Some((
                r"C:\Users\developer\tool\uninstall.ps1".to_string(),
                vec!["--remove".to_string()]
            ))
        );

        let encoded = tokenize_registered_arguments(
            r#"-EncodedCommand ZQB2AGkAbAA= -Command "& 'C:\Users\developer\tool\uninstall.ps1'""#,
        )
        .expect("the command should tokenize");
        assert_eq!(parse_powershell_command_tokens(&encoded), None);

        let trailing = tokenize_registered_arguments(
            r#"-Command "& 'C:\Users\developer\tool\uninstall.ps1'" unexpected"#,
        )
        .expect("the command should tokenize");
        assert_eq!(parse_powershell_command_tokens(&trailing), None);
    }

    #[test]
    fn powershell_command_parser_accepts_safe_trailing_host_options() {
        let tokens = tokenize_registered_arguments(
            r#"-Command "& 'C:\Users\developer\tool\uninstall.ps1' -PauseOnError" -ExecutionPolicy Bypass"#,
        )
        .expect("the command should tokenize");
        assert_eq!(
            parse_powershell_command_tokens(&tokens),
            Some((
                r"C:\Users\developer\tool\uninstall.ps1".to_string(),
                vec!["-PauseOnError".to_string()]
            ))
        );

        let duplicate = tokenize_registered_arguments(
            r#"-ExecutionPolicy Bypass -Command "& 'C:\Users\developer\tool\uninstall.ps1'" -ExecutionPolicy Bypass"#,
        )
        .expect("the command should tokenize");
        assert_eq!(parse_powershell_command_tokens(&duplicate), None);
    }

    #[test]
    fn shell_and_script_hosts_are_not_valid_uninstaller_executables() {
        for name in [
            "cmd.exe",
            "cscript.exe",
            "mshta.exe",
            "powershell.exe",
            "pwsh.exe",
            "regsvr32.exe",
            "rundll32.exe",
            "wscript.exe",
        ] {
            assert!(blocked_uninstaller_host(Path::new(name)), "{name}");
        }
        assert!(!blocked_uninstaller_host(Path::new("uninstall.exe")));
    }

    #[test]
    fn winget_parser_accepts_only_an_exact_product_code_command() {
        assert_eq!(
            parse_winget_product_code(
                "winget uninstall --product-code Ninja-build.Ninja_Microsoft.Winget.Source_8wekyb3d8bbwe"
            ),
            Some("Ninja-build.Ninja_Microsoft.Winget.Source_8wekyb3d8bbwe".to_string())
        );
        assert_eq!(
            parse_winget_product_code(
                "winget.exe uninstall --product-code Ninja-build.Ninja_Microsoft.Winget.Source_8wekyb3d8bbwe"
            ),
            Some("Ninja-build.Ninja_Microsoft.Winget.Source_8wekyb3d8bbwe".to_string())
        );
        assert_eq!(
            parse_winget_product_code("winget uninstall --id Ninja-build.Ninja --silent"),
            None
        );
        assert_eq!(
            parse_winget_product_code("winget uninstall --product-code Ninja-build.Ninja --force"),
            None
        );
    }

    #[test]
    fn app_execution_alias_parser_requires_the_appinstaller_identity() {
        let buffer = app_execution_alias_buffer(&[
            WINGET_PACKAGE_FAMILY,
            WINGET_AUMID,
            r"C:\Program Files\WindowsApps\Microsoft.DesktopAppInstaller_1.29.280.0_x64__8wekyb3d8bbwe\winget.exe",
            "0",
        ]);
        let alias = parse_app_execution_alias(&buffer).expect("the alias should parse");

        assert_eq!(alias.package_family, WINGET_PACKAGE_FAMILY);
        assert_eq!(alias.application_user_model_id, WINGET_AUMID);
        assert!(trusted_winget_target(
            &alias.target,
            Path::new(r"C:\Program Files")
        ));
    }

    #[test]
    fn app_execution_alias_parser_rejects_an_unknown_binary_version() {
        let mut buffer = app_execution_alias_buffer(&[
            WINGET_PACKAGE_FAMILY,
            WINGET_AUMID,
            r"C:\Program Files\WindowsApps\Microsoft.DesktopAppInstaller_1.29.280.0_x64__8wekyb3d8bbwe\winget.exe",
            "0",
        ]);
        buffer[8..12].copy_from_slice(&4_u32.to_le_bytes());

        assert!(parse_app_execution_alias(&buffer).is_none());
    }

    #[test]
    #[ignore = "reads the current user's winget app execution alias"]
    fn real_winget_alias_resolves_to_the_appinstaller_package() {
        assert!(trusted_winget_path().is_some());
    }

    #[test]
    #[ignore = "reads the current user's winget app execution alias"]
    fn real_winget_product_command_passes_generic_registration_validation() {
        let product_code = "Ninja-build.Ninja_Microsoft.Winget.Source_8wekyb3d8bbwe";
        let validated = validated_registered_command(
            &format!("winget uninstall --product-code {product_code}"),
            product_code,
            ApplicationInstallScope::Machine,
        );

        assert!(matches!(
            validated,
            Some(ValidatedRegisteredCommand::WingetProduct {
                product_code: validated_product
            }) if validated_product == product_code
        ));
    }

    #[test]
    fn winget_target_rejects_paths_outside_the_appinstaller_package() {
        assert!(!trusted_winget_target(
            Path::new(r"C:\Users\developer\bin\winget.exe"),
            Path::new(r"C:\Program Files")
        ));
        assert!(!trusted_winget_target(
            Path::new(
                r"C:\Program Files\WindowsApps\Untrusted.Package_1.0.0_x64__8wekyb3d8bbwe\winget.exe"
            ),
            Path::new(r"C:\Program Files")
        ));
        assert!(!trusted_winget_target(
            Path::new(
                r"C:\Program Files\WindowsApps\Microsoft.DesktopAppInstaller_1.29.280.0_x64__8wekyb3d8bbwe\subdirectory\winget.exe"
            ),
            Path::new(r"C:\Program Files")
        ));
    }

    fn app_execution_alias_buffer(fields: &[&str]) -> Vec<u8> {
        let mut data = 3_u32.to_le_bytes().to_vec();
        data.extend(
            fields
                .iter()
                .flat_map(|field| field.encode_utf16().chain(iter::once(0)))
                .flat_map(u16::to_le_bytes),
        );
        let mut buffer = Vec::with_capacity(8 + data.len());
        buffer.extend_from_slice(&IO_REPARSE_TAG_APPEXECLINK.to_le_bytes());
        buffer.extend_from_slice(
            &u16::try_from(data.len())
                .expect("test alias data should fit")
                .to_le_bytes(),
        );
        buffer.extend_from_slice(&0_u16.to_le_bytes());
        buffer.append(&mut data);
        buffer
    }

    #[test]
    #[ignore = "requires the MangoDisk per-user MSI fixture to be installed explicitly"]
    fn real_fixture_reports_current_user_scope() {
        assert_eq!(
            msi_install_scope("{9627E855-337D-45EC-A2D9-CBB92B447399}")
                .expect("the MSI context query should succeed"),
            Some(ApplicationInstallScope::CurrentUser)
        );
    }

    #[test]
    #[ignore = "requires the MangoDisk per-user MSI fixture to be installed explicitly"]
    fn real_fixture_reports_installed_state() {
        let registration = ApplicationUninstallRegistration::WindowsMsi {
            product_code: "{9627E855-337D-45EC-A2D9-CBB92B447399}".to_string(),
            scope: ApplicationInstallScope::CurrentUser,
            estimated_bytes: 36 * 1024,
        };
        assert_eq!(
            registration_state(&registration).expect("the MSI state query should succeed"),
            ApplicationUninstallRegistrationState::Installed
        );
    }

    #[test]
    #[ignore = "uninstalls only the explicitly installed MangoDisk-owned MSI fixture"]
    fn real_fixture_uninstalls_and_verifies_absence() {
        let registration = ApplicationUninstallRegistration::WindowsMsi {
            product_code: "{9627E855-337D-45EC-A2D9-CBB92B447399}".to_string(),
            scope: ApplicationInstallScope::CurrentUser,
            estimated_bytes: 36 * 1024,
        };
        assert_eq!(
            registration_state(&registration).expect("the MSI state query should succeed"),
            ApplicationUninstallRegistrationState::Installed
        );
        assert!(matches!(
            execute_registration(&registration).expect("the MSI fixture should uninstall"),
            ApplicationUninstallExecutionOutcome::Completed
                | ApplicationUninstallExecutionOutcome::RestartRequired
        ));
        assert_eq!(
            registration_state(&registration).expect("the MSI state query should succeed"),
            ApplicationUninstallRegistrationState::Absent
        );
    }

    #[test]
    #[ignore = "uninstalls only the explicitly named current-user Scoop fixture"]
    fn real_scoop_fixture_uninstalls_and_verifies_absence() {
        let package_name = std::env::var("MANGODISK_TEST_SCOOP_UNINSTALL_PACKAGE")
            .expect("set MANGODISK_TEST_SCOOP_UNINSTALL_PACKAGE to a disposable Scoop package");
        let install_root = package_locations::scoop_roots()
            .into_iter()
            .find(|root| {
                root.scope == ApplicationInstallScope::CurrentUser
                    && root
                        .path
                        .join("apps")
                        .join(&package_name)
                        .join("current")
                        .is_dir()
            })
            .expect("the current-user Scoop fixture should be installed")
            .path;
        let package_current = install_root
            .join("apps")
            .join(&package_name)
            .join("current");
        let registration = ApplicationUninstallRegistration::WindowsScoop {
            package_name,
            scope: ApplicationInstallScope::CurrentUser,
            package_marker_digest: package_evidence::file_set_digest(&[
                &package_current.join("install.json"),
                &package_current.join("manifest.json"),
            ])
            .expect("the package marker should be readable"),
            scoop_script_digest: package_evidence::file_set_digest(&[&scoop_script_path(
                &install_root,
            )])
            .expect("the Scoop script should be readable"),
            install_root,
            estimated_bytes: 0,
        };

        assert_eq!(
            registration_state(&registration).expect("the Scoop state query should succeed"),
            ApplicationUninstallRegistrationState::Installed
        );
        assert_eq!(
            execute_registration(&registration).expect("the Scoop fixture should uninstall"),
            ApplicationUninstallExecutionOutcome::Completed
        );
        assert_eq!(
            registration_state(&registration).expect("the Scoop state query should succeed"),
            ApplicationUninstallRegistrationState::Absent
        );
    }
}
