use super::*;
use crate::system_maintenance_helper::step_diagnostics::{
    MaintenanceComponent, MaintenanceStage, ServiceStartup, ServiceStatus,
};

const SERVICE_FIXTURE: &str = r#"
$states = @{bits='Stopped'; cryptsvc='Running'; wuauserv='Stopped'; WSearch='Running'; Spooler='Running'; Audiosrv='Running'; W32Time='Stopped'}
function Get-Service($Name, $ErrorAction) {
    if ($mode -eq 'query' -and $Name -eq 'bits') { throw [System.ComponentModel.Win32Exception]::new(1060) }
    $startup = 'Manual'
    if ($mode -eq 'disabled' -and $Name -eq 'bits') { $startup = 'Disabled' }
    [pscustomobject]@{Status=$script:states[$Name]; StartType=$startup}
}
function Start-Service($Name, $ErrorAction) {
    if ($mode -eq 'unauthorized') { throw [UnauthorizedAccessException]::new() }
    if ($mode -eq 'disabled' -and $Name -eq 'bits') { throw [System.InvalidOperationException]::new('private detail', [System.ComponentModel.Win32Exception]::new(1058)) }
    $script:states[$Name]='Running'
}
function Restart-Service($Name, [switch]$Force, $ErrorAction) {
    if ($mode -eq 'restart' -and $Name -eq 'cryptsvc') {
        $script:states[$Name]='Stopped'
        throw [System.ComponentModel.Win32Exception]::new(5)
    }
    $script:states[$Name]='Running'
}
function Stop-Service($Name, [switch]$Force, $ErrorAction) { $script:states[$Name]='Stopped' }
function Start-Process($FilePath, $ArgumentList, $WindowStyle, $ErrorAction) {
    if ($mode -eq 'scan') { throw [System.ComponentModel.Win32Exception]::new(5) }
    if ($mode -eq 'missingTool') { throw [System.ComponentModel.Win32Exception]::new(2) }
}
"#;

fn run_fixture(prefix: &str, body: &str) -> PrivilegedMaintenanceResult {
    let windows = windows_directory().unwrap();
    let mut channel = ElevatedProgressChannel::bind().unwrap();
    let script = format!(
        "{prefix}\n{}",
        maintenance_script(&windows, body, Some(&channel))
    );
    run_privileged_powershell(
        &windows,
        UPDATE_COMPONENTS,
        &script,
        PlatformSystemMaintenancePhase::RestartingServices,
        &|_| {},
        Some(&mut channel),
        None,
    )
}

