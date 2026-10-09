//! Changes only a service's configured startup mode, never its running state.
//! Authority comes from a fresh SCM snapshot, not a path supplied by the UI.
use std::io::ErrorKind;

use serde::{Deserialize, Serialize};
use windows::Win32::System::Services::*;
use windows_core::{HSTRING, PCWSTR};
use winreg::{enums::*, RegKey};

use super::services::{artifact_from_config, query_service_config, EnumeratedService};
use crate::{
    PlatformError, PlatformErrorCode, PlatformResult, PlatformStartupChangeRequest,
    PlatformStartupChangeResult, PlatformStartupConfiguredState, PlatformStartupControlCapability,
    PlatformStartupDesiredState,
};

const RESTORE_VALUE: &str = "MangoDiskStartupRestoreV1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreConfig {
    schema_version: u32,
    binary_digest: String,
    delayed: u32,
}

struct ServiceHandle(SC_HANDLE);
impl Drop for ServiceHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn service_key(name: &str, access: u32) -> std::io::Result<RegKey> {
    RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(
        format!(r"SYSTEM\CurrentControlSet\Services\{name}"),
        access | KEY_WOW64_64KEY,
    )
}

pub(super) fn configuration_modified_at(name: &str) -> Option<u64> {
    let time = service_key(name, KEY_READ)
        .ok()?
        .query_info()
        .ok()?
        .last_write_time;
    let ticks = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    (ticks / 10_000).checked_sub(11_644_473_600_000)
}

fn service_name(request: &PlatformStartupChangeRequest) -> PlatformResult<&str> {
    let name = request
        .provider_item_id
        .strip_prefix("service:")
        .unwrap_or_default();
    if request.source_id != "windows.services"
        || name.is_empty()
        || name.len() > 256
        || name.contains(['\\', '/', '\0'])
    {
        return Err(PlatformError::invalid_path("service_change stage=identity"));
    }
    Ok(name)
}

