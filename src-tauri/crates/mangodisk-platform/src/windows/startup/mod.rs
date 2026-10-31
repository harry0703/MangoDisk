mod advanced;
mod metadata;
mod packaged;
mod registry;
mod service_control;
mod service_target;
mod services;
mod startup_folder;
mod tasks;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use crate::{
    PlatformCancellation, PlatformError, PlatformErrorCode, PlatformResult,
    PlatformStartupChangeRequest, PlatformStartupChangeResult, PlatformStartupControlCapability,
    PlatformStartupCoverageReason, PlatformStartupDesiredState, PlatformStartupSourceResult,
};

// Native sources revalidate and verify every item independently. A small worker limit shortens
// slow task and registry batches without flooding COM, registry, or endpoint security hooks.
const MAX_DIRECT_CHANGE_CONCURRENCY: usize = 3;

pub(super) fn scan(
    cancellation: &PlatformCancellation,
) -> PlatformResult<Vec<PlatformStartupSourceResult>> {
    if cancellation.is_cancelled() {
        return Ok(vec![PlatformStartupSourceResult::unavailable(
            "windows.startup",
            true,
            PlatformStartupCoverageReason::Cancelled,
        )]);
    }

    let registry_cancellation = cancellation.clone();
    let packaged_cancellation = cancellation.clone();
    let folder_cancellation = cancellation.clone();
    let service_cancellation = cancellation.clone();
    let task_cancellation = cancellation.clone();
    let advanced_cancellation = cancellation.clone();
    let results = std::thread::scope(|scope| {
        let registry = scope.spawn(|| registry::scan(&registry_cancellation));
        let packaged = scope.spawn(|| packaged::scan(&packaged_cancellation));
        let folders = scope.spawn(|| startup_folder::scan(&folder_cancellation));
        let services = scope.spawn(|| services::scan(&service_cancellation));
        let tasks = scope.spawn(|| tasks::scan(&task_cancellation));
        let advanced = scope.spawn(|| advanced::scan(&advanced_cancellation));
        let mut results = vec![
            joined_source(registry, "windows.registry.run", true),
            joined_source(packaged, "windows.packaged_startup_tasks", false),
        ];
        results.extend(folders.join().unwrap_or_else(|_| {
            vec![PlatformStartupSourceResult::unavailable(
                "windows.startup_folder",
                true,
                PlatformStartupCoverageReason::InvalidData,
            )]
        }));
        results.push(joined_source(services, "windows.services", true));
        results.push(joined_source(tasks, "windows.scheduled_tasks", true));
        results.push(joined_source(advanced, "windows.advanced_autoruns", false));
        results
    });
    Ok(results)
}

fn joined_source(
    handle: std::thread::ScopedJoinHandle<'_, PlatformStartupSourceResult>,
    source_id: &'static str,
    required: bool,
) -> PlatformStartupSourceResult {
    handle.join().unwrap_or_else(|_| {
        PlatformStartupSourceResult::unavailable(
            source_id,
            required,
            PlatformStartupCoverageReason::InvalidData,
        )
    })
}

pub(super) fn change(
    request: &PlatformStartupChangeRequest,
) -> PlatformResult<PlatformStartupChangeResult> {
    // Elevated helpers exit before desktop logging is initialized, so retain locations here.
    registry::log_change_request(request);
    if requires_privileges(request) {
        return crate::startup_helper::change_with_privileges(request, None);
    }
    change_direct(request)
}

