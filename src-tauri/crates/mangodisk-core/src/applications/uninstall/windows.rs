use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use mangodisk_platform::{
    current_platform, ApplicationInstallScope, ApplicationUninstallExecutionOutcome,
    ApplicationUninstallPlatformError, ApplicationUninstallRegistration,
    ApplicationUninstallRegistrationState, Platform,
};

use crate::filesystem::metadata::now_ms;

use super::models::{
    ApplicationUninstallActionReason, ApplicationUninstallCandidate, ApplicationUninstallComponent,
    ApplicationUninstallComponentKind, ApplicationUninstallComponentSummary,
    ApplicationUninstallInspection, ApplicationUninstallInstallerKind, ApplicationUninstallRisk,
    APPLICATION_UNINSTALL_INSPECTION_SCHEMA_VERSION,
};

const UNINSTALL_RESULT_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(super) enum ApplicationUninstallExecution {
    Completed(ApplicationUninstallExecutionOutcome),
    Cancelled,
    Detached,
}

pub(super) fn summarize_candidate(
    candidate: &ApplicationUninstallCandidate,
) -> (Vec<ApplicationUninstallComponentSummary>, bool) {
    let Some(registration) = candidate.uninstall_registration.as_ref() else {
        return (Vec::new(), false);
    };
    let (native_identity, estimated_bytes) = registration_summary(registration);
    (
        vec![ApplicationUninstallComponentSummary {
            component_id: component_id(&candidate.application_id, native_identity),
            kind: ApplicationUninstallComponentKind::NativeInstaller,
            risk: ApplicationUninstallRisk::Required,
            path: None,
            bytes: estimated_bytes,
            file_count: 1,
            default_selected: true,
        }],
        // Windows owns the registered uninstaller, but a registry entry alone
        // does not prove that every cache or preference path belongs to this
        // application. Keep the native uninstall action available and leave
        // unverified associated data untouched.
        false,
    )
}

pub(super) fn inspect_candidate(
    candidate: &ApplicationUninstallCandidate,
    catalog_revision: &str,
    started: Instant,
) -> Result<ApplicationUninstallInspection, String> {
    if !candidate.capability.supports_execution() {
        return Err("Windows application is not ready for uninstall inspection".to_string());
    }
    let registration = candidate
        .uninstall_registration
        .as_ref()
        .ok_or_else(|| "Windows uninstall registration is unavailable".to_string())?;
    if current_platform()
        .application_uninstall_registration_state(registration)
        .map_err(|error| format!("Windows Installer state query failed: {error}"))?
        != ApplicationUninstallRegistrationState::Installed
    {
        return Err("Windows uninstall registration changed during inspection".to_string());
    }

    let (native_identity, estimated_bytes) = registration_summary(registration);
    let snapshot_fingerprint = registration_fingerprint(&candidate.application_id, registration);
    let component = ApplicationUninstallComponent {
        component_id: component_id(&candidate.application_id, native_identity),
        kind: ApplicationUninstallComponentKind::NativeInstaller,
        risk: ApplicationUninstallRisk::Required,
        path: None,
        bytes: estimated_bytes,
        file_count: 1,
        default_selected: true,
        snapshot_fingerprint,
    };
    Ok(ApplicationUninstallInspection {
        schema_version: APPLICATION_UNINSTALL_INSPECTION_SCHEMA_VERSION,
        inspected_at_ms: now_ms(),
        application_id: candidate.application_id.clone(),
        application_name: candidate.name.clone(),
        primary_identifier: candidate.primary_identifier.clone(),
        platform: candidate.platform,
        installer_kind: Some(installer_kind(registration)),
        capability: candidate.capability,
        catalog_revision: catalog_revision.to_string(),
        total_bytes: component.bytes,
        default_selected_bytes: component.bytes,
        components: vec![component],
        elapsed_ms: started.elapsed().as_millis() as u64,
        uninstall_registration: Some(registration.clone()),
    })
}