pub(super) fn change(
    request: &PlatformStartupChangeRequest,
) -> PlatformResult<PlatformStartupChangeResult> {
    // Removal is deliberately outside this capability, including forged helper requests.
    if !matches!(
        request.desired_state,
        PlatformStartupDesiredState::Enabled | PlatformStartupDesiredState::Disabled
    ) {
        return Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "service_change stage=unsupported_action",
        ));
    }
    let name = service_name(request)?;
    let manager = ServiceHandle(
        unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
            .map_err(|error| native_error("open_manager", error))?,
    );
    let service = ServiceHandle(
        unsafe {
            OpenServiceW(
                manager.0,
                &HSTRING::from(name),
                SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS | SERVICE_CHANGE_CONFIG,
            )
        }
        .map_err(|error| native_error("open_service", error))?,
    );
    let config = query_service_config(service.0, name)
        .map_err(|_| PlatformError::operation_failed("service_change stage=read_config"))?;
    let mut status = SERVICE_STATUS::default();
    unsafe { QueryServiceStatus(service.0, &mut status) }
        .map_err(|error| native_error("read_status", error))?;
    let current = artifact_from_config(
        &EnumeratedService {
            name: name.to_owned(),
            display_name: request.expected_artifact.display_name.clone(),
            running: status.dwCurrentState == SERVICE_RUNNING,
        },
        &config,
        None,
    );
    if current.control_capability != PlatformStartupControlCapability::ElevationRequired
        || crate::startup_helper::artifact_digest(&current)
            != crate::startup_helper::artifact_digest(&request.expected_artifact)
    {
        return Err(PlatformError::item_changed(
            "service_change stage=preflight",
        ));
    }
    let desired = if request.desired_state == PlatformStartupDesiredState::Enabled {
        PlatformStartupConfiguredState::Enabled
    } else {
        PlatformStartupConfiguredState::Disabled
    };
    if current.configured_state == desired {
        return Ok(PlatformStartupChangeResult {
            previous_state: current.configured_state,
            configured_state: desired,
            verified: true,
        });
    }

    // Store only a versioned digest and the original delay flag under the SCM-owned
    // key. Its administrator ACL prevents an unelevated client from injecting a
    // restore target, and removal of the service naturally removes its backup.
    let key = service_key(name, KEY_QUERY_VALUE | KEY_SET_VALUE)
        .map_err(|error| PlatformError::io("service_change stage=open_backup", &error))?;
    let binary_digest = blake3::hash(config.binary_path.as_bytes())
        .to_hex()
        .to_string();
    let enabling = desired == PlatformStartupConfiguredState::Enabled;
    let delayed = if enabling {
        match key.get_value::<String, _>(RESTORE_VALUE) {
            Ok(value) => restore_delay(&value, &binary_digest, config.delayed.unwrap_or(0))?,
            Err(error) if error.kind() == ErrorKind::NotFound => config.delayed.unwrap_or(0),
            Err(error) => {
                return Err(PlatformError::io(
                    "service_change stage=read_backup",
                    &error,
                ))
            }
        }
    } else {
        let delayed = config.delayed.unwrap_or(0);
        let backup = serde_json::to_string(&RestoreConfig {
            schema_version: 1,
            binary_digest,
            delayed,
        })
        .map_err(|_| PlatformError::operation_failed("service_change stage=encode_backup"))?;
        key.set_value(RESTORE_VALUE, &backup)
            .map_err(|error| PlatformError::io("service_change stage=write_backup", &error))?;
        delayed
    };
    let start_type = if enabling {
        SERVICE_AUTO_START
    } else {
        SERVICE_DISABLED
    };
    set_start_type(service.0, start_type)
        .map_err(|error| native_error("write_start_type", error).with_possible_side_effects())?;
    let after_start_type = query_service_config(service.0, name).map_err(|_| {
        PlatformError::operation_failed("service_change stage=verify_read")
            .with_possible_side_effects()
    })?;
    // Avoid materializing a redundant DelayedAutoStart=0 registry value when
    // Windows already retained the intended setting during the start-type change.
    if enabling && after_start_type.delayed != Some(delayed) {
        let delay = SERVICE_DELAYED_AUTO_START_INFO {
            fDelayedAutostart: (delayed != 0).into(),
        };
        if let Err(error) = unsafe {
            ChangeServiceConfig2W(
                service.0,
                SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
                Some((&delay as *const SERVICE_DELAYED_AUTO_START_INFO).cast()),
            )
        } {
            // A failed delay restore must not leave an apparently successful enable.
            // Return to disabled so the retained backup can be retried. Even a
            // verified rollback remains a possible mutation for Core reconciliation.
            let rollback_verified = set_start_type(service.0, SERVICE_DISABLED).is_ok()
                && query_service_config(service.0, name)
                    .is_ok_and(|value| value.start_type == SERVICE_DISABLED);
            let failure = native_error("restore_delay", error);
            let rollback = if rollback_verified {
                "verified"
            } else {
                "unverified"
            };
            return Err(PlatformError::new(
                failure.code(),
                format!("{} rollback={rollback}", failure.diagnostic()),
            )
            .with_possible_side_effects());
        }
    }
    let verified = query_service_config(service.0, name).map_err(|_| {
        PlatformError::operation_failed("service_change stage=verify_read")
            .with_possible_side_effects()
    })?;
    if verified.start_type != start_type || (enabling && verified.delayed != Some(delayed)) {
        return Err(
            PlatformError::operation_failed("service_change stage=verify_mismatch")
                .with_possible_side_effects(),
        );
    }
    if enabling {
        // A leftover backup cannot undo a verified configuration. A later disable
        // overwrites it, so cleanup failure must not falsely report the switch failed.
        if let Err(error) = key.delete_value(RESTORE_VALUE) {
            if error.kind() != ErrorKind::NotFound {
                log::warn!(
                    "service_change stage=cleanup_backup os_error={:?}",
                    error.raw_os_error()
                );
            }
        }
    }
    Ok(PlatformStartupChangeResult {
        previous_state: current.configured_state,
        configured_state: desired,
        verified: true,
    })
}

fn set_start_type(service: SC_HANDLE, start_type: SERVICE_START_TYPE) -> windows_core::Result<()> {
    unsafe {
        ChangeServiceConfigW(
            service,
            ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
            start_type,
            SERVICE_ERROR(SERVICE_NO_CHANGE),
            PCWSTR::null(),
            PCWSTR::null(),
            None,
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
        )
    }
}

fn restore_delay(value: &str, binary_digest: &str, default: u32) -> PlatformResult<u32> {
    let backup: RestoreConfig = if value.len() <= 1024 {
        serde_json::from_str(value).ok()
    } else {
        None
    }
    .ok_or_else(|| {
        PlatformError::new(
            PlatformErrorCode::InvalidData,
            "service_change stage=invalid_backup",
        )
    })?;
    if backup.schema_version != 1 || backup.delayed > 1 {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "service_change stage=backup_version",
        ));
    }
    // An application update may replace the binary while disabled. Enabling still
    // means automatic startup, but must not replay another installation's delay.
    Ok(if backup.binary_digest == binary_digest {
        backup.delayed
    } else {
        default
    })
}