pub(super) fn change_many(
    requests: &[PlatformStartupChangeRequest],
) -> PlatformResult<Vec<PlatformResult<PlatformStartupChangeResult>>> {
    let started = Instant::now();
    for request in requests {
        registry::log_change_request(request);
    }
    let privileged = requests
        .iter()
        .enumerate()
        .filter(|(_, request)| requires_privileges(request))
        .collect::<Vec<_>>();
    let direct = requests
        .iter()
        .enumerate()
        .filter(|(_, request)| !requires_privileges(request))
        .collect::<Vec<_>>();
    let mut ordered_results = (0..requests.len()).map(|_| None).collect::<Vec<_>>();
    let mut privileged_result_count = 0;

    if !privileged.is_empty() {
        let privileged_requests = privileged
            .iter()
            .map(|(_, request)| *request)
            .collect::<Vec<_>>();
        let privileged_results =
            crate::startup_helper::change_many_with_privileges(&privileged_requests, None)?;
        privileged_result_count = privileged_results.len();
        if privileged_result_count == privileged.len() {
            for ((index, _), result) in privileged.iter().zip(privileged_results) {
                ordered_results[*index] = Some(result);
            }
        } else {
            for (index, _) in &privileged {
                ordered_results[*index] = Some(Err(PlatformError::new(
                    PlatformErrorCode::InvalidData,
                    "startup helper returned an invalid batch result count",
                )
                .with_possible_side_effects()));
            }
        }
    }

    let direct_requests = direct
        .iter()
        .map(|(_, request)| *request)
        .collect::<Vec<_>>();
    let direct_results =
        execute_bounded(&direct_requests, MAX_DIRECT_CHANGE_CONCURRENCY, |request| {
            change_direct(request)
        });
    let worker_panic_count = direct_results
        .iter()
        .filter(|result| result.is_none())
        .count();
    for ((index, _), result) in direct.iter().zip(direct_results) {
        ordered_results[*index] = Some(result.unwrap_or_else(|| {
            Err(PlatformError::new(
                PlatformErrorCode::OperationFailed,
                "startup change worker terminated unexpectedly",
            )
            .with_possible_side_effects())
        }));
    }

    let missing_result_count = ordered_results
        .iter()
        .filter(|result| result.is_none())
        .count();
    log::info!(
        "windows_startup_change_batch_finished target_count={} direct_count={} privileged_count={} privileged_result_count={} concurrency_limit={} worker_panic_count={} missing_result_count={} elapsed_ms={}",
        requests.len(),
        direct.len(),
        privileged.len(),
        privileged_result_count,
        MAX_DIRECT_CHANGE_CONCURRENCY,
        worker_panic_count,
        missing_result_count,
        started.elapsed().as_millis()
    );

    Ok(ordered_results
        .into_iter()
        .map(|result| {
            result.unwrap_or_else(|| {
                Err(PlatformError::new(
                    PlatformErrorCode::InvalidData,
                    "startup change produced no result",
                )
                .with_possible_side_effects())
            })
        })
        .collect())
}

fn execute_bounded<T, R, F>(items: &[T], limit: usize, action: F) -> Vec<Option<R>>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    if items.is_empty() {
        return Vec::new();
    }
    let worker_count = limit.max(1).min(items.len());
    let cursor = AtomicUsize::new(0);
    let results = (0..items.len())
        .map(|_| Mutex::new(None))
        .collect::<Vec<_>>();
    std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            workers.push(scope.spawn(|| loop {
                let index = cursor.fetch_add(1, Ordering::Relaxed);
                if index >= items.len() {
                    break;
                }
                let output = action(&items[index]);
                if let Ok(mut result) = results[index].lock() {
                    *result = Some(output);
                }
            }));
        }
        for worker in workers {
            let _ = worker.join();
        }
    });
    results
        .into_iter()
        .map(|result| result.into_inner().ok().flatten())
        .collect()
}

fn requires_privileges(request: &PlatformStartupChangeRequest) -> bool {
    request.expected_artifact.control_capability
        == PlatformStartupControlCapability::ElevationRequired
        || (request.desired_state == PlatformStartupDesiredState::Removed
            && matches!(
                request.expected_artifact.scope,
                crate::PlatformStartupScope::AllUsers
                    | crate::PlatformStartupScope::Machine
                    | crate::PlatformStartupScope::System
            ))
}