#[test]
fn shared_runner_retains_partial_service_results_and_nested_native_errors() {
    for mode in [
        "success",
        "unauthorized",
        "disabled",
        "restart",
        "query",
        "scan",
        "missingTool",
    ] {
        let result = run_fixture(
            &format!("$mode='{mode}'\n{SERVICE_FIXTURE}"),
            task_scripts::recipe(UPDATE_COMPONENTS).unwrap(),
        );
        assert_eq!(result.is_ok(), mode == "success", "{mode}");
        let diagnostics = match result {
            Ok(outcome) => outcome.diagnostics,
            Err(failure) => failure.diagnostics.unwrap(),
        };
        assert!(diagnostics.progress_channel_authenticated, "{mode}");
        assert!(!diagnostics.progress_channel_failed, "{mode}");
        let steps = diagnostics.steps;
        let last = steps.last().unwrap();
        match mode {
            "success" => {
                assert_eq!(steps.len(), 4);
                assert_eq!(steps[0].after, Some(ServiceStatus::Running));
                assert!(steps
                    .iter()
                    .all(|step| step.result == MaintenanceStepResult::Succeeded));
            }
            "unauthorized" => {
                assert_eq!(last.native_error, Some(5));
                assert_eq!(
                    classify_failure(last).code(),
                    PlatformErrorCode::AccessDenied
                );
            }
            "disabled" => {
                assert_eq!(steps.len(), 1);
                assert_eq!(last.startup, Some(ServiceStartup::Disabled));
                assert_eq!(last.native_error, Some(1058));
            }
            "restart" => {
                assert_eq!(steps.len(), 2);
                assert_eq!(steps[0].result, MaintenanceStepResult::Succeeded);
                assert_eq!(last.before, Some(ServiceStatus::Running));
                assert_eq!(last.after, Some(ServiceStatus::Stopped));
                assert_eq!(last.native_error, Some(5));
            }
            "query" => {
                assert_eq!(last.native_error, Some(1060));
                assert!(last.state_query_failed);
            }
            "scan" | "missingTool" => {
                assert_eq!(last.component, MaintenanceComponent::UsoClient);
                assert_eq!(last.native_error, Some(if mode == "scan" { 5 } else { 2 }));
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn broken_diagnostic_channel_preserves_collected_steps_and_native_outcome() {
    for mode in ["success", "disabled"] {
        // Exercise the real PowerShell process and TCP reader without changing host services.
        // Keep the child alive after the malformed frame so the failure occurs in the polling
        // loop, not just in the final drain after process exit.
        let result = run_fixture(
            &format!("$mode='{mode}'\n{SERVICE_FIXTURE}"),
            r#"
try { Invoke-MangoService 'bits' }
finally {
    Send-MangoLine '{invalid-diagnostic-frame'
    Start-Sleep -Milliseconds 1000
}
"#,
        );
        let diagnostics = if mode == "disabled" {
            let failure = result.expect_err("the service failure must remain a failure");
            assert_eq!(
                failure.error.failure_reason(),
                Some(crate::PlatformFailureReason::ServiceDisabled)
            );
            assert_eq!(failure.native_error_code, Some(1058));
            assert_eq!(
                failure.error.mutation_state(),
                crate::PlatformMutationState::MayHaveChanged
            );
            failure.diagnostics.unwrap()
        } else {
            result
                .expect("telemetry failure must not turn successful maintenance into failure")
                .diagnostics
        };
        assert!(diagnostics.progress_channel_authenticated);
        assert!(diagnostics.progress_channel_failed);
        assert!(diagnostics.progress_event_count >= 3);
        assert_eq!(diagnostics.steps.len(), 1);
        let step = diagnostics.steps[0];
        assert_eq!(step.component, MaintenanceComponent::Bits);
        assert_eq!(step.before, Some(ServiceStatus::Stopped));
        assert_eq!(
            step.after,
            Some(if mode == "disabled" {
                ServiceStatus::Stopped
            } else {
                ServiceStatus::Running
            })
        );
        assert_eq!(
            step.result,
            if mode == "disabled" {
                MaintenanceStepResult::Failed
            } else {
                MaintenanceStepResult::Succeeded
            }
        );
        assert!(!serde_json::to_string(&diagnostics)
            .unwrap()
            .contains("invalid-diagnostic-frame"));
        println!(
            "fault_injection_diagnostics scenario={mode} {}",
            serde_json::to_string(&diagnostics).unwrap()
        );
    }
}

#[test]
fn service_executor_is_shared_by_search_audio_print_and_time_tasks() {
    let result = run_fixture(&format!("$mode='success'\n{SERVICE_FIXTURE}"), "foreach ($name in @('WSearch','Spooler','Audiosrv','W32Time')) { Invoke-MangoService $name }").unwrap();
    assert_eq!(result.diagnostics.steps.len(), 4);
    assert!(result
        .diagnostics
        .steps
        .iter()
        .all(|step| step.result == MaintenanceStepResult::Succeeded
            && step.after == Some(ServiceStatus::Running)));
}

#[test]
fn native_verification_failure_retains_exit_code_and_redacted_output() {
    let directory = std::env::temp_dir().join(format!(
        "md-maint-{}-{}",
        std::process::id(),
        PROGRESS_CHANNEL_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("fixture.cmd");
    std::fs::write(
        &executable,
        "@echo off\r\necho private fixture output\r\nexit /b 5\r\n",
    )
    .unwrap();
    let body = format!(
        "Invoke-MangoNative 'Lodctr' '{}' @() 'Verify'",
        executable.to_string_lossy().replace('\'', "''")
    );
    let failure = run_fixture("", &body).unwrap_err();
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(
        failure.error.failure_reason(),
        Some(crate::PlatformFailureReason::VerificationFailed)
    );
    let step = failure.diagnostics.unwrap().steps.last().copied().unwrap();
    assert_eq!(step.stage, MaintenanceStage::Verify);
    assert_eq!(step.exit_code, Some(5));
    assert_eq!(
        step.native_error, None,
        "arbitrary command exit codes are not Win32 errors"
    );
    assert!(step.output_bytes > 0);
    assert!(step.output_digest.is_some());
    assert!(!serde_json::to_string(&step)
        .unwrap()
        .contains("private fixture"));
}

#[test]
fn every_compiled_recipe_parses_without_executing_maintenance() {
    let recipes = SUPPORTED_TASKS
        .iter()
        .filter_map(|id| task_scripts::recipe(id))
        .collect::<Vec<_>>();
    assert_eq!(recipes.len(), 10);
    assert!(task_scripts::recipe("unknown").is_none());
    let encoded = recipes
        .iter()
        .map(|body| format!("'{}'", STANDARD.encode(body.as_bytes())))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        r#"foreach ($encoded in @({encoded})) {{
        $body = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($encoded))
        $tokens = $null; $errors = $null
        [void][System.Management.Automation.Language.Parser]::ParseInput($body, [ref]$tokens, [ref]$errors)
        if ($errors.Count -gt 0) {{ exit 1 }}
    }}; exit 0"#
    );
    run_privileged_powershell(
        &windows_directory().unwrap(),
        UPDATE_COMPONENTS,
        &script,
        PlatformSystemMaintenancePhase::Preparing,
        &|_| {},
        None,
        None,
    )
    .unwrap();
}

fn install_native_test_logger() {
    struct TestLog;
    impl log::Log for TestLog {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            println!("{}", record.args());
        }
        fn flush(&self) {}
    }
    static LOGGER: TestLog = TestLog;
    static INITIALIZED: std::sync::Once = std::sync::Once::new();
    INITIALIZED.call_once(|| {
        log::set_logger(&LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Info);
    });
}

/// Opt-in validation of the production helper and its parent-side log records.
/// Normal test runs must never flush host caches or rebuild performance counters.
#[test]
#[ignore = "runs native maintenance; requires an explicitly authorized Windows test host"]
fn authorized_native_maintenance_reports_real_steps() {
    assert_eq!(
        std::env::var("MANGODISK_NATIVE_MAINTENANCE_TEST").as_deref(),
        Ok("1")
    );
    install_native_test_logger();
    for task_id in [DNS_CACHE, PERFORMANCE_COUNTERS] {
        let outcome =
            run_with_privileges(task_id, &|_| {}).expect("authorized native task must complete");
        assert!(!outcome.diagnostics.steps.is_empty());
        assert!(outcome
            .diagnostics
            .steps
            .iter()
            .all(|step| step.result == MaintenanceStepResult::Succeeded));
        if task_id == PERFORMANCE_COUNTERS {
            assert_eq!(
                outcome.diagnostics.steps.last().unwrap().stage,
                MaintenanceStage::Verify
            );
        }
    }
}

#[test]
#[ignore = "attempts a native service start; requires an authorized host with Windows Update disabled"]
fn authorized_native_maintenance_disabled_service_retains_diagnostics() {
    assert_eq!(
        std::env::var("MANGODISK_NATIVE_MAINTENANCE_TEST").as_deref(),
        Ok("1")
    );
    install_native_test_logger();
    // The precondition is read-only. Never change the host's service configuration just to
    // manufacture an error; this test targets a machine that already has the service disabled.
    let failure = run_fixture(
        "",
        r#"
$service = Get-Service -Name wuauserv -ErrorAction Stop
if ($service.StartType -ne 'Disabled') { throw 'Test requires Windows Update to be disabled' }
try { Invoke-MangoService 'wuauserv' 'Start' }
finally {
    Send-MangoLine '{invalid-diagnostic-frame'
    Start-Sleep -Milliseconds 1000
}
"#,
    )
    .expect_err("a disabled service must not start");
    assert_eq!(
        failure.error.failure_reason(),
        Some(crate::PlatformFailureReason::ServiceDisabled)
    );
    assert_eq!(failure.native_error_code, Some(1058));
    let diagnostics = failure.diagnostics.unwrap();
    assert!(diagnostics.progress_channel_failed);
    assert_eq!(diagnostics.steps.len(), 1);
    let step = diagnostics.steps[0];
    assert_eq!(step.component, MaintenanceComponent::WindowsUpdate);
    assert_eq!(step.startup, Some(ServiceStartup::Disabled));
    assert_eq!(step.after, Some(ServiceStatus::Stopped));
    crate::system_maintenance_helper::step_diagnostics::log_step_diagnostics(
        "authorized-fault-test",
        UPDATE_COMPONENTS,
        1,
        &diagnostics.steps,
    );
    println!(
        "fault_injection_diagnostics scenario=native-disabled {}",
        serde_json::to_string(&diagnostics).unwrap()
    );
}

#[test]
fn performance_verification_requires_structured_data_and_reports_query_failures() {
    for mode in ["success", "empty", "permission"] {
        let prefix = format!(
            r#"
$mode='{mode}'
function Get-CimInstance($ClassName, $OperationTimeoutSec, $ErrorAction) {{
    if ($mode -eq 'permission') {{ throw [UnauthorizedAccessException]::new() }}
    if ($mode -eq 'empty') {{ return $null }}
    [pscustomobject]@{{ PercentProcessorTime=0; Timestamp_PerfTime=100 }}
}}
"#
        );
        // Keep the production verification recipe but replace repair commands with no-ops.
        let body = format!(
            "function Invoke-MangoNative {{}}\n{}",
            task_scripts::recipe(PERFORMANCE_COUNTERS).unwrap()
        );
        let result = run_fixture(&prefix, &body);
        match mode {
            "success" => assert_eq!(
                result.unwrap().diagnostics.steps.last().unwrap().stage,
                MaintenanceStage::Verify
            ),
            "empty" => assert_eq!(
                result.unwrap_err().error.failure_reason(),
                Some(crate::PlatformFailureReason::VerificationFailed)
            ),
            "permission" => assert_eq!(
                result.unwrap_err().error.failure_reason(),
                Some(crate::PlatformFailureReason::VerificationPermissionDenied)
            ),
            _ => unreachable!(),
        }
    }
}
