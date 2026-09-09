//! Explicit, read-only validation of the normal cleanup entry point against
//! installed Codex worktrees. Reports contain counts and bytes, never paths.

use mangodisk_core::{
    configure_application_paths, ApplicationPaths, CleanupScanService, CleanupSourceBlockReason,
};
use mangodisk_platform::{current_platform, Platform};

#[test]
#[ignore = "scans real cleanup locations without deleting files"]
fn standard_scan_includes_codex_worktree_artifacts() {
    let fixture = tempfile::tempdir().unwrap();
    configure_application_paths(
        ApplicationPaths::from_base_directories(
            fixture.path().join("data"),
            fixture.path().join("cache"),
        )
        .unwrap(),
    )
    .unwrap();
    let home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            current_platform()
                .user_directories()
                .unwrap()
                .home_directory()
                .join(".codex")
        });
    let root = std::fs::canonicalize(home.join("worktrees"))
        .expect("validation requires installed Codex worktrees");
    // Optional targeting prevents a different checkout from hiding a regression
    // in the real worktree under investigation. No private paths are printed.
    let target = std::env::var_os("MANGODISK_TEST_WORKTREE")
        .map(|path| std::fs::canonicalize(path).expect("validation worktree must exist"))
        .unwrap_or_else(|| root.clone());
    assert!(current_platform().path_is_same_or_child(&target, &root));
    let started = std::time::Instant::now();
    let result = CleanupScanService::scan_with_progress(|_| {}).unwrap();
    let sources = result
        .rules
        .iter()
        .filter(|rule| rule.rule_id.starts_with("project."))
        .flat_map(|rule| &rule.sources)
        .filter(|source| {
            current_platform().path_is_same_or_child(std::path::Path::new(&source.path), &target)
        })
        .collect::<Vec<_>>();
    assert!(
        !sources.is_empty(),
        "normal scanning must discover installed Codex build artifacts"
    );
    assert!(
        sources
            .iter()
            .all(|source| !current_platform()
                .paths_equal(std::path::Path::new(&source.path), &target))
    );
    let requires_close_count = sources
        .iter()
        .filter(|source| source.block_reason == Some(CleanupSourceBlockReason::RequiresClose))
        .count();
    let limited_count = sources
        .iter()
        .filter(|source| {
            source.block_reason == Some(CleanupSourceBlockReason::IncompleteMeasurement)
        })
        .count();
    if std::env::var("MANGODISK_TEST_REQUIRE_PROCESS_GUARD").as_deref() == Ok("1") {
        assert!(
            sources.iter().all(|source| source.block_reason.is_some()),
            "active worktree artifacts must retain the close requirement"
        );
        assert!(
            result
                .rules
                .iter()
                .filter(|rule| rule.sources.iter().any(|source| {
                    current_platform()
                        .path_is_same_or_child(std::path::Path::new(&source.path), &target)
                        && source.block_reason == Some(CleanupSourceBlockReason::RequiresClose)
                }))
                .all(|rule| rule.selectable && rule.requires_app_close),
            "known Codex apps must allow selection and user-confirmed closure"
        );
        assert!(
            result.rules.iter().any(|rule| {
                rule.sources.iter().any(|source| {
                    current_platform()
                        .path_is_same_or_child(std::path::Path::new(&source.path), &target)
                }) && !rule.running_processes.is_empty()
            }),
            "the real process inventory must identify active writers"
        );
    }
    println!(
        "standard_codex_validation source_count={} estimated_bytes={} requires_close_count={} limited_count={} elapsed_ms={}",
        sources.len(),
        sources.iter().map(|source| source.bytes).sum::<u64>(),
        requires_close_count,
        limited_count,
        started.elapsed().as_millis()
    );
}
