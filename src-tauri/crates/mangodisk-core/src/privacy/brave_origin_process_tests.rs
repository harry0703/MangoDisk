use super::*;

fn install_brave_close_plan(path: Option<PathBuf>) -> (tempfile::TempDir, String) {
    let fixture = tempfile::tempdir().unwrap();
    fs::write(fixture.path().join("payload"), b"fixture").unwrap();
    let roots = vec![fixture.path().to_path_buf()];
    let fingerprint = roots_summary_fingerprint(&roots, &PlatformCancellation::new(|| false))
        .unwrap()
        .0;
    let token = "c".repeat(64);
    let item = privacy_item(PrivacyItemInput {
        token: token.clone(),
        source_id: "brave-origin".into(),
        source_name: "Brave Origin".into(),
        profile_id: Some("brave-origin:Default".into()),
        profile_name: Some("Default".into()),
        kind: PrivacyDataKind::SiteStorage,
        capability: PrivacyCapabilityState::BrowserRunning,
        item_count: 1,
        estimated_bytes: 7,
        requires_browser_close: true,
    });
    let candidate = NativePrivacyCandidate {
        token: token.clone(),
        item: item.clone(),
        fingerprint,
        action: NativePrivacyAction::Directories { roots },
        browser_process_names: vec!["brave.exe".into()],
    };
    replace_scan_session(PrivacyScanSession {
        browser_application_paths: path
            .map(|path| BTreeMap::from([("brave-origin".into(), path)]))
            .unwrap_or_default(),
        public_result: PrivacyScanResult {
            schema_version: PRIVACY_SCAN_SCHEMA_VERSION,
            scan_id: "brave-close-scan".into(),
            revision: "fixture".into(),
            time_range: PrivacyTimeRange::AllTime,
            scanned_at_ms: 1,
            elapsed_ms: 1,
            items: vec![item],
            coverage: Vec::new(),
        },
        candidates: BTreeMap::from([(token.clone(), candidate)]),
    })
    .unwrap();
    let plan = PrivacyService::prepare(PrivacyExecutionRequest {
        scan_id: "brave-close-scan".into(),
        tokens: vec![token],
    })
    .unwrap();
    (fixture, plan.plan_id)
}

#[test]
fn brave_origin_close_plan_preserves_its_verified_path() {
    let _guard = crate::shared::operation::test_operation_lock();
    let path = PathBuf::from(r"C:\Program Files\BraveSoftware\Brave-Origin\Application\brave.exe");
    let (_fixture, plan_id) = install_brave_close_plan(Some(path.clone()));
    let targets = resolve_browser_process_targets(&plan_id, &["brave-origin".into()]).unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].executable_paths, vec![path]);
    assert!(resolve_browser_process_targets(&plan_id, &["brave".into()]).is_err());
    clear_scan_session().unwrap();
    clear_pending_plan().unwrap();
}

#[test]
fn brave_origin_without_verified_path_rejects_automatic_close() {
    let _guard = crate::shared::operation::test_operation_lock();
    let (_fixture, plan_id) = install_brave_close_plan(None);
    let error = PrivacyService::close_browsers(PrivacyBrowserCloseRequest {
        plan_id,
        source_ids: vec!["brave-origin".into()],
        mode: crate::ApplicationCloseMode::Force,
    })
    .unwrap_err();
    assert_eq!(error.code(), CoreErrorCode::OperationFailed);
    clear_scan_session().unwrap();
    clear_pending_plan().unwrap();
}

struct FixtureProcess(std::process::Child);

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_fixture(directory: &Path) -> FixtureProcess {
    fs::create_dir_all(directory).unwrap();
    let executable = directory.join("brave.exe");
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let ready = directory.join("ready");
    let child = FixtureProcess(
        std::process::Command::new(executable)
            .args([
                "--ignored",
                "--exact",
                "privacy::service::brave_origin_process_tests::brave_process_fixture_waits",
            ])
            .env("MANGODISK_BRAVE_FIXTURE_READY", &ready)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "fixture process must initialize");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    child
}

#[test]
#[ignore = "child entry point for the isolated Brave process test"]
fn brave_process_fixture_waits() {
    let Some(ready) = std::env::var_os("MANGODISK_BRAVE_FIXTURE_READY") else {
        return;
    };
    fs::write(ready, b"ready").unwrap();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

#[test]
#[ignore = "starts and closes only generated same-name process fixtures"]
fn actual_brave_origin_close_preserves_other_same_name_processes() {
    let _guard = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().unwrap();
    let mut origin = spawn_fixture(&fixture.path().join("Brave-Origin/Application"));
    let mut regular = spawn_fixture(&fixture.path().join("Brave-Browser/Application"));
    let mut unrelated = spawn_fixture(&fixture.path().join("unrelated"));
    let path = fixture.path().join("Brave-Origin/Application/brave.exe");
    let (_data, plan_id) = install_brave_close_plan(Some(path.clone()));
    let targets = resolve_browser_process_targets(&plan_id, &["brave-origin".into()]).unwrap();
    assert_eq!(targets[0].executable_paths, vec![path]);
    let result = PrivacyService::close_browsers(PrivacyBrowserCloseRequest {
        plan_id: plan_id.clone(),
        source_ids: vec!["brave-origin".into()],
        mode: crate::ApplicationCloseMode::Force,
    })
    .unwrap();
    assert_eq!(result.matched_process_count, 1, "{result:?}");
    assert_eq!(result.requested_process_count, 1, "{result:?}");
    assert_eq!(result.remaining_process_count, 0, "{result:?}");
    assert_eq!(result.failed_target_count, 0, "{result:?}");
    assert!(origin.0.try_wait().unwrap().is_some());
    assert!(regular.0.try_wait().unwrap().is_none());
    assert!(unrelated.0.try_wait().unwrap().is_none());
    // A successful exact-image close must not authorize deletion while another
    // same-name writer remains; execution owns that conservative safety check.
    let execution = PrivacyService::execute(PrivacyExecutionRunRequest {
        plan_id,
        excluded_source_ids: Vec::new(),
    })
    .unwrap();
    assert_eq!(execution.affected_item_count, 0, "{execution:?}");
    assert_eq!(execution.failed_item_count, 1, "{execution:?}");
    assert_eq!(fs::read(_data.path().join("payload")).unwrap(), b"fixture");
    println!("origin_closed=1 regular_preserved=1 unrelated_same_name_preserved=1 close_completed=1 cleanup_still_blocked=1");
    clear_scan_session().unwrap();
    clear_pending_plan().unwrap();
}