pub(super) fn execute_registration(
    inspection: &ApplicationUninstallInspection,
    plan_id: &str,
    cancellation: Arc<AtomicBool>,
) -> Result<ApplicationUninstallExecution, ApplicationUninstallActionReason> {
    let registration = inspection
        .uninstall_registration
        .as_ref()
        .ok_or(ApplicationUninstallActionReason::ComponentUnavailable)?
        .clone();
    let application_id = inspection.application_id.clone();
    let plan_id = plan_id.to_string();
    let (sender, receiver) = mpsc::sync_channel(1);
    // Windows uninstallers may display UI and wait indefinitely for user
    // input. Run the platform wait on a detached worker so a cooperative Core
    // cancellation can release the batch without terminating the OS-owned
    // process or keeping MangoDisk's progress dialog blocked.
    thread::Builder::new()
        .name("application-uninstall-wait".to_string())
        .spawn(move || {
            let started = Instant::now();
            log::info!("application_uninstall_native_started application_id={application_id} plan_id={plan_id}");
            let result = match current_platform()
                .execute_application_uninstall_registration(&registration)
            {
                Ok(outcome) => Ok(ApplicationUninstallExecution::Completed(outcome)),
                Err(ApplicationUninstallPlatformError::UserCancelled) => {
                    log::info!(
                        "application_uninstall_native_execution_cancelled reason=elevation_prompt"
                    );
                    Ok(ApplicationUninstallExecution::Cancelled)
                }
                Err(error) => {
                    log::warn!(
                        "application_uninstall_native_execution_failed application_id={application_id} plan_id={plan_id} platform_error={} native_code={}",
                        error.stable_code(),
                        error
                            .native_code()
                            .map_or_else(|| "none".to_string(), |code| code.to_string())
                    );
                    Err(map_platform_error(error))
                }
            };
            log::info!(
                "application_uninstall_native_finished application_id={application_id} plan_id={plan_id} outcome={} elapsed_ms={}",
                match &result {
                    Ok(ApplicationUninstallExecution::Completed(_)) => "completed",
                    Ok(ApplicationUninstallExecution::Cancelled) => "cancelled",
                    Ok(ApplicationUninstallExecution::Detached) => "detached",
                    Err(_) => "failed",
                }, started.elapsed().as_millis()
            );
            let _ = sender.send(result);
        })
        .map_err(|error| {
            log::warn!(
                "application_uninstall_wait_worker_start_failed error_digest={}",
                blake3::hash(error.to_string().as_bytes()).to_hex()
            );
            ApplicationUninstallActionReason::NativeInstallerFailed
        })?;
    await_execution_result(receiver, cancellation)
}

fn await_execution_result(
    receiver: Receiver<Result<ApplicationUninstallExecution, ApplicationUninstallActionReason>>,
    cancellation: Arc<AtomicBool>,
) -> Result<ApplicationUninstallExecution, ApplicationUninstallActionReason> {
    loop {
        match receiver.recv_timeout(UNINSTALL_RESULT_POLL_INTERVAL) {
            Ok(result) => return result,
            Err(RecvTimeoutError::Timeout) if cancellation.load(Ordering::Relaxed) => {
                log::info!("application_uninstall_wait_detached reason=user_cancelled");
                return Ok(ApplicationUninstallExecution::Detached);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(ApplicationUninstallActionReason::NativeInstallerFailed);
            }
        }
    }
}

fn map_platform_error(
    error: ApplicationUninstallPlatformError,
) -> ApplicationUninstallActionReason {
    match error {
        ApplicationUninstallPlatformError::RegistrationChanged => {
            ApplicationUninstallActionReason::ComponentChanged
        }
        ApplicationUninstallPlatformError::Unsupported
        | ApplicationUninstallPlatformError::RequiresElevation
        | ApplicationUninstallPlatformError::UserCancelled
        | ApplicationUninstallPlatformError::NativeFailure(_) => {
            ApplicationUninstallActionReason::NativeInstallerFailed
        }
    }
}

fn component_id(application_id: &str, native_identity: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-windows-native-component-id-v2");
    hasher.update(application_id.as_bytes());
    hasher.update(native_identity.as_bytes());
    format!("component-{}", &hasher.finalize().to_hex()[..24])
}

