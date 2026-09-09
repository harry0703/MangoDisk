use std::{cell::Cell, process::Command, sync::Mutex};

use super::*;
use crate::shared::operation::{test_operation_lock, CoordinatedOperationKind};

const ARTIFACT_BYTES: usize = 4096;
const SOURCE: &str = "fn main() { println!(\"source must remain\"); }";

struct WorktreeFixture {
    root: tempfile::TempDir,
    home: PathBuf,
    checkout: PathBuf,
    admin: PathBuf,
}

impl WorktreeFixture {
    fn new(real_git: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("codex");
        let checkout = home.join("worktrees/test/project");
        let repository = root.path().join("repository");
        fs::create_dir_all(&repository).unwrap();
        fs::create_dir_all(checkout.parent().unwrap()).unwrap();
        write_project(&repository);
        let admin = repository.join(".git/worktrees/project");
        if real_git {
            git(&repository, &["init", "--quiet"]);
            git(&repository, &["add", "."]);
            git(
                &repository,
                &[
                    "-c",
                    "user.name=MangoDisk Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--quiet",
                    "-m",
                    "test fixture",
                ],
            );
            git(
                &repository,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "--detach",
                    checkout.to_str().unwrap(),
                ],
            );
        } else {
            fs::create_dir_all(&checkout).unwrap();
            fs::create_dir_all(&admin).unwrap();
            write_project(&checkout);
            fs::write(
                checkout.join(".git"),
                format!("gitdir: {}", admin.display()),
            )
            .unwrap();
            fs::write(
                admin.join("gitdir"),
                checkout.join(".git").to_str().unwrap(),
            )
            .unwrap();
        }
        // Real Codex installations may omit this optional metadata. The
        // configured location and reciprocal Git pointers must suffice.
        assert!(!admin.join("codex-thread.json").exists());
        assert!(codex_worktrees::is_linked_checkout(&checkout));
        write_artifact(&checkout);
        Self {
            root,
            home,
            checkout,
            admin,
        }
    }

    fn plan(&self, mixed: bool) -> CatalogPlan {
        let mut roots = vec![display_path(&self.checkout)];
        if mixed {
            let ordinary = self.root.path().join("ordinary");
            fs::create_dir_all(&ordinary).unwrap();
            write_project(&ordinary);
            write_artifact(&ordinary);
            roots.push(display_path(&ordinary));
        }
        build_plan_with_progress(
            &roots,
            false,
            current_platform_rules().unwrap(),
            &|| false,
            &|_| {},
            &|_, _, _| {},
            Some(&self.home),
        )
        .unwrap()
    }

    fn assert_source_preserved(&self) {
        assert_eq!(
            fs::read_to_string(self.checkout.join("src/main.rs")).unwrap(),
            SOURCE
        );
        assert!(self.checkout.join("Cargo.toml").is_file());
        assert!(self.checkout.join(".git").is_file());
    }
}

fn git(repository: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "fixture Git operation failed");
}

fn write_project(project: &Path) {
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(project.join("src/main.rs"), SOURCE).unwrap();
    fs::write(project.join(".gitignore"), "/target/\n").unwrap();
}

fn write_artifact(project: &Path) {
    fs::create_dir_all(project.join("target/debug")).unwrap();
    fs::write(
        project.join("target/debug/fixture.bin"),
        vec![0x5a; ARTIFACT_BYTES],
    )
    .unwrap();
}

fn rust_rule(plan: &CatalogPlan) -> &RulePlan {
    let rule = plan
        .rules
        .iter()
        .find(|rule| rule.source.id == "project.rust-build-artifacts")
        .unwrap();
    assert!(rule
        .candidates
        .iter()
        .any(|candidate| candidate.codex_checkout.is_some()));
    rule
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    Running,
    InspectionFailed,
    Changed,
    Ready,
    Mixed,
    DryRun,
}