fn change_direct(
    request: &PlatformStartupChangeRequest,
) -> PlatformResult<PlatformStartupChangeResult> {
    match request.source_id.as_str() {
        "windows.registry.run" => registry::change(request),
        "windows.startup_folder.user" | "windows.startup_folder.common" => {
            startup_folder::change(request)
        }
        "windows.scheduled_tasks" => tasks::change(request),
        "windows.services" => service_control::change(request),
        "windows.advanced_autoruns" => advanced::change(request),
        _ => Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "startup source does not support configured-state changes",
        )),
    }
}

#[cfg(test)]
pub(super) fn helper_change(
    source_id: &str,
    provider_item_id: &str,
    expected_artifact_digest: &str,
    desired_state: PlatformStartupDesiredState,
) -> PlatformResult<PlatformStartupChangeResult> {
    let cancellation = PlatformCancellation::new(|| false);
    let results = scan(&cancellation)?;
    helper_change_from_snapshot(
        source_id,
        provider_item_id,
        expected_artifact_digest,
        desired_state,
        &results,
    )
}

pub(super) fn helper_change_many(
    requests: &[crate::startup_helper::StartupHelperChangeRequest],
) -> Vec<PlatformResult<PlatformStartupChangeResult>> {
    let cancellation = PlatformCancellation::new(|| false);
    let mut results = Vec::new();
    if requests
        .iter()
        .any(|request| request.source_id == "windows.services")
    {
        results.push(services::scan(&cancellation));
    }
    if requests
        .iter()
        .any(|request| request.source_id == "windows.registry.run")
    {
        results.push(registry::scan(&cancellation));
    }
    if requests
        .iter()
        .any(|request| request.source_id.starts_with("windows.startup_folder."))
    {
        results.extend(startup_folder::scan(&cancellation));
    }
    if requests
        .iter()
        .any(|request| request.source_id == "windows.scheduled_tasks")
    {
        results.push(tasks::scan(&cancellation));
    }
    if requests
        .iter()
        .any(|request| request.source_id == "windows.advanced_autoruns")
    {
        results.push(advanced::scan(&cancellation));
    }
    requests
        .iter()
        .map(|request| {
            helper_change_from_snapshot(
                &request.source_id,
                &request.provider_item_id,
                &request.expected_artifact_digest,
                request.desired_state,
                &results,
            )
        })
        .collect()
}

fn helper_change_from_snapshot(
    source_id: &str,
    provider_item_id: &str,
    expected_artifact_digest: &str,
    desired_state: PlatformStartupDesiredState,
    results: &[PlatformStartupSourceResult],
) -> PlatformResult<PlatformStartupChangeResult> {
    if !helper_source_is_allowlisted(source_id) {
        return Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "startup helper source is not allowlisted",
        ));
    }
    let artifact = results
        .iter()
        .find(|source| source.source_id == source_id)
        .and_then(|source| {
            source
                .items
                .iter()
                .find(|artifact| artifact.provider_item_id == provider_item_id)
        })
        .cloned()
        .ok_or_else(|| PlatformError::item_changed("startup helper target no longer exists"))?;
    let authorized = artifact.control_capability
        == PlatformStartupControlCapability::ElevationRequired
        || (desired_state == PlatformStartupDesiredState::Removed
            && matches!(
                artifact.control_capability,
                PlatformStartupControlCapability::Toggleable
                    | PlatformStartupControlCapability::RemoveOnly
            )
            && matches!(
                artifact.scope,
                crate::PlatformStartupScope::AllUsers
                    | crate::PlatformStartupScope::Machine
                    | crate::PlatformStartupScope::System
            ));
    if !authorized || crate::startup_helper::artifact_digest(&artifact) != expected_artifact_digest
    {
        return Err(PlatformError::item_changed(
            "startup helper target changed after preflight",
        ));
    }
    change_direct(&PlatformStartupChangeRequest {
        provider_item_id: provider_item_id.to_owned(),
        source_id: source_id.to_owned(),
        expected_artifact: artifact,
        desired_state,
    })
}