fn registration_fingerprint(
    application_id: &str,
    registration: &ApplicationUninstallRegistration,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-windows-native-component-v2");
    hasher.update(application_id.as_bytes());
    match registration {
        ApplicationUninstallRegistration::WindowsMsi {
            product_code,
            scope,
            estimated_bytes,
        } => {
            hasher.update(b"msi");
            hasher.update(product_code.as_bytes());
            hasher.update(match scope {
                ApplicationInstallScope::CurrentUser => b"current-user",
                ApplicationInstallScope::Machine => b"machine",
            });
            hasher.update(&estimated_bytes.to_le_bytes());
        }
        ApplicationUninstallRegistration::WindowsAppx {
            package_family_name,
            package_full_name,
            estimated_bytes,
        } => {
            hasher.update(b"appx");
            hasher.update(package_family_name.as_bytes());
            hasher.update(package_full_name.as_bytes());
            hasher.update(&estimated_bytes.to_le_bytes());
        }
        ApplicationUninstallRegistration::WindowsScoop {
            package_name,
            scope,
            install_root,
            package_marker_digest,
            scoop_script_digest,
            estimated_bytes,
        } => {
            hasher.update(b"scoop");
            hasher.update(package_name.as_bytes());
            hasher.update(match scope {
                ApplicationInstallScope::CurrentUser => b"current-user",
                ApplicationInstallScope::Machine => b"machine",
            });
            hasher.update(
                current_platform()
                    .path_identity_key(install_root)
                    .as_bytes(),
            );
            hasher.update(package_marker_digest.as_bytes());
            hasher.update(scoop_script_digest.as_bytes());
            hasher.update(&estimated_bytes.to_le_bytes());
        }
        ApplicationUninstallRegistration::WindowsChocolatey {
            package_name,
            install_root,
            package_marker_digest,
            chocolatey_executable: _,
            estimated_bytes,
        } => {
            hasher.update(b"chocolatey");
            hasher.update(package_name.as_bytes());
            hasher.update(
                current_platform()
                    .path_identity_key(install_root)
                    .as_bytes(),
            );
            hasher.update(package_marker_digest.as_bytes());
            hasher.update(&estimated_bytes.to_le_bytes());
        }
        ApplicationUninstallRegistration::WindowsRegistered {
            key_name,
            scope,
            registry_view,
            command_kind,
            command_digest,
            estimated_bytes,
        } => {
            hasher.update(b"registered");
            hasher.update(key_name.as_bytes());
            hasher.update(match scope {
                ApplicationInstallScope::CurrentUser => b"current-user",
                ApplicationInstallScope::Machine => b"machine",
            });
            hasher.update(match registry_view {
                mangodisk_platform::WindowsRegistryView::Registry32 => b"registry-32",
                mangodisk_platform::WindowsRegistryView::Registry64 => b"registry-64",
            });
            hasher.update(match command_kind {
                mangodisk_platform::WindowsRegisteredUninstallKind::Executable => b"executable",
                mangodisk_platform::WindowsRegisteredUninstallKind::UserPowerShellScript => {
                    b"user-powershell-script"
                }
                mangodisk_platform::WindowsRegisteredUninstallKind::WingetProduct => {
                    b"winget-product"
                }
            });
            hasher.update(command_digest.as_bytes());
            hasher.update(&estimated_bytes.to_le_bytes());
        }
    }
    hasher.finalize().to_hex().to_string()
}

fn registration_summary(registration: &ApplicationUninstallRegistration) -> (&str, u64) {
    match registration {
        ApplicationUninstallRegistration::WindowsMsi {
            product_code,
            estimated_bytes,
            ..
        } => (product_code, *estimated_bytes),
        ApplicationUninstallRegistration::WindowsAppx {
            package_full_name,
            estimated_bytes,
            ..
        } => (package_full_name, *estimated_bytes),
        ApplicationUninstallRegistration::WindowsScoop {
            package_name,
            estimated_bytes,
            ..
        } => (package_name, *estimated_bytes),
        ApplicationUninstallRegistration::WindowsChocolatey {
            package_name,
            estimated_bytes,
            ..
        } => (package_name, *estimated_bytes),
        ApplicationUninstallRegistration::WindowsRegistered {
            key_name,
            estimated_bytes,
            ..
        } => (key_name, *estimated_bytes),
    }
}

