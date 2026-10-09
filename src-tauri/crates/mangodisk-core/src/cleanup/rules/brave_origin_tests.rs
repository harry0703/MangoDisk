use std::{fs, path::Path};

use super::super::declarative_schema::SourcePlatform;
use super::{
    compile_declarative_source, current_source_platform, parse_catalog,
    EMBEDDED_DECLARATIVE_RULE_SOURCES,
};
use crate::{
    applications::catalog::ProcessSnapshot,
    cleanup::{
        exclusions::CleanupExclusions,
        rule_execution::{execute_rule, measure_owned_rule, RuleExecutionContext},
        rules::{compile_scan_plan, validation::compile_rules, ScanPlan},
        CleanupActionReason, CleanupActionResult, CleanupActionStatus,
    },
    shared::operation::{CoordinatedOperationKind, OperationGuard},
};

const CACHE: &[u8] = b"regenerable browser cache";
const STATE: &[u8] = b"browser state must remain unchanged";

fn write(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn plan(sandbox: &Path, platform: SourcePlatform, process_names: Option<&[&str]>) -> ScanPlan {
    let mut source = parse_catalog(EMBEDDED_DECLARATIVE_RULE_SOURCES)
        .unwrap()
        .into_iter()
        .find(|parsed| {
            parsed.rule.id == "browser.brave-origin-cache" && parsed.rule.platform == platform
        })
        .expect("the production Origin rule must exist")
        .rule;
    // Exercise both production layouts on each host without changing process
    // environment or touching real profiles. Keep every production suffix intact.
    if let Some(names) = process_names {
        // Runtime preflight refreshes real processes during slow cleanup. Give
        // successful fixture executions identities unrelated to installed browsers.
        source.required_stopped_processes = names.iter().map(|name| (*name).to_string()).collect();
    }
    source.platform = current_source_platform();
    source.verification.verified_platform = current_source_platform();
    let base = format!(
        "${{temp}}/{}",
        sandbox.file_name().unwrap().to_str().unwrap()
    );
    for root in &mut source.roots {
        root.template = root
            .template
            .replace("${local_app_data}", &base)
            .replace("${user_library}", &format!("{base}/Library"))
            .replace(
                "${application_support}",
                &format!("{base}/Library/Application Support"),
            );
    }
    let mut rule = compile_declarative_source(source).unwrap();
    for root in &mut rule.roots {
        if root.resolved_path.exists() {
            root.resolved_path = fs::canonicalize(&root.resolved_path).unwrap();
        }
    }
    let rules = compile_rules(vec![rule]).unwrap();
    compile_scan_plan(rules, &[true], &[]).unwrap()
}

fn execute(plan: &ScanPlan, dry_run: bool, processes: &ProcessSnapshot) -> CleanupActionResult {
    let measured = measure_owned_rule(plan, 0, None, &CleanupExclusions::default()).unwrap();
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    let action = execute_rule(
        &plan.rules[0],
        0,
        Some(measured),
        &RuleExecutionContext {
            ownership_plan: plan,
            process_snapshot: processes,
            source_scope: None,
            empty_directory_authorizations: None,
            operation: &operation,
            dry_run,
        },
        &mut |_, _| {},
    );
    operation.complete();
    action
}

#[test]
fn brave_origin_cache_cleanup_preserves_profiles_and_regular_brave() {
    let _lock = crate::shared::operation::test_operation_lock();
    for platform in [SourcePlatform::Windows, SourcePlatform::Macos] {
        let sandbox = tempfile::tempdir().unwrap();
        let base = match platform {
            SourcePlatform::Windows => sandbox.path().to_path_buf(),
            SourcePlatform::Macos => sandbox.path().join("Library/Application Support"),
            SourcePlatform::Linux => unreachable!(),
        };
        let relative_data = if platform == SourcePlatform::Windows {
            "BraveSoftware/Brave-Origin/User Data"
        } else {
            "BraveSoftware/Brave-Origin"
        };
        let data = base.join(relative_data);
        let mut caches = Vec::new();
        let mut preserved = vec![
            data.join("Local State"),
            base.join(relative_data.replace("Brave-Origin", "Brave-Browser"))
                .join("Default/Cache/regular.bin"),
            sandbox
                .path()
                .join("Library/Caches/BraveSoftware/Brave-Browser/Default/Cache/regular.bin"),
            data.join("Other Profile/Cache/unrecognized.bin"),
            data.join("Profile/Cache/unrecognized.bin"),
            data.join("Default/nested/Cache/unrecognized.bin"),
        ];
        if platform == SourcePlatform::Macos {
            caches.push(
                sandbox
                    .path()
                    .join("Library/Caches/BraveSoftware/Brave-Origin/Default/Cache/payload.bin"),
            );
        }
        for name in [
            "ShaderCache",
            "GrShaderCache",
            "GraphiteDawnCache",
            "DawnGraphiteCache",
            "DawnWebGPUCache",
            "component_crx_cache",
            "extensions_crx_cache",
        ] {
            caches.push(data.join(name).join("payload.bin"));
        }
        for profile in ["Default", "Profile 1", "Guest Profile", "System Profile"] {
            for cache in [
                "Cache",
                "Code Cache",
                "GPUCache",
                "DawnCache",
                "DawnGraphiteCache",
                "DawnWebGPUCache",
                "GrShaderCache",
                "GraphiteDawnCache",
                "Media Cache",
            ] {
                caches.push(data.join(profile).join(cache).join("nested/payload.bin"));
            }
            for state in [
                "Preferences",
                "Secure Preferences",
                "Bookmarks",
                "History",
                "Login Data",
                "Network/Cookies",
                "Extensions/extension/manifest.json",
                "Local Storage/store.bin",
                "IndexedDB/store.bin",
                "Service Worker/ScriptCache/script.bin",
            ] {
                preserved.push(data.join(profile).join(state));
            }
        }
        for path in &caches {
            write(path, CACHE);
        }
        for path in &preserved {
            write(path, STATE);
        }
        let plan = plan(
            sandbox.path(),
            platform,
            Some(&["mangodisk-origin-cache-fixture-never-running.exe"]),
        );
        let idle = ProcessSnapshot::default();
        let preview = execute(&plan, true, &idle);
        assert_eq!(
            preview.status,
            CleanupActionStatus::Previewed,
            "{preview:?}"
        );
        assert_eq!(preview.released_bytes, 0);
        assert_eq!(preview.bytes_expected, (caches.len() * CACHE.len()) as u64);
        for path in &caches {
            assert_eq!(fs::read(path).unwrap(), CACHE);
        }
        let result = execute(&plan, false, &idle);
        assert_eq!(result.status, CleanupActionStatus::Completed, "{result:?}");
        assert_eq!(result.failed_item_count, 0);
        assert_eq!(result.affected_item_count, caches.len() as u64);
        assert_eq!(result.released_bytes, (caches.len() * CACHE.len()) as u64);
        for path in &caches {
            assert!(!path.exists(), "{path:?}");
        }
        for path in &preserved {
            assert_eq!(fs::read(path).unwrap(), STATE, "{path:?}");
        }
    }
}

#[test]
fn brave_origin_running_browser_blocks_cleanup() {
    let _lock = crate::shared::operation::test_operation_lock();
    for platform in [SourcePlatform::Windows, SourcePlatform::Macos] {
        let sandbox = tempfile::tempdir().unwrap();
        let cache = sandbox.path().join(if platform == SourcePlatform::Windows {
            "BraveSoftware/Brave-Origin/User Data/Default/Cache/payload.bin"
        } else {
            "Library/Application Support/BraveSoftware/Brave-Origin/Default/Cache/payload.bin"
        });
        write(&cache, CACHE);
        let plan = plan(sandbox.path(), platform, None);
        let names: &[&str] = if platform == SourcePlatform::Windows {
            &["brave.exe", "BRAVE.EXE", "Brave Origin"]
        } else {
            &["Brave Origin"]
        };
        for name in names {
            let running = ProcessSnapshot::from_process_names(vec![name.to_string()]);
            let result = execute(&plan, false, &running);
            assert_eq!(
                result.status,
                CleanupActionStatus::Blocked,
                "{name}: {result:?}"
            );
            assert_eq!(
                result.reason_code,
                Some(CleanupActionReason::RunningProcesses)
            );
            assert_eq!(result.released_bytes, 0);
            assert_eq!(result.affected_item_count, 0);
            assert_eq!(fs::read(&cache).unwrap(), CACHE);
        }
    }
}

#[test]
fn brave_origin_without_cache_keeps_durable_state_outside_roots() {
    let _lock = crate::shared::operation::test_operation_lock();
    for platform in [SourcePlatform::Windows, SourcePlatform::Macos] {
        let sandbox = tempfile::tempdir().unwrap();
        let state = sandbox.path().join(if platform == SourcePlatform::Windows {
            "BraveSoftware/Brave-Origin/User Data/Default/Bookmarks"
        } else {
            "Library/Application Support/BraveSoftware/Brave-Origin/Default/Bookmarks"
        });
        let missing = plan(sandbox.path(), platform, None);
        assert_eq!(
            measure_owned_rule(&missing, 0, None, &CleanupExclusions::default())
                .unwrap()
                .bytes,
            0
        );
        write(&state, STATE);
        let state_only = plan(sandbox.path(), platform, None);
        assert_eq!(
            measure_owned_rule(&state_only, 0, None, &CleanupExclusions::default())
                .unwrap()
                .bytes,
            0
        );
        assert_eq!(fs::read(&state).unwrap(), STATE);
    }
}
