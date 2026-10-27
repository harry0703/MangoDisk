//! Installation is the only mutation boundary; runtime IPC exposes read-only facts.
use super::{cpu, security};
use crate::artifacts::{Artifact, AMD_ZEN_MODULE, INTEL_MODULE, PAWNIO_INSTALLER};
use mangodisk_privileged_protocol::SERVICE_NAME;
use sha2::{Digest, Sha256};
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use windows_service::{
    service::{
        ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState,
        ServiceType,
    },
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use windows_sys::Win32::{
    Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
        WINTRUST_FILE_INFO, WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOKE_NONE,
        WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    },
    Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION,
    },
    Storage::FileSystem::{
        CreateDirectoryW, GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
        VS_FIXEDFILEINFO,
    },
};

pub const EXECUTABLE_NAME: &str = "MangoDiskPrivileged.exe";
pub const REBOOT_EXIT_CODE: u32 = 3010;
pub const BUSY_EXIT_CODE: u32 = 1618;
pub const VERIFICATION_EXIT_CODE: u32 = 13;
pub const DRIVER_REMOVAL_EXIT_CODE: u32 = 1603;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn hardware_supported() -> bool {
    cpu::hardware_supported()
}
pub fn temperature_sensor() -> Option<crate::cpu_temperature::CpuTemperatureSensor> {
    cpu::sensor()
}
pub fn install_directory() -> io::Result<PathBuf> {
    Ok(security::program_files()?.join("MangoDiskPrivileged"))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverDependency {
    Ready,
    Missing,
    Incompatible,
}

pub fn driver_available() -> bool {
    matches!(driver_dependency(), Ok(DriverDependency::Ready))
}

pub fn driver_dependency() -> io::Result<DriverDependency> {
    let root = security::program_files()?.join("PawnIO");
    let path = root.join("PawnIOLib.dll");
    for path in [&root, &path] {
        if let Err(error) = security::reject_reparse(path) {
            return if error.kind() == io::ErrorKind::NotFound {
                Ok(DriverDependency::Missing)
            } else {
                Err(error)
            };
        }
    }
    // An installed but untrusted library must not be treated as a missing driver.
    verify_signature(&path)?;
    Ok(if minimum_driver_version(&path).is_ok() {
        DriverDependency::Ready
    } else {
        DriverDependency::Incompatible
    })
}

fn minimum_driver_version(path: &Path) -> io::Result<()> {
    let path = security::wide(path);
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut()) };
    if size == 0 || size > 1024 * 1024 {
        return Err(io::Error::other("PawnIO library version unavailable"));
    }
    let mut bytes = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size, bytes.as_mut_ptr().cast()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut value = std::ptr::null_mut();
    let mut length = 0;
    if unsafe {
        VerQueryValueW(
            bytes.as_ptr().cast(),
            security::wide("\\").as_ptr(),
            &mut value,
            &mut length,
        )
    } == 0
        || value.is_null()
        || length < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32
    {
        return Err(io::Error::other("PawnIO library version malformed"));
    }
    let version = unsafe { std::ptr::read_unaligned(value.cast::<VS_FIXEDFILEINFO>()) };
    if version.dwSignature != 0xfeef04bd
        || (
            version.dwFileVersionMS >> 16,
            version.dwFileVersionMS & 0xffff,
        ) < (2, 2)
    {
        return Err(io::Error::other("PawnIO 2.2 or newer is required"));
    }
    Ok(())
}

pub fn verify_artifact(bytes: &[u8], artifact: &Artifact) -> io::Result<()> {
    if bytes.is_empty()
        || bytes.len() as u64 > artifact.maximum_bytes
        || format!("{:x}", Sha256::digest(bytes)) != artifact.sha256
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("component verification failed file={}", artifact.file_name),
        ));
    }
    Ok(())
}
pub(crate) fn read_verified(path: &Path, artifact: &Artifact) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(artifact.maximum_bytes + 1)
        .read_to_end(&mut bytes)?;
    verify_artifact(&bytes, artifact)?;
    Ok(bytes)
}

