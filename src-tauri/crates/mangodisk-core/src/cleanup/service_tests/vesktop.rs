use std::{fs, io::Read, path::Path};

use mangodisk_platform::{current_platform, Platform};
#[cfg(windows)]
use std::path::PathBuf;

use crate::cleanup::{CleanupActionReason, CleanupActionStatus, CleanupRequest, CleanupService};

fn persistent_digest(root: &Path) -> (u64, blake3::Hash) {
    let mut paths = vec![root.to_path_buf()];
    let mut entries = Vec::new();
    while let Some(path) = paths.pop() {
        let relative = path.strip_prefix(root).unwrap();
        if relative.starts_with("sessionData/Cache")
            || relative.starts_with("sessionData/Code Cache")
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).unwrap();
        assert!(
            !current_platform().is_link_like(&metadata),
            "durable test state must not follow links"
        );
        if metadata.is_dir() {
            for child in fs::read_dir(&path).unwrap() {
                paths.push(child.unwrap().path());
            }
        } else {
            assert!(metadata.is_file());
            let mut hash = blake3::Hasher::new();
            let mut file = fs::File::open(&path).unwrap();
            let mut buffer = [0; 64 * 1024];
            loop {
                let count = file.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            entries.push((relative.to_path_buf(), hash.finalize()));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hash = blake3::Hasher::new();
    for (path, digest) in &entries {
        hash.update(path.to_string_lossy().as_bytes());
        hash.update(digest.as_bytes());
    }
    (entries.len() as u64, hash.finalize())
}

#[test]
#[ignore = "inspects or clears only the real Vesktop cache after explicit opt-in"]
fn vesktop_real_cache_lifecycle_validation() {
    let _lock = crate::shared::operation::test_operation_lock();
    let mode =
        std::env::var("MANGODISK_VERIFY_VESKTOP").expect("explicit blocked or apply mode required");
    assert!(["blocked", "apply"].contains(&mode.as_str()));
    #[cfg(target_os = "macos")]
    let root = current_platform()
        .user_directories()
        .unwrap()
        .home_directory()
        .join("Library/Application Support/vesktop");
    #[cfg(windows)]
    let root = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("vesktop");
    assert!(
        root.is_dir(),
        "initialize the real standard Vesktop installation first"
    );
    let request = |dry_run| CleanupRequest {
        rule_ids: vec!["app.vesktop-cache".to_string()],
        source_selections: Vec::new(),
        dry_run,
        project_roots: Vec::new(),
    };
    if mode == "blocked" {
        let result = CleanupService::execute(request(false)).unwrap();
        assert_eq!(result.actions.len(), 1);
        assert_eq!(result.actions[0].status, CleanupActionStatus::Blocked);
        assert_eq!(
            result.actions[0].reason_code,
            Some(CleanupActionReason::RunningProcesses)
        );
        assert_eq!(result.released_bytes, 0);
        eprintln!(
            "vesktop_real_validation mode=blocked released_bytes=0 running_owner_preserved=true"
        );
        return;
    }
    let before = persistent_digest(&root);
    assert!(
        before.0 > 0,
        "the real client must have durable state to verify"
    );
    let preview = CleanupService::execute(request(true)).unwrap();
    assert_eq!(
        preview.actions[0].status,
        CleanupActionStatus::Previewed,
        "{:?}",
        preview.actions
    );
    assert!(
        preview.actions[0].bytes_expected > 0,
        "the app must generate real cache data first"
    );
    assert_eq!(preview.released_bytes, 0);
    assert_eq!(before, persistent_digest(&root));
    let result = CleanupService::execute(request(false)).unwrap();
    assert_eq!(
        result.actions[0].status,
        CleanupActionStatus::Completed,
        "{:?}",
        result.actions
    );
    assert_eq!(result.failed_item_count, 0);
    assert!(result.affected_item_count > 0);
    assert_eq!(before, persistent_digest(&root));
    let remaining = CleanupService::execute(request(true)).unwrap();
    assert_eq!(remaining.actions[0].bytes_expected, 0);
    eprintln!("vesktop_real_validation mode=apply expected_bytes={} released_bytes={} affected_files={} preserved_state_files={} rescan_cache_bytes=0", preview.actions[0].bytes_expected, result.released_bytes, result.affected_item_count, before.0);
}