const fn installer_kind(
    registration: &ApplicationUninstallRegistration,
) -> ApplicationUninstallInstallerKind {
    match registration {
        ApplicationUninstallRegistration::WindowsMsi { .. } => {
            ApplicationUninstallInstallerKind::WindowsMsi
        }
        ApplicationUninstallRegistration::WindowsAppx { .. } => {
            ApplicationUninstallInstallerKind::WindowsAppx
        }
        ApplicationUninstallRegistration::WindowsScoop { .. } => {
            ApplicationUninstallInstallerKind::WindowsScoop
        }
        ApplicationUninstallRegistration::WindowsChocolatey { .. } => {
            ApplicationUninstallInstallerKind::WindowsChocolatey
        }
        ApplicationUninstallRegistration::WindowsRegistered { .. } => {
            ApplicationUninstallInstallerKind::WindowsRegistered
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_wait_detaches_promptly_after_cancellation() {
        let (_sender, receiver) = mpsc::sync_channel(1);
        let cancellation = Arc::new(AtomicBool::new(false));
        let cancellation_probe = Arc::clone(&cancellation);
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            cancellation_probe.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();

        let outcome = await_execution_result(receiver, cancellation)
            .expect("cancellation should detach the native wait");

        canceller.join().expect("cancellation worker should finish");
        assert!(matches!(outcome, ApplicationUninstallExecution::Detached));
        assert!(started.elapsed() < Duration::from_millis(250));
    }

    #[test]
    fn completed_native_result_wins_before_cancellation() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender
            .send(Ok(ApplicationUninstallExecution::Completed(
                ApplicationUninstallExecutionOutcome::Completed,
            )))
            .expect("fixture result should be queued");
        let cancellation = Arc::new(AtomicBool::new(true));

        let outcome = await_execution_result(receiver, cancellation)
            .expect("completed native result should be retained");

        assert!(matches!(
            outcome,
            ApplicationUninstallExecution::Completed(
                ApplicationUninstallExecutionOutcome::Completed
            )
        ));
    }

    #[test]
    fn elevation_prompt_cancellation_remains_a_cancelled_execution() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender
            .send(Ok(ApplicationUninstallExecution::Cancelled))
            .expect("fixture result should be queued");

        let outcome = await_execution_result(receiver, Arc::new(AtomicBool::new(false)))
            .expect("UAC cancellation should remain a normal cancellation");

        assert!(matches!(outcome, ApplicationUninstallExecution::Cancelled));
    }

    #[test]
    fn msi_component_identity_and_snapshot_are_deterministic() {
        let first_id = component_id("application-1", "{A}");
        let second_id = component_id("application-1", "{A}");
        assert_eq!(first_id, second_id);
        assert_ne!(first_id, component_id("application-2", "{A}"));

        let current_user = ApplicationUninstallRegistration::WindowsMsi {
            product_code: "{A}".to_string(),
            scope: ApplicationInstallScope::CurrentUser,
            estimated_bytes: 1024,
        };
        let machine = ApplicationUninstallRegistration::WindowsMsi {
            product_code: "{A}".to_string(),
            scope: ApplicationInstallScope::Machine,
            estimated_bytes: 1024,
        };
        let first = registration_fingerprint("application-1", &current_user);
        assert_eq!(
            first,
            registration_fingerprint("application-1", &current_user)
        );
        assert_ne!(first, registration_fingerprint("application-1", &machine));
    }

    #[test]
    fn appx_component_identity_uses_the_full_package_name() {
        let registration = ApplicationUninstallRegistration::WindowsAppx {
            package_family_name: "Example_123".to_string(),
            package_full_name: "Example_1.0.0.0_x64__123".to_string(),
            estimated_bytes: 2048,
        };

        assert_eq!(
            registration_summary(&registration),
            ("Example_1.0.0.0_x64__123", 2048)
        );
        assert_eq!(
            installer_kind(&registration),
            ApplicationUninstallInstallerKind::WindowsAppx
        );
    }

    #[test]
    fn registered_component_identity_includes_verified_uninstaller_evidence() {
        let registration = ApplicationUninstallRegistration::WindowsRegistered {
            key_name: "Example".to_string(),
            scope: ApplicationInstallScope::CurrentUser,
            registry_view: mangodisk_platform::WindowsRegistryView::Registry64,
            command_kind: mangodisk_platform::WindowsRegisteredUninstallKind::Executable,
            command_digest: "a".repeat(64),
            estimated_bytes: 4096,
        };

        assert_eq!(registration_summary(&registration), ("Example", 4096));
        assert_eq!(
            installer_kind(&registration),
            ApplicationUninstallInstallerKind::WindowsRegistered
        );
        let changed_registration = ApplicationUninstallRegistration::WindowsRegistered {
            key_name: "Example".to_string(),
            scope: ApplicationInstallScope::CurrentUser,
            registry_view: mangodisk_platform::WindowsRegistryView::Registry64,
            command_kind: mangodisk_platform::WindowsRegisteredUninstallKind::Executable,
            command_digest: "b".repeat(64),
            estimated_bytes: 4096,
        };
        assert_ne!(
            registration_fingerprint("application-1", &registration),
            registration_fingerprint("application-1", &changed_registration)
        );
    }

    #[cfg(windows)]
    #[test]
    fn package_fingerprint_ignores_windows_verbatim_path_prefix() {
        let display = ApplicationUninstallRegistration::WindowsScoop {
            package_name: "example".to_string(),
            scope: ApplicationInstallScope::CurrentUser,
            install_root: r"C:\Users\fixture\scoop\apps\example\current".into(),
            package_marker_digest: "marker".to_string(),
            scoop_script_digest: "script".to_string(),
            estimated_bytes: 1024,
        };
        let canonical = ApplicationUninstallRegistration::WindowsScoop {
            package_name: "example".to_string(),
            scope: ApplicationInstallScope::CurrentUser,
            install_root: r"\\?\C:\Users\fixture\scoop\apps\example\current".into(),
            package_marker_digest: "marker".to_string(),
            scoop_script_digest: "script".to_string(),
            estimated_bytes: 1024,
        };

        assert_eq!(
            registration_fingerprint("application-1", &display),
            registration_fingerprint("application-1", &canonical)
        );
    }
}