fn native_error(stage: &'static str, error: windows_core::Error) -> PlatformError {
    let code = match error.code().0 as u32 {
        0x8007_0005 => PlatformErrorCode::AccessDenied,
        0x8007_0424 | 0x8007_0430 => PlatformErrorCode::ItemChanged,
        _ => PlatformErrorCode::OperationFailed,
    };
    PlatformError::new(
        code,
        format!(
            "service_change stage={stage} hresult={:08x}",
            error.code().0 as u32
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    #[ignore = "toggles explicitly named real third-party services in a snapshot-backed Windows VM"]
    fn actual_existing_services_restore_configuration_without_stopping() {
        let names = std::env::var("MANGODISK_TEST_SERVICE_NAMES")
            .expect("explicit test service names are required");
        for name in names.split(';').filter(|name| !name.is_empty()) {
            let manager = ServiceHandle(
                unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
                    .unwrap(),
            );
            let service = ServiceHandle(
                unsafe {
                    OpenServiceW(
                        manager.0,
                        &HSTRING::from(name),
                        SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS | SERVICE_CHANGE_CONFIG,
                    )
                }
                .unwrap(),
            );
            let original = query_service_config(service.0, name).unwrap();
            assert!(matches!(
                original.start_type,
                SERVICE_AUTO_START | SERVICE_DISABLED
            ));
            let key = service_key(name, KEY_QUERY_VALUE | KEY_SET_VALUE).unwrap();
            let backup = match key.get_raw_value(RESTORE_VALUE) {
                Ok(value) => Some(value),
                Err(error) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => panic!("cannot snapshot restore metadata: {:?}", error.kind()),
            };
            let mut status = SERVICE_STATUS::default();
            unsafe { QueryServiceStatus(service.0, &mut status) }.unwrap();
            let was_running = status.dwCurrentState;
            let restore = ExistingRestore {
                name: name.to_owned(),
                service,
                key,
                original,
                backup,
            };
            let states = if restore.original.start_type == SERVICE_AUTO_START {
                [
                    PlatformStartupDesiredState::Disabled,
                    PlatformStartupDesiredState::Enabled,
                ]
            } else {
                [
                    PlatformStartupDesiredState::Enabled,
                    PlatformStartupDesiredState::Disabled,
                ]
            };
            for desired in states {
                let config = query_service_config(restore.service.0, name).unwrap();
                let artifact = artifact_from_config(
                    &EnumeratedService {
                        name: name.into(),
                        display_name: name.into(),
                        running: was_running == SERVICE_RUNNING,
                    },
                    &config,
                    None,
                );
                assert_eq!(
                    artifact.control_capability,
                    PlatformStartupControlCapability::ElevationRequired
                );
                let result = super::super::helper_change(
                    "windows.services",
                    &artifact.provider_item_id,
                    &crate::startup_helper::artifact_digest(&artifact),
                    desired,
                )
                .expect("native helper change must succeed");
                assert!(result.verified);
                println!("service_fixture desired={desired:?} verified=true");
            }
            let verified = query_service_config(restore.service.0, name).unwrap();
            assert_eq!(verified.start_type, restore.original.start_type);
            assert_eq!(verified.delayed, restore.original.delayed);
            unsafe { QueryServiceStatus(restore.service.0, &mut status) }.unwrap();
            assert_eq!(
                status.dwCurrentState, was_running,
                "a startup switch must not stop a service"
            );
        }
    }

    struct ExistingRestore {
        name: String,
        service: ServiceHandle,
        key: RegKey,
        original: super::super::services::ServiceConfig,
        backup: Option<winreg::RegValue<'static>>,
    }
    impl Drop for ExistingRestore {
        fn drop(&mut self) {
            unsafe {
                let _ = ChangeServiceConfigW(
                    self.service.0,
                    ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                    self.original.start_type,
                    SERVICE_ERROR(SERVICE_NO_CHANGE),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    None,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    PCWSTR::null(),
                );
                if self.original.start_type == SERVICE_AUTO_START
                    && query_service_config(self.service.0, &self.name)
                        .is_ok_and(|config| config.delayed != self.original.delayed)
                {
                    let delay = SERVICE_DELAYED_AUTO_START_INFO {
                        fDelayedAutostart: (self.original.delayed == Some(1)).into(),
                    };
                    let _ = ChangeServiceConfig2W(
                        self.service.0,
                        SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
                        Some((&delay as *const SERVICE_DELAYED_AUTO_START_INFO).cast()),
                    );
                }
            }
            if let Some(backup) = &self.backup {
                let _ = self.key.set_raw_value(RESTORE_VALUE, backup);
            } else {
                let _ = self.key.delete_value(RESTORE_VALUE);
            }
        }
    }
    #[test]
    fn restore_records_reject_corruption_and_preserve_delay() {
        let value = serde_json::to_string(&RestoreConfig {
            schema_version: 1,
            binary_digest: "fixture".into(),
            delayed: 1,
        })
        .unwrap();
        assert_eq!(restore_delay(&value, "fixture", 0).unwrap(), 1);
        assert_eq!(restore_delay(&value, "updated", 0).unwrap(), 0);
        for invalid in [
            "{}",
            "garbage",
            "{\"schema_version\":2,\"binary_digest\":\"fixture\",\"delayed\":1}",
        ] {
            assert!(restore_delay(invalid, "fixture", 0).is_err());
        }
    }

    #[test]
    #[ignore = "creates isolated SCM fixtures; requires an elevated Windows test VM"]
    fn actual_service_switch_restores_delay_and_rejects_stale_requests() {
        for delayed in [0, 1] {
            let fixture = Fixture::create(delayed);
            let enabled = fixture.request(PlatformStartupDesiredState::Disabled);
            let disabled = super::super::helper_change(
                "windows.services",
                &enabled.provider_item_id,
                &crate::startup_helper::artifact_digest(&enabled.expected_artifact),
                enabled.desired_state,
            )
            .expect("the helper must disable the fixture");
            assert!(disabled.verified);
            assert_eq!(
                disabled.configured_state,
                PlatformStartupConfiguredState::Disabled
            );
            assert_eq!(
                change(&enabled).unwrap_err().code(),
                PlatformErrorCode::ItemChanged
            );
            let restore = fixture.request(PlatformStartupDesiredState::Enabled);
            assert!(change(&restore).expect("restoration must succeed").verified);
            assert_eq!(
                query_service_config(fixture.service.0, &fixture.name)
                    .unwrap()
                    .delayed,
                Some(delayed)
            );
            assert!(fixture.key().get_raw_value(RESTORE_VALUE).is_err());
            let removal = fixture.request(PlatformStartupDesiredState::Removed);
            assert_eq!(
                change(&removal).unwrap_err().code(),
                PlatformErrorCode::Unsupported
            );
            let fresh = fixture.request(PlatformStartupDesiredState::Disabled);
            assert!(change(&fresh).unwrap().verified);
            fixture
                .key()
                .set_value(RESTORE_VALUE, &"invalid fixture backup")
                .unwrap();
            let corrupt = fixture.request(PlatformStartupDesiredState::Enabled);
            assert_eq!(
                change(&corrupt).unwrap_err().code(),
                PlatformErrorCode::InvalidData
            );
            assert_eq!(
                query_service_config(fixture.service.0, &fixture.name)
                    .unwrap()
                    .start_type,
                SERVICE_DISABLED
            );
            fixture.key().delete_value(RESTORE_VALUE).unwrap();
            assert!(
                change(&fixture.request(PlatformStartupDesiredState::Enabled))
                    .unwrap()
                    .verified
            );
        }
    }

    #[test]
    #[ignore = "creates temporary services; requires administrator rights and a built MangoDisk helper"]
    fn actual_service_access_denied_preserves_configuration() {
        assert!(std::env::var_os("MANGODISK_TEST_ELEVATION_HELPER_EXE").is_some());
        struct Evidence(std::sync::Mutex<Vec<String>>);
        impl log::Log for Evidence {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("{} {}", record.level(), record.args()));
            }
            fn flush(&self) {}
        }
        static EVIDENCE: Evidence = Evidence(std::sync::Mutex::new(Vec::new()));
        log::set_logger(&EVIDENCE).unwrap();
        log::set_max_level(log::LevelFilter::Info);
        let fixture = Fixture::create(0);
        // Deny only configuration changes, even to administrators. The existing handle
        // retains cleanup access; the inert fixture is never started.
        let status = std::process::Command::new("sc.exe")
            .args([
                "sdset",
                &fixture.name,
                "D:(D;;DC;;;WD)(A;;GA;;;SY)(A;;GA;;;BA)",
            ])
            .status()
            .unwrap();
        assert!(status.success());
        let request = fixture.request(PlatformStartupDesiredState::Disabled);
        let error = change(&request).unwrap_err();
        assert_eq!(error.code(), PlatformErrorCode::AccessDenied);
        let helper_error =
            crate::startup_helper::change_with_privileges(&request, None).unwrap_err();
        assert_eq!(helper_error.code(), PlatformErrorCode::AccessDenied);
        assert_eq!(
            helper_error.mutation_state(),
            crate::PlatformMutationState::NotAttempted
        );
        let writable = Fixture::create(0);
        assert!(
            crate::startup_helper::change_with_privileges(
                &writable.request(PlatformStartupDesiredState::Disabled),
                None,
            )
            .unwrap()
            .verified
        );
        let scan = super::super::services::scan(&crate::PlatformCancellation::new(|| false));
        assert!(!scan.items.is_empty());
        let evidence = EVIDENCE.0.lock().unwrap();
        assert!(!evidence
            .iter()
            .any(|line| line.contains("windows_startup_service_target ")));
        assert!(evidence
            .iter()
            .any(|line| line.contains("windows_startup_service_controls ")));
        assert!(evidence.iter().any(
            |line| line.contains("startup_helper_item_failed source_id=")
                && line.contains(&fixture.name.to_lowercase())
        ));
        assert_eq!(
            evidence
                .iter()
                .filter(|line| line.contains("windows_elevation_authorization_requested "))
                .count(),
            1
        );
        assert_eq!(
            evidence
                .iter()
                .filter(|line| line.contains("windows_elevation_session_reused "))
                .count(),
            1
        );
        for line in evidence.iter().filter(|line| {
            line.contains("startup_helper_item_failed ")
                || line.contains("windows_elevation_")
                || line.contains("windows_startup_service_controls ")
        }) {
            println!("{line}");
        }
        drop(evidence);
        assert_eq!(
            error.mutation_state(),
            crate::PlatformMutationState::NotAttempted
        );
        assert_eq!(
            query_service_config(fixture.service.0, &fixture.name)
                .unwrap()
                .start_type,
            SERVICE_AUTO_START
        );
        assert!(fixture.key().get_raw_value(RESTORE_VALUE).is_err());
        println!(
            "service_name={:?} reason={:?} mutation_state={:?} configuration_unchanged=true",
            fixture.name,
            error.code(),
            error.mutation_state()
        );
    }

    struct Fixture {
        name: String,
        service: ServiceHandle,
        directory: std::path::PathBuf,
    }

    impl Fixture {
        fn create(delayed: u32) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let name = format!("MangoDiskServiceFixture{nonce}");
            let directory = std::env::temp_dir().join(&name);
            std::fs::create_dir(&directory).unwrap();
            let binary = directory.join("inert service.exe");
            // The fixture is never started. No real application or service process
            // is stopped by this test; only the SCM configuration is exercised.
            std::fs::write(&binary, b"inert MangoDisk fixture").unwrap();
            let manager = ServiceHandle(
                unsafe {
                    OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CREATE_SERVICE)
                }
                .unwrap(),
            );
            let command = HSTRING::from(format!("\"{}\"", binary.display()));
            let service = ServiceHandle(
                unsafe {
                    CreateServiceW(
                        manager.0,
                        &HSTRING::from(&name),
                        &HSTRING::from(&name),
                        SERVICE_ALL_ACCESS,
                        SERVICE_WIN32_OWN_PROCESS,
                        SERVICE_AUTO_START,
                        SERVICE_ERROR_NORMAL,
                        &command,
                        PCWSTR::null(),
                        None,
                        PCWSTR::null(),
                        PCWSTR::null(),
                        PCWSTR::null(),
                    )
                }
                .unwrap(),
            );
            let fixture = Self {
                name,
                service,
                directory,
            };
            let delay = SERVICE_DELAYED_AUTO_START_INFO {
                fDelayedAutostart: (delayed != 0).into(),
            };
            unsafe {
                ChangeServiceConfig2W(
                    fixture.service.0,
                    SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
                    Some((&delay as *const SERVICE_DELAYED_AUTO_START_INFO).cast()),
                )
            }
            .unwrap();
            fixture
        }

        fn key(&self) -> RegKey {
            service_key(&self.name, KEY_READ | KEY_SET_VALUE).unwrap()
        }

        fn request(
            &self,
            desired_state: PlatformStartupDesiredState,
        ) -> PlatformStartupChangeRequest {
            let config = query_service_config(self.service.0, &self.name).unwrap();
            let artifact = artifact_from_config(
                &EnumeratedService {
                    name: self.name.clone(),
                    display_name: self.name.clone(),
                    running: false,
                },
                &config,
                None,
            );
            assert_eq!(
                artifact.control_capability,
                PlatformStartupControlCapability::ElevationRequired
            );
            PlatformStartupChangeRequest {
                source_id: "windows.services".into(),
                provider_item_id: artifact.provider_item_id.clone(),
                expected_artifact: artifact,
                desired_state,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteService(self.service.0);
            }
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}