pub(crate) fn verify_signature(path: &Path) -> io::Result<()> {
    let file = OpenOptions::new().read(true).share_mode(1).open(path)?;
    verify_signature_open(path, &file)
}
pub(super) fn verify_signature_open(path: &Path, file: &File) -> io::Result<()> {
    let path = security::wide(path);
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: path.as_ptr(),
        hFile: file.as_raw_handle(),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let code = unsafe {
        WinVerifyTrust(
            std::ptr::null_mut(),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        )
    };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe {
        WinVerifyTrust(
            std::ptr::null_mut(),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        )
    };
    if code != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            crate::signature_verification::SignatureFailure(code as u32),
        ));
    }
    Ok(())
}

fn protect_directory(directory: &Path) -> io::Result<()> {
    let descriptor =
        security::Descriptor::new("O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)")?;
    if !directory.exists() {
        let attributes = descriptor.attributes();
        if unsafe { CreateDirectoryW(security::wide(directory).as_ptr(), &attributes) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    security::reject_reparse(directory)?;
    if unsafe {
        SetFileSecurityW(
            security::wide(directory).as_ptr(),
            DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION
                | OWNER_SECURITY_INFORMATION,
            descriptor.raw(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn write_protected(directory: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let path = directory.join(name);
    if path.exists() {
        security::reject_reparse(&path)?;
        fs::remove_file(&path)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(path)
}
pub(super) fn stop_service(service: &windows_service::service::Service) -> Result<()> {
    if service.query_status()?.current_state == ServiceState::Stopped {
        return Ok(());
    }
    service.stop()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while service.query_status()?.current_state != ServiceState::Stopped {
        if Instant::now() >= deadline {
            return Err(
                io::Error::new(io::ErrorKind::TimedOut, "privileged service did not stop").into(),
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
pub(crate) fn install(installer: &[u8], module: &[u8], artifact: &Artifact) -> Result<()> {
    let started = Instant::now();
    super::diagnostics::info(format!(
        "service_install_started version={}",
        env!("CARGO_PKG_VERSION")
    ));
    if !hardware_supported() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "verified CPU hardware unavailable",
        )
        .into());
    }
    // Embedded payloads come from the helper image, never a client-supplied path.
    // Validate before touching the driver or the existing service.
    verify_artifact(module, artifact)?;
    let directory = install_directory()?;
    protect_directory(&directory)?;
    let _lock = installation_lock(&directory)?;
    let dependency = driver_dependency().inspect_err(|error| {
        super::diagnostics::error(format!(
            "pawnio_dependency outcome=blocked component=PawnIOLib.dll stage=verification error={error}"
        ));
    })?;
    let installer = if dependency == DriverDependency::Ready {
        super::diagnostics::info("pawnio_dependency outcome=reuse version_minimum=2.2");
        None
    } else {
        verify_artifact(installer, &PAWNIO_INSTALLER)?;
        super::diagnostics::info("pawnio_dependency outcome=install version=2.2.0");
        Some(installer)
    };
    let own_path = std::env::current_exe()?;
    // Keep verification and the bytes installed under one exclusive read
    // handle. A user-writable bootstrap path cannot be swapped between them.
    let mut own_file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&own_path)?;
    if !cfg!(debug_assertions) {
        verify_signature_open(&own_path, &own_file)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    }
    let mut executable = Vec::new();
    (&mut own_file)
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut executable)?;
    if executable.is_empty() || executable.len() > 16 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "privileged executable exceeds installation bounds",
        )
        .into());
    }
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::QUERY_CONFIG
        | ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::DELETE;
    let mut existing = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => Some(service),
        Err(windows_service::Error::Winapi(error)) if error.raw_os_error() == Some(1060) => None,
        Err(error) => return Err(error.into()),
    };
    let was_existing = existing.is_some();
    let was_running = existing
        .as_ref()
        .map(|service| service.query_status())
        .transpose()?
        .is_some_and(|status| status.current_state == ServiceState::Running);
    let mut replacements = Vec::new();
    for (name, bytes) in [
        (EXECUTABLE_NAME, executable.as_slice()),
        (artifact.file_name, module),
    ] {
        for suffix in ["", ".next", ".previous"] {
            let path = directory.join(format!("{name}{suffix}"));
            if path.exists() {
                security::reject_reparse(&path)?;
            }
        }
        replacements.push(crate::replacement::Replacement::prepare(
            &directory, name, bytes,
        )?);
    }
    let result = (|| -> Result<()> {
        if let Some(service) = existing.as_ref() {
            stop_service(service)?;
        }
        if let Some(bytes) = installer {
            let path = write_protected(&directory, PAWNIO_INSTALLER.file_name, bytes)?;
            verify_signature(&path)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            super::diagnostics::info(format!("pawnio_install_started path={}", path.display()));
            let driver_started = Instant::now();
            let mut child = Command::new(&path)
                .args(["-install", "-silent"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let deadline = Instant::now() + Duration::from_secs(120);
            let mut reported_slow = false;
            let status = loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if !reported_slow && Instant::now() >= deadline {
                    reported_slow = true;
                    super::diagnostics::error("pawnio_installation_slow: retaining installation lock until the installer exits");
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            super::diagnostics::info(format!(
                "pawnio_install_completed exit={:?} elapsed_ms={}",
                status.code(),
                driver_started.elapsed().as_millis()
            ));
            fs::remove_file(path)?;
            if status.code() == Some(REBOOT_EXIT_CODE as i32) {
                return Err(io::Error::from_raw_os_error(REBOOT_EXIT_CODE as i32).into());
            }
            if !status.success() || driver_dependency()? != DriverDependency::Ready {
                return Err(io::Error::other(format!(
                    "PawnIO installation failed exit={:?}",
                    status.code()
                ))
                .into());
            }
        }
        for replacement in &mut replacements {
            replacement.apply()?;
        }
        let executable_path = directory.join(EXECUTABLE_NAME);
        let info = ServiceInfo {
            name: OsString::from(SERVICE_NAME),
            display_name: OsString::from("MangoDisk Privileged Service"),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path,
            launch_arguments: vec![],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };
        if let Some(service) = existing.as_ref() {
            service.change_config(&info)?;
        } else {
            existing = Some(manager.create_service(&info, access)?);
        }
        let service = existing.as_ref().expect("service registered");
        service.set_description(
            "Provides optional privileged MangoDisk capabilities through restricted local interfaces.",
        )?;
        service.start::<&str>(&[])?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let state = service.query_status()?.current_state;
            if state == ServiceState::Running {
                break;
            }
            if state == ServiceState::Stopped || Instant::now() >= deadline {
                return Err(io::Error::other("privileged service failed to start").into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    })();
    if let Err(error) = result {
        super::diagnostics::error(format!("service_install_failed stage=deployment elapsed_ms={} error={error}; rollback_started previous_running={was_running}", started.elapsed().as_millis()));
        if let Some(service) = existing.as_ref() {
            if let Err(stop) = stop_service(service) {
                super::diagnostics::error(format!("service_rollback_stop_failed error={stop}"));
            }
        }
        for replacement in replacements.iter_mut().rev() {
            if let Err(rollback) = replacement.rollback() {
                super::diagnostics::error(format!("service_file_rollback_failed error={rollback}"));
            }
        }
        if let Some(service) = existing.as_ref() {
            if was_existing && was_running {
                if let Err(restart) = service.start::<&str>(&[]) {
                    super::diagnostics::error(format!(
                        "service_rollback_restart_failed error={restart}"
                    ));
                }
            } else if !was_existing {
                if let Err(delete) = service.delete() {
                    super::diagnostics::error(format!(
                        "service_rollback_delete_failed error={delete}"
                    ));
                }
            }
        }
        return Err(error);
    }
    for replacement in &mut replacements {
        if let Err(error) = replacement.commit() {
            super::diagnostics::error(format!("service_backup_cleanup_failed error={error}"));
        }
    }
    super::diagnostics::info(format!(
        "service_install_completed outcome=running path={} elapsed_ms={}",
        directory.display(),
        started.elapsed().as_millis()
    ));
    Ok(())
}

pub fn uninstall(remove_driver: bool) -> Result<()> {
    let started = Instant::now();
    super::diagnostics::info(format!(
        "service_remove_started remove_driver={remove_driver}"
    ));
    let directory = install_directory()?;
    ensure_external_uninstaller(&std::env::current_exe()?, &directory)?;
    protect_directory(&directory)?;
    let lock = installation_lock(&directory)?;
    // Inspect the driver and signed vendor tool before removing the app service.
    let driver_removal = if remove_driver {
        Some(super::pawnio_removal::Removal::prepare().map_err(driver_removal_error)?)
    } else {
        None
    };
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    match manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) {
        Ok(service) => {
            stop_service(&service)?;
            service.delete()?;
        }
        Err(windows_service::Error::Winapi(error)) if error.raw_os_error() == Some(1060) => (),
        Err(error) => return Err(error.into()),
    }
    let driver_result = driver_removal
        .map(|removal| removal.execute().map_err(driver_removal_error))
        .transpose();
    if directory.exists() {
        security::reject_reparse(&directory)?;
        for name in [
            EXECUTABLE_NAME,
            INTEL_MODULE.file_name,
            AMD_ZEN_MODULE.file_name,
            PAWNIO_INSTALLER.file_name,
        ] {
            for suffix in ["", ".next", ".previous"] {
                let path = directory.join(format!("{name}{suffix}"));
                if path.exists() {
                    security::reject_reparse(&path)?;
                    fs::remove_file(&path).map_err(|error| {
                        super::diagnostics::error(format!(
                            "service_remove_failed stage=file path={} error={error}",
                            path.display()
                        ));
                        error
                    })?;
                }
            }
        }
        drop(lock);
        fs::remove_file(directory.join("installation.lock"))?;
        fs::remove_dir(&directory).map_err(|error| {
            super::diagnostics::error(format!(
                "service_remove_failed stage=directory path={} error={error}",
                directory.display()
            ));
            error
        })?;
    }
    super::diagnostics::info(format!(
        "service_remove_completed path={} elapsed_ms={} remove_driver={remove_driver} driver_outcome={}",
        directory.display(),
        started.elapsed().as_millis(),
        if driver_result.is_ok() { "completed" } else { "incomplete" }
    ));
    driver_result.map(|_| ())
}

fn driver_removal_error(error: Box<dyn std::error::Error>) -> Box<dyn std::error::Error> {
    let restart = error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.raw_os_error() == Some(REBOOT_EXIT_CODE as i32));
    super::diagnostics::error(format!(
        "pawnio_remove_failed restart_required={restart} error={error}"
    ));
    if crate::signature_verification::native_code(error.as_ref()).is_some() {
        return error;
    }
    io::Error::from_raw_os_error(if restart {
        REBOOT_EXIT_CODE
    } else {
        DRIVER_REMOVAL_EXIT_CODE
    } as i32)
    .into()
}

fn ensure_external_uninstaller(executable: &Path, directory: &Path) -> io::Result<()> {
    let installed = directory.join(EXECUTABLE_NAME);
    if installed.exists() && fs::canonicalize(executable)? == fs::canonicalize(installed)? {
        // Windows cannot delete a running executable. Reject before removing SCM state.
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "run removal from the application's bundled helper, not the installed service image",
        ));
    }
    Ok(())
}

fn installation_lock(directory: &Path) -> io::Result<File> {
    let path = directory.join("installation.lock");
    if path.exists() {
        security::reject_reparse(&path)?;
    }
    // A protected, exclusive file handle serializes installers across users and
    // app instances without exposing a globally named privileged mutex.
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
        .map_err(|error| {
            if matches!(error.raw_os_error(), Some(32 | 33)) {
                io::Error::from_raw_os_error(BUSY_EXIT_CODE as i32)
            } else {
                error
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires Windows trust for the bundled PawnIO installer"]
    fn native_signature_rejects_a_tampered_copy_and_retains_the_removal_status() {
        use crate::signature_verification::native_code;

        let original =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("vendors/pawnio/2.2.0/PawnIO_setup.exe");
        verify_signature(&original).expect("the original vendor signature is trusted");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tampered.exe");
        let mut bytes = fs::read(original).unwrap();
        bytes[0x1000] ^= 1;
        fs::write(&path, bytes).unwrap();
        let error = verify_signature(&path).expect_err("modified signed bytes must be rejected");
        assert_eq!(native_code(&error), Some(0x80096010));
        assert_eq!(
            native_code(driver_removal_error(error.into()).as_ref()),
            Some(0x80096010)
        );
    }

    #[test]
    fn driver_removal_retains_certificate_failures_and_other_removal_outcomes() {
        use crate::signature_verification::{native_code, SignatureFailure, CERT_E_UNTRUSTEDROOT};

        for code in [CERT_E_UNTRUSTEDROOT, 0x80096010] {
            let error = io::Error::new(io::ErrorKind::InvalidData, SignatureFailure(code));
            assert_eq!(
                native_code(driver_removal_error(error.into()).as_ref()),
                Some(code)
            );
        }
        for (error, expected) in [
            (
                io::Error::from_raw_os_error(REBOOT_EXIT_CODE as i32),
                REBOOT_EXIT_CODE,
            ),
            (
                io::Error::other("PawnIO removal incomplete"),
                DRIVER_REMOVAL_EXIT_CODE,
            ),
        ] {
            let error = driver_removal_error(error.into());
            assert_eq!(
                error.downcast_ref::<io::Error>().unwrap().raw_os_error(),
                Some(expected as i32)
            );
        }
    }

    #[test]
    #[ignore = "requires an installed, trusted PawnIO 2.2 or newer dependency"]
    fn native_installed_pawnio_is_reused_after_signature_and_version_checks() {
        assert_eq!(driver_dependency().unwrap(), DriverDependency::Ready);
    }

    #[test]
    fn self_removal_is_rejected_before_service_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let installed = directory.path().join(EXECUTABLE_NAME);
        let bundled = directory.path().join("bundled.exe");
        fs::write(&installed, b"installed").unwrap();
        fs::write(&bundled, b"bundled").unwrap();
        assert_eq!(
            ensure_external_uninstaller(&installed, directory.path())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(ensure_external_uninstaller(&bundled, directory.path()).is_ok());
        fs::remove_file(installed).unwrap();
        assert!(ensure_external_uninstaller(&bundled, directory.path()).is_ok());
    }
    #[test]
    fn component_verification_rejects_tampering_empty_and_oversized_bytes() {
        let artifact = Artifact {
            file_name: "test.bin",
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            maximum_bytes: 3,
        };
        assert!(verify_artifact(b"abc", &artifact).is_ok());
        for bytes in [b"".as_slice(), b"abd", b"abcd"] {
            assert!(verify_artifact(bytes, &artifact).is_err());
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.bin");
        fs::write(&path, b"abcd").unwrap();
        assert!(read_verified(&path, &artifact).is_err());
    }
    #[test]
    fn installation_lock_serializes_mutations_until_the_handle_is_released() {
        let directory = tempfile::tempdir().unwrap();
        let first = installation_lock(directory.path()).unwrap();
        assert_eq!(
            installation_lock(directory.path())
                .unwrap_err()
                .raw_os_error(),
            Some(BUSY_EXIT_CODE as i32)
        );
        drop(first);
        assert!(installation_lock(directory.path()).is_ok());
    }
}