fn verify_scenario(scenario: Scenario, real_git: bool) {
    let fixture = WorktreeFixture::new(real_git);
    let mixed = matches!(scenario, Scenario::Mixed);
    let plan = fixture.plan(mixed);
    let rule = rust_rule(&plan);
    assert_eq!(rule.candidates.len(), if mixed { 2 } else { 1 });
    if matches!(scenario, Scenario::Changed) {
        fs::remove_file(fixture.admin.join("gitdir")).unwrap();
    }
    let calls = Cell::new(0);
    let process_check = || {
        calls.set(calls.get() + 1);
        match scenario {
            Scenario::Running | Scenario::Mixed => {
                Ok(vec!["Codex".into(), "codex".into(), "ChatGPT".into()])
            }
            Scenario::InspectionFailed => Err("private-process-inspection-detail".into()),
            _ => Ok(Vec::new()),
        }
    };
    let _lock = test_operation_lock();
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    let action = execute_rule_with_process_check(
        rule,
        None,
        matches!(scenario, Scenario::DryRun),
        &operation,
        &process_check,
    );
    let (status, reason, released) = match scenario {
        Scenario::Running => (
            CleanupActionStatus::Blocked,
            Some(CleanupActionReason::RunningProcesses),
            0,
        ),
        Scenario::InspectionFailed | Scenario::Changed => (
            CleanupActionStatus::Failed,
            Some(CleanupActionReason::PreflightFailed),
            0,
        ),
        Scenario::Ready => (CleanupActionStatus::Completed, None, ARTIFACT_BYTES as u64),
        Scenario::Mixed => (
            CleanupActionStatus::Partial,
            Some(CleanupActionReason::RunningProcesses),
            ARTIFACT_BYTES as u64,
        ),
        Scenario::DryRun => (CleanupActionStatus::Previewed, None, 0),
    };
    assert_eq!(action.status, status, "{scenario:?}");
    assert_eq!(action.reason_code, reason);
    assert_eq!(action.released_bytes, released);
    assert_eq!(action.affected_item_count, u64::from(released > 0));
    assert_eq!(
        calls.get(),
        usize::from(!matches!(scenario, Scenario::Changed | Scenario::DryRun))
    );
    assert_eq!(
        fixture.checkout.join("target").exists(),
        !matches!(scenario, Scenario::Ready)
    );
    fixture.assert_source_preserved();
    if matches!(scenario, Scenario::Running | Scenario::Mixed) {
        assert_eq!(action.running_processes, vec!["Codex", "ChatGPT"]);
    } else {
        assert!(action.running_processes.is_empty());
    }
    if mixed {
        assert!(!fixture.root.path().join("ordinary/target").exists());
        assert_eq!(
            fs::read_to_string(fixture.root.path().join("ordinary/src/main.rs")).unwrap(),
            SOURCE
        );
    }
    println!("codex_execution_validation scenario={scenario:?} real_git={real_git} status={status:?} released_bytes={released} source_preserved=true");
}

#[test]
fn codex_execution_guards_and_real_artifact_deletion() {
    for scenario in [
        Scenario::Running,
        Scenario::InspectionFailed,
        Scenario::Changed,
        Scenario::Ready,
        Scenario::Mixed,
        Scenario::DryRun,
    ] {
        verify_scenario(scenario, false);
    }
}

// Installed only by the explicitly requested, isolated validation run. Normal
// unit tests do not compete with the application or other tests for a logger.
static RECORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());
struct ValidationLogger;
impl log::Log for ValidationLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        if record.target().contains("project_artifacts") {
            RECORDS.lock().unwrap().push(record.args().to_string());
        }
    }
    fn flush(&self) {}
}

#[test]
#[ignore = "creates disposable Git worktrees and deletes only generated fixture artifacts"]
fn codex_real_git_worktree_deletion_and_diagnostics() {
    log::set_logger(&ValidationLogger).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    // This phase uses the live platform inventory with no injected process
    // state. This explicitly requested validation requires a running Codex app.
    assert!(
        !codex_worktrees::blocking_processes().unwrap().is_empty(),
        "start Codex or ChatGPT for live guard validation"
    );
    let fixture = WorktreeFixture::new(true);
    let plan = fixture.plan(false);
    {
        let _lock = test_operation_lock();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
        let action = execute_rule(rust_rule(&plan), None, false, &operation);
        assert_eq!(action.status, CleanupActionStatus::Blocked);
        assert_eq!(
            action.reason_code,
            Some(CleanupActionReason::RunningProcesses)
        );
        assert!(!action.running_processes.is_empty());
        assert_eq!(action.released_bytes, 0);
        assert!(fixture.checkout.join("target/debug/fixture.bin").exists());
        fixture.assert_source_preserved();
        println!("codex_execution_validation live_process_guard=passed released_bytes=0");
    }
    for scenario in [
        Scenario::Running,
        Scenario::InspectionFailed,
        Scenario::Changed,
        Scenario::Ready,
        Scenario::Mixed,
        Scenario::DryRun,
    ] {
        verify_scenario(scenario, true);
    }
    let records = RECORDS.lock().unwrap().join("\n");
    for reason in [
        "codexWritersRunning",
        "codexProcessInspectionFailed",
        "codexCheckoutChanged",
        "project_artifact_execution_finished",
        "status=Completed",
        "status=Partial",
        "error_digest=",
    ] {
        assert!(records.contains(reason), "missing diagnostic: {reason}");
    }
    assert!(!records.contains("private-process-inspection-detail"));
    assert!(!records.contains(fixture.root.path().to_str().unwrap()));
    for record in records.lines().filter(|line| {
        line.contains("project_artifact_delete_skipped")
            || line.contains("project_artifact_execution_finished")
    }) {
        println!("{record}");
    }
    println!("codex_execution_validation diagnostics=passed");
}
