//! Elevation adapters for the optional Windows sensor capability.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceState {
    NotInstalled,
    Ready,
    Unavailable,
    Incompatible,
    UnsupportedHardware,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub state: ServiceState,
    pub driver_available: bool,
    pub service_installed: bool,
    #[cfg(windows)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_issue: Option<DependencyIssue>,
}
#[cfg(windows)]
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyIssueReason {
    Missing,
    Incompatible,
    CertificateUntrusted,
    VerificationFailed,
    Unavailable,
}
#[cfg(windows)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyIssue {
    pub reason: DependencyIssueReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_code: Option<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SetupOutcome {
    Ready,
    Removed,
    UnsupportedHardware,
    Busy,
    VerificationFailed,
    CertificateUntrusted,
    UserCancelled,
    InstallationFailed,
    ServiceUnavailable,
    RestartRequired,
    DriverRemovalFailed,
    RemovalFailed,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupResult {
    pub outcome: SetupOutcome,
    pub observation: Observation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_code: Option<String>,
}

#[cfg(windows)]
fn signature_outcome(code: u32) -> SetupOutcome {
    if code == mangodisk_privileged::signature_verification::CERT_E_UNTRUSTEDROOT {
        SetupOutcome::CertificateUntrusted
    } else {
        SetupOutcome::VerificationFailed
    }
}

pub fn observe() -> Observation {
    #[cfg(windows)]
    return native::observe();
    #[cfg(not(windows))]
    Observation {
        state: ServiceState::UnsupportedHardware,
        driver_available: false,
        service_installed: false,
    }
}
pub async fn install(app: tauri::AppHandle) -> SetupResult {
    #[cfg(windows)]
    return native::install(app).await;
    #[cfg(not(windows))]
    {
        let _ = app;
        SetupResult {
            outcome: SetupOutcome::UnsupportedHardware,
            observation: observe(),
            native_code: None,
        }
    }
}
pub async fn uninstall(app: tauri::AppHandle, remove_driver: bool) -> SetupResult {
    #[cfg(windows)]
    return native::uninstall(app, remove_driver).await;
    #[cfg(not(windows))]
    {
        let _ = (app, remove_driver);
        SetupResult {
            outcome: SetupOutcome::UnsupportedHardware,
            observation: observe(),
            native_code: None,
        }
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use mangodisk_privileged::windows::{client, deployment};
    use mangodisk_privileged_protocol::{CapabilityStatus, Operation, Reply};
    use std::sync::atomic::{AtomicBool, Ordering};

    static BOOTSTRAP: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/resources/privileged/MangoDiskPrivileged.exe"
    ));

    static BUSY: AtomicBool = AtomicBool::new(false);
    struct OperationGuard;
    impl OperationGuard {
        fn acquire() -> Option<Self> {
            BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                .ok()
                .map(|_| Self)
        }
    }
    impl Drop for OperationGuard {
        fn drop(&mut self) {
            BUSY.store(false, Ordering::Release);
        }
    }
    pub(super) fn observe() -> Observation {
        let installation = client::service_installed();
        let service_installed = matches!(installation, Ok(true));
        if !deployment::hardware_supported() {
            return Observation {
                state: ServiceState::UnsupportedHardware,
                driver_available: false,
                service_installed,
                dependency_issue: None,
            };
        }
        let (driver_available, dependency_issue) = match deployment::driver_dependency() {
            Ok(deployment::DriverDependency::Ready) => (true, None),
            Ok(dependency) => (
                false,
                Some(DependencyIssue {
                    reason: match dependency {
                        deployment::DriverDependency::Missing => DependencyIssueReason::Missing,
                        deployment::DriverDependency::Incompatible => {
                            DependencyIssueReason::Incompatible
                        }
                        deployment::DriverDependency::Ready => unreachable!(),
                    },
                    native_code: None,
                }),
            ),
            Err(error) => {
                let code = mangodisk_privileged::signature_verification::native_code(&error);
                (
                    false,
                    Some(DependencyIssue {
                        reason: match code {
                            Some(
                                mangodisk_privileged::signature_verification::CERT_E_UNTRUSTEDROOT,
                            ) => DependencyIssueReason::CertificateUntrusted,
                            Some(_) => DependencyIssueReason::VerificationFailed,
                            None => DependencyIssueReason::Unavailable,
                        },
                        native_code: code.map(|code| format!("0x{code:08X}")),
                    }),
                )
            }
        };
        let state = match installation {
            Ok(false) => ServiceState::NotInstalled,
            Err(_) => ServiceState::Unavailable,
            Ok(true) => match client::request(Operation::Health) {
                Ok(response) => match response.reply {
                    Reply::Health {
                        cpu_temperature, ..
                    } => {
                        if cpu_temperature == CapabilityStatus::Ready && sensor_ready() {
                            ServiceState::Ready
                        } else {
                            ServiceState::Unavailable
                        }
                    }
                    _ => ServiceState::Incompatible,
                },
                Err(error) => {
                    log::warn!(
                        "windows_monitoring_status_failed error={}",
                        mangodisk_platform::diagnostics::text(&error)
                    );
                    if error.kind() == std::io::ErrorKind::InvalidData {
                        ServiceState::Incompatible
                    } else {
                        ServiceState::Unavailable
                    }
                }
            },
        };
        Observation {
            state,
            driver_available,
            service_installed,
            dependency_issue,
        }
    }
    fn sensor_ready() -> bool {
        let Ok(response) = client::request(Operation::CpuTemperature) else {
            return false;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        matches!(response.reply, Reply::CpuTemperature { status: CapabilityStatus::Ready, reading: Some(reading) } if reading.validate(now).is_ok())
    }
    async fn observe_async() -> Observation {
        tauri::async_runtime::spawn_blocking(observe)
            .await
            .unwrap_or(Observation {
                state: ServiceState::Unavailable,
                driver_available: false,
                service_installed: false,
                dependency_issue: None,
            })
    }
    pub(super) async fn install(_app: tauri::AppHandle) -> SetupResult {
        let Some(guard) = OperationGuard::acquire() else {
            return SetupResult {
                outcome: SetupOutcome::Busy,
                observation: observe_async().await,
                native_code: None,
            };
        };
        let initial = observe_async().await;
        if matches!(initial.state, ServiceState::UnsupportedHardware) {
            return SetupResult {
                outcome: SetupOutcome::UnsupportedHardware,
                observation: initial,
                native_code: None,
            };
        }
        if let Some(issue) = initial.dependency_issue.as_ref().filter(|issue| {
            matches!(
                issue.reason,
                DependencyIssueReason::CertificateUntrusted
                    | DependencyIssueReason::VerificationFailed
            )
        }) {
            log::warn!(
                "windows_monitoring_dependency_blocked component=PawnIOLib.dll stage=preflight reason={:?} native_code={:?}",
                issue.reason, issue.native_code
            );
            return SetupResult {
                outcome: if matches!(issue.reason, DependencyIssueReason::CertificateUntrusted) {
                    SetupOutcome::CertificateUntrusted
                } else {
                    SetupOutcome::VerificationFailed
                },
                native_code: issue.native_code.clone(),
                observation: initial,
            };
        }
        let result = tauri::async_runtime::spawn_blocking(move || {
            // The worker owns the gate even if its async caller is dropped.
            let _guard = guard;
            client::elevate_embedded(BOOTSTRAP, client::SetupAction::InstallCpuTemperature)
        })
        .await;
        let mut native_code = None;
        let outcome = match result {
            Ok(Ok(code)) if mangodisk_privileged::signature_verification::is_trust_status(code) => {
                native_code = Some(format!("0x{code:08X}"));
                log::warn!("windows_monitoring_signature_failed native_code=0x{code:08X}");
                signature_outcome(code)
            }
            Ok(Err(ref error))
                if mangodisk_privileged::signature_verification::native_code(error).is_some() =>
            {
                let code = mangodisk_privileged::signature_verification::native_code(error)
                    .expect("checked signature status");
                native_code = Some(format!("0x{code:08X}"));
                log::warn!("windows_monitoring_signature_failed native_code=0x{code:08X}");
                signature_outcome(code)
            }
            Ok(Ok(0)) => SetupOutcome::Ready,
            Ok(Ok(deployment::REBOOT_EXIT_CODE)) => SetupOutcome::RestartRequired,
            Ok(Ok(deployment::BUSY_EXIT_CODE)) => SetupOutcome::Busy,
            Ok(Ok(deployment::VERIFICATION_EXIT_CODE)) => SetupOutcome::VerificationFailed,
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::InvalidData => {
                log::warn!(
                    "windows_monitoring_bootstrap_verification_failed error={}",
                    mangodisk_platform::diagnostics::text(&error)
                );
                SetupOutcome::VerificationFailed
            }
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::WouldBlock => SetupOutcome::Busy,
            Ok(Err(error)) if error.raw_os_error() == Some(1223) => SetupOutcome::UserCancelled,
            Ok(Err(error)) => {
                log::warn!(
                    "windows_monitoring_install_failed native_code={:?} error={}",
                    error.raw_os_error(),
                    mangodisk_platform::diagnostics::text(&error)
                );
                SetupOutcome::InstallationFailed
            }
            other => {
                log::warn!("windows_monitoring_install_failed result={other:?}");
                SetupOutcome::InstallationFailed
            }
        };
        let observation = observe_async().await;
        let outcome = match outcome {
            SetupOutcome::Ready if !matches!(observation.state, ServiceState::Ready) => {
                SetupOutcome::ServiceUnavailable
            }
            outcome => outcome,
        };
        SetupResult {
            outcome,
            observation,
            native_code,
        }
    }
    pub(super) async fn uninstall(_app: tauri::AppHandle, remove_driver: bool) -> SetupResult {
        let Some(guard) = OperationGuard::acquire() else {
            return SetupResult {
                outcome: SetupOutcome::Busy,
                observation: observe_async().await,
                native_code: None,
            };
        };
        let result = tauri::async_runtime::spawn_blocking(move || {
            let _guard = guard;
            client::elevate_embedded(BOOTSTRAP, client::SetupAction::Uninstall { remove_driver })
        })
        .await;
        let observation = observe_async().await;
        let mut native_code = None;
        let outcome = match result {
            Ok(Ok(code)) if mangodisk_privileged::signature_verification::is_trust_status(code) => {
                native_code = Some(format!("0x{code:08X}"));
                log::warn!("windows_monitoring_signature_failed native_code=0x{code:08X}");
                signature_outcome(code)
            }
            Ok(Err(ref error))
                if mangodisk_privileged::signature_verification::native_code(error).is_some() =>
            {
                let code = mangodisk_privileged::signature_verification::native_code(error)
                    .expect("checked signature status");
                native_code = Some(format!("0x{code:08X}"));
                log::warn!("windows_monitoring_signature_failed native_code=0x{code:08X}");
                signature_outcome(code)
            }
            Ok(Ok(deployment::BUSY_EXIT_CODE)) => SetupOutcome::Busy,
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::WouldBlock => SetupOutcome::Busy,
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::InvalidData => {
                log::warn!(
                    "windows_monitoring_bootstrap_verification_failed error={}",
                    mangodisk_platform::diagnostics::text(&error)
                );
                SetupOutcome::VerificationFailed
            }
            Ok(Ok(0))
                if !observation.service_installed
                    && (!remove_driver || !observation.driver_available) =>
            {
                SetupOutcome::Removed
            }
            Ok(Ok(deployment::REBOOT_EXIT_CODE)) => SetupOutcome::RestartRequired,
            Ok(Ok(deployment::DRIVER_REMOVAL_EXIT_CODE)) => SetupOutcome::DriverRemovalFailed,
            Ok(Err(error)) if error.raw_os_error() == Some(1223) => SetupOutcome::UserCancelled,
            Ok(Err(error)) => {
                log::warn!(
                    "windows_monitoring_uninstall_failed remove_driver={remove_driver} native_code={:?} error={}",
                    error.raw_os_error(),
                    mangodisk_platform::diagnostics::text(&error)
                );
                SetupOutcome::RemovalFailed
            }
            other => {
                log::warn!("windows_monitoring_uninstall_failed remove_driver={remove_driver} result={other:?}");
                SetupOutcome::RemovalFailed
            }
        };
        SetupResult {
            outcome,
            observation,
            native_code,
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn distinguishes_untrusted_roots_from_other_signature_failures() {
        assert!(matches!(
            signature_outcome(0x800b0109),
            SetupOutcome::CertificateUntrusted
        ));
        for code in [0x800b0101, 0x80096010, 0x800b0100, 0x800b010a] {
            assert!(matches!(
                signature_outcome(code),
                SetupOutcome::VerificationFailed
            ));
        }
    }

    #[test]
    fn serializes_dependency_diagnostics_and_native_status_without_free_form_errors() {
        let result = SetupResult {
            outcome: SetupOutcome::CertificateUntrusted,
            native_code: Some("0x800B0109".into()),
            observation: Observation {
                state: ServiceState::NotInstalled,
                driver_available: false,
                service_installed: false,
                dependency_issue: Some(DependencyIssue {
                    reason: DependencyIssueReason::CertificateUntrusted,
                    native_code: Some("0x800B0109".into()),
                }),
            },
        };
        let json = serde_json::to_value(result).expect("setup result serializes");
        assert_eq!(json["outcome"], "certificateUntrusted");
        assert_eq!(json["nativeCode"], "0x800B0109");
        assert_eq!(
            json["observation"]["dependencyIssue"]["reason"],
            "certificateUntrusted"
        );
    }
}