fn helper_source_is_allowlisted(source_id: &str) -> bool {
    matches!(
        source_id,
        "windows.registry.run"
            | "windows.startup_folder.user"
            | "windows.startup_folder.common"
            | "windows.scheduled_tasks"
            | "windows.services"
            | "windows.advanced_autoruns"
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::PlatformStartupConfiguredState;
    use winreg::{
        enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY},
        RegKey,
    };

    use super::*;

    const RUN_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVAL_PATH: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    #[test]
    fn helper_allowlist_includes_service_configuration_changes() {
        assert!(helper_source_is_allowlisted("windows.scheduled_tasks"));
        assert!(helper_source_is_allowlisted("windows.advanced_autoruns"));
        assert!(helper_source_is_allowlisted("windows.services"));
    }

    #[test]
    fn bounded_executor_preserves_order_and_limits_parallel_work() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let items = (0..9).collect::<Vec<_>>();

        let results = execute_bounded(&items, 3, |item| {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(10));
            active.fetch_sub(1, Ordering::SeqCst);
            item * 2
        });

        assert_eq!(
            results,
            items.iter().map(|item| Some(item * 2)).collect::<Vec<_>>()
        );
        assert_eq!(peak.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn bounded_executor_treats_a_zero_limit_as_one_worker() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let results = execute_bounded(&[1, 2, 3], 0, |item| {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(5));
            active.fetch_sub(1, Ordering::SeqCst);
            item * 2
        });

        assert_eq!(results, vec![Some(2), Some(4), Some(6)]);
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    #[ignore = "requires administrator rights and a startup helper; captures process-local logs"]
    fn actual_elevated_registry_changes_retain_location_logs_in_parent() {
        assert!(std::env::var_os("MANGODISK_TEST_ELEVATION_HELPER_EXE").is_some());
        struct Evidence(Mutex<Vec<String>>);
        impl log::Log for Evidence {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }

            fn log(&self, record: &log::Record<'_>) {
                self.0.lock().unwrap().push(record.args().to_string());
            }

            fn flush(&self) {}
        }
        static EVIDENCE: Evidence = Evidence(Mutex::new(Vec::new()));
        log::set_logger(&EVIDENCE).expect("the isolated diagnostic owns its logger");
        log::set_max_level(log::LevelFilter::Info);

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let value_name = format!("MangoDiskRegistryLogFixture{suffix}");
        let root = RegKey::predef(HKEY_LOCAL_MACHINE);
        let (run, _) = root
            .create_subkey_with_flags(RUN_PATH, KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY)
            .expect("the isolated machine Run fixture must be writable");
        let _cleanup = MachineRegistryFixture {
            value_name: value_name.clone(),
        };
        run.set_value(
            &value_name,
            &r#""C:\Windows\System32\notepad.exe" --mangodisk-log-fixture"#,
        )
        .expect("the isolated Run fixture must be created");

        for desired_state in [
            PlatformStartupDesiredState::Disabled,
            PlatformStartupDesiredState::Enabled,
            PlatformStartupDesiredState::Removed,
        ] {
            let artifact = machine_fixture(&value_name);
            let request = PlatformStartupChangeRequest {
                provider_item_id: artifact.provider_item_id.clone(),
                source_id: "windows.registry.run".to_owned(),
                expected_artifact: artifact,
                desired_state,
            };
            let result = if desired_state == PlatformStartupDesiredState::Enabled {
                vec![change(&request)]
            } else {
                change_many(&[request]).expect("the actual helper batch must finish")
            };
            assert!(
                result[0]
                    .as_ref()
                    .expect("the isolated change must succeed")
                    .verified
            );
        }

        let evidence = EVIDENCE.0.lock().unwrap();
        for line in evidence.iter() {
            println!("{line}");
        }
        let registry_lines = evidence
            .iter()
            .filter(|line| line.starts_with("windows_startup_registry_change_requested "))
            .collect::<Vec<_>>();
        assert_eq!(
            registry_lines.len(),
            3,
            "each elevated change must retain registry location in the parent log"
        );
        for line in registry_lines {
            assert!(line.contains("registry_root=hklm"));
            assert!(line.contains("registry_view=64"));
            assert!(line.contains(&format!("registry_value_name=\"{value_name}\"")));
            assert!(line.contains(r"Software\\Microsoft\\Windows\\CurrentVersion\\Run"));
            assert!(line.contains(r"Explorer\\StartupApproved\\Run"));
            assert!(!line.contains("--mangodisk-log-fixture"));
        }
    }

    #[test]
    #[ignore = "requires an elevated Windows fixture"]
    fn actual_machine_registry_helper_revalidates_toggles_and_removes() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be available")
            .as_nanos();
        let value_name = format!("MangoDiskMachineStartupFixture{suffix}");
        let root = RegKey::predef(HKEY_LOCAL_MACHINE);
        let (run, _) = root
            .create_subkey_with_flags(RUN_PATH, KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY)
            .expect("the machine Run fixture key must be writable");
        let (approval, _) = root
            .create_subkey_with_flags(APPROVAL_PATH, KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY)
            .expect("the machine approval fixture key must be writable");
        let _cleanup = MachineRegistryFixture {
            value_name: value_name.clone(),
        };
        run.set_value(
            &value_name,
            &r#""C:\Windows\System32\notepad.exe" --mangodisk-machine-fixture"#,
        )
        .expect("the machine Run fixture must be created");
        let _ = approval.delete_value(&value_name);

        let enabled = machine_fixture(&value_name);
        assert_eq!(
            enabled.control_capability,
            PlatformStartupControlCapability::ElevationRequired
        );
        let disabled = helper_change(
            "windows.registry.run",
            &enabled.provider_item_id,
            &crate::startup_helper::artifact_digest(&enabled),
            PlatformStartupDesiredState::Disabled,
        )
        .expect("the elevated helper boundary must disable the fixture");
        assert!(disabled.verified);

        let disabled_artifact = machine_fixture(&value_name);
        let restored = helper_change(
            "windows.registry.run",
            &disabled_artifact.provider_item_id,
            &crate::startup_helper::artifact_digest(&disabled_artifact),
            PlatformStartupDesiredState::Enabled,
        )
        .expect("the elevated helper boundary must restore the fixture");
        assert!(restored.verified);
        assert!(approval.get_raw_value(&value_name).is_err());

        let enabled_artifact = machine_fixture(&value_name);
        let removed = helper_change(
            "windows.registry.run",
            &enabled_artifact.provider_item_id,
            &crate::startup_helper::artifact_digest(&enabled_artifact),
            PlatformStartupDesiredState::Removed,
        )
        .expect("the elevated helper boundary must remove the fixture");
        assert!(removed.verified);
        assert_eq!(
            removed.configured_state,
            PlatformStartupConfiguredState::NotApplicable
        );
        assert!(run.get_raw_value(&value_name).is_err());
        assert!(approval.get_raw_value(&value_name).is_err());
    }

    fn machine_fixture(value_name: &str) -> crate::PlatformStartupArtifact {
        let cancellation = PlatformCancellation::new(|| false);
        scan(&cancellation)
            .expect("the startup scan must complete")
            .into_iter()
            .find(|source| source.source_id == "windows.registry.run")
            .and_then(|source| {
                source
                    .items
                    .into_iter()
                    .find(|artifact| artifact.display_name == value_name)
            })
            .expect("the machine fixture must be discoverable")
    }

    struct MachineRegistryFixture {
        value_name: String,
    }

    impl Drop for MachineRegistryFixture {
        fn drop(&mut self) {
            let root = RegKey::predef(HKEY_LOCAL_MACHINE);
            if let Ok(run) = root.open_subkey_with_flags(RUN_PATH, KEY_SET_VALUE | KEY_WOW64_64KEY)
            {
                let _ = run.delete_value(&self.value_name);
            }
            if let Ok(approval) =
                root.open_subkey_with_flags(APPROVAL_PATH, KEY_SET_VALUE | KEY_WOW64_64KEY)
            {
                let _ = approval.delete_value(&self.value_name);
            }
        }
    }
}
