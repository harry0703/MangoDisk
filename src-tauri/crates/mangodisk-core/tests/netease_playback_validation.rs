#![cfg(any(target_os = "macos", windows))]

use mangodisk_core::{
    configure_application_paths, ApplicationPaths, CleanupActionReason, CleanupActionStatus,
    CleanupRequest, CleanupService,
};

fn configure_test_storage() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("create isolated storage");
    configure_application_paths(
        ApplicationPaths::new(
            root.path().join("data"),
            root.path().join("cache"),
            root.path().join("runtime"),
        )
        .unwrap(),
    )
    .unwrap();
    root
}

fn request(dry_run: bool) -> CleanupRequest {
    CleanupRequest {
        rule_ids: vec!["app.netease-music-playback-cache".to_string()],
        source_selections: Vec::new(),
        dry_run,
        project_roots: Vec::new(),
    }
}

#[test]
#[ignore = "requires the real NetEase Music client to be running"]
fn playback_cache_blocks_running_client() {
    let _storage = configure_test_storage();
    assert_eq!(
        std::env::var("MANGODISK_TEST_NETEASE_BLOCK").as_deref(),
        Ok("1")
    );
    let result = CleanupService::execute(request(false)).expect("cleanup must return a result");
    assert_eq!(result.actions.len(), 1);
    assert_eq!(result.actions[0].status, CleanupActionStatus::Blocked);
    assert_eq!(
        result.actions[0].reason_code,
        Some(CleanupActionReason::RunningProcesses)
    );
    assert_eq!(result.released_bytes, 0);
}

#[test]
#[ignore = "permanently removes the real NetEase Music playback cache"]
fn playback_cache_real_cleanup() {
    let _storage = configure_test_storage();
    assert_eq!(
        std::env::var("MANGODISK_TEST_NETEASE_CLEAN").as_deref(),
        Ok("1")
    );
    let preview = CleanupService::execute(request(true)).expect("preview must return a result");
    assert_eq!(preview.failed_item_count, 0, "{:?}", preview.actions);
    assert!(
        preview.expected_bytes > 0,
        "a real playback cache must exist"
    );
    let result = CleanupService::execute(request(false)).expect("cleanup must return a result");
    assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
    assert_eq!(result.actions[0].status, CleanupActionStatus::Completed);
    assert!(result.released_bytes > 0);
    println!(
        "preview_bytes={} released_bytes={} affected_items={}",
        preview.expected_bytes, result.released_bytes, result.affected_item_count
    );
}
