use super::{compile_declarative_source, parse_current_platform_catalog};
use std::{fs, path::Path};

use crate::{
    applications::catalog::ProcessSnapshot,
    cleanup::{
        exclusions::CleanupExclusions,
        rule_execution::{execute_rule, measure_owned_rule, RuleExecutionContext},
        rules::{compile_scan_plan, models::RuleSpec, validation::compile_rules, ScanPlan},
        CleanupActionResult, CleanupActionStatus,
    },
    shared::operation::{CoordinatedOperationKind, OperationGuard},
};

const CACHE: &[u8] = b"rebuildable fixture";
const STATE: &[u8] = b"persistent fixture must remain unchanged";

#[cfg(target_os = "macos")]
#[test]
fn netease_playback_cache_preserves_downloads_and_account_data() {
    let _lock = crate::shared::operation::test_operation_lock();
    let sandbox = tempfile::Builder::new()
        .prefix("mangodisk-netease-")
        .tempdir()
        .unwrap();
    let data = sandbox
        .path()
        .join("home/Library/Application Support/com.netease.163music");
    let cache = data.join("Caches/online_play_cache/stream.bin");
    let state = [
        data.join("Caches/orpheus_cache/state.bin"),
        data.join("Documents/storage/account.bin"),
        sandbox.path().join("home/Music/NetEase/download.mp3"),
    ];
    write_fixture(&cache, CACHE);
    for path in &state {
        write_fixture(path, STATE);
    }
    let plan = isolated_plan("app.netease-music-playback-cache", sandbox.path());
    execute_fixture(&plan, 0, true);
    assert_eq!(fs::read(&cache).unwrap(), CACHE);
    execute_fixture(&plan, 0, false);
    assert!(!cache.exists());
    for path in &state {
        assert_eq!(fs::read(path).unwrap(), STATE);
    }
}

#[cfg(windows)]
#[test]
fn netease_playback_cache_preserves_unknown_files_and_persistent_data() {
    let _lock = crate::shared::operation::test_operation_lock();
    let sandbox = tempfile::Builder::new()
        .prefix("mangodisk-netease-")
        .tempdir()
        .unwrap();
    let data = sandbox.path().join("home/AppData/Local/NetEase/CloudMusic");
    let cache = [
        data.join("Cache/Cache/stream.uc"),
        data.join("Cache/Cache/stream.idx"),
        data.join("Cache/Cache/stream.info"),
    ];
    let state = [
        data.join("Cache/Cache/unknown.dat"),
        data.join("Cache/other.uc"),
        data.join("Library/library.dat"),
        data.join("localdata"),
        sandbox.path().join("home/Music/NetEase/download.mp3"),
    ];
    for path in &cache {
        write_fixture(path, CACHE);
    }
    for path in &state {
        write_fixture(path, STATE);
    }
    let plan = isolated_plan("app.netease-music-playback-cache", sandbox.path());
    execute_fixture(&plan, 0, true);
    for path in &cache {
        assert_eq!(fs::read(path).unwrap(), CACHE);
    }
    execute_fixture(&plan, 0, false);
    for path in &cache {
        assert!(!path.exists());
    }
    for path in &state {
        assert_eq!(fs::read(path).unwrap(), STATE);
    }
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn vesktop_cache_cleanup_preserves_authentication_settings_and_themes() {
    let _lock = crate::shared::operation::test_operation_lock();
    let sandbox = tempfile::Builder::new()
        .prefix("mangodisk-vesktop-")
        .tempdir()
        .unwrap();
    #[cfg(target_os = "macos")]
    let data = sandbox
        .path()
        .join("home/Library/Application Support/vesktop");
    #[cfg(windows)]
    let data = sandbox.path().join("home/AppData/Roaming/vesktop");
    #[cfg(any(windows, target_os = "macos"))]
    {
        let caches = [
            data.join("sessionData/Cache/Cache_Data/fixture.bin"),
            data.join("sessionData/Code Cache/js/fixture.bin"),
        ];
        let state = [
            data.join("sessionData/Cookies"),
            data.join("sessionData/IndexedDB/account.bin"),
            data.join("sessionData/Local Storage/account.bin"),
            data.join("settings/settings.json"),
            data.join("themes/custom.css"),
            data.join("vencordFiles/renderer.js"),
        ];
        for path in &caches {
            write_fixture(path, CACHE);
        }
        for path in &state {
            write_fixture(path, STATE);
        }
        let plan = isolated_plan("app.vesktop-cache", sandbox.path());
        execute_fixture(&plan, 0, true);
        for path in &caches {
            assert_eq!(fs::read(path).unwrap(), CACHE);
        }
        execute_fixture(&plan, 0, false);
        for path in &caches {
            assert!(!path.exists());
        }
        for path in &state {
            assert_eq!(fs::read(path).unwrap(), STATE);
        }
        assert_eq!(
            measure_owned_rule(&plan, 0, None, &CleanupExclusions::default())
                .unwrap()
                .bytes,
            0
        );
        for path in &caches {
            write_fixture(path, CACHE);
        }
        assert!(
            measure_owned_rule(&plan, 0, None, &CleanupExclusions::default())
                .unwrap()
                .bytes
                > 0
        );
    }
}

#[test]
fn vesktop_cache_roots_have_exclusive_ownership() {
    let catalog = parse_current_platform_catalog().unwrap();
    for source in &catalog {
        for root in &source.roots {
            if root.template.contains("vesktop/") {
                assert_eq!(source.id, "app.vesktop-cache");
            }
        }
    }
}

fn write_fixture(path: &Path, content: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn isolated_spec(id: &str, sandbox: &Path) -> RuleSpec {
    let mut source = parse_current_platform_catalog()
        .unwrap()
        .into_iter()
        .find(|source| source.id == id)
        .expect("the production catalog must provide the tested rule");
    assert_eq!(
        source.execution.requires_app_close(),
        !source.required_stopped_processes.is_empty()
    );
    let name = sandbox.file_name().unwrap().to_str().unwrap();
    for root in &mut source.roots {
        let relative = root
            .template
            .replace("${application_support}", "home/Library/Application Support")
            .replace("${user_library}", "home/Library")
            .replace("${local_app_data}", "home/AppData/Local")
            .replace("${roaming_app_data}", "home/AppData/Roaming")
            .replace("${home}", "home");
        assert!(!relative.contains("${"), "unexpected template: {relative}");
        fs::create_dir_all(sandbox.join(&relative)).unwrap();
        root.template = format!("${{temp}}/{name}/{relative}");
    }
    // Fixture trees have no application writers. Production process gates
    // remain mandatory and are checked above, not bypassed by an empty list.
    source.required_stopped_processes.clear();
    let mut spec = compile_declarative_source(source).unwrap();
    let logical_sandbox = std::env::temp_dir().join(name);
    let physical_sandbox = fs::canonicalize(sandbox).unwrap();
    for root in &mut spec.roots {
        // Resolve the sandbox's physical spelling without requiring optional
        // cache leaves to exist. Production expansion also retains absent leaves.
        let relative = root.resolved_path.strip_prefix(&logical_sandbox).unwrap();
        root.resolved_path = physical_sandbox.join(relative);
    }
    spec
}

fn isolated_plan(id: &str, sandbox: &Path) -> ScanPlan {
    let rules = compile_rules(vec![isolated_spec(id, sandbox)]).unwrap();
    compile_scan_plan(rules, &[true], &[]).unwrap()
}

fn execute_fixture(plan: &ScanPlan, rule_index: usize, dry_run: bool) -> CleanupActionResult {
    let measured =
        measure_owned_rule(plan, rule_index, None, &CleanupExclusions::default()).unwrap();
    assert_eq!(measured.skipped_count, 0);
    let process_snapshot = ProcessSnapshot::default();
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    let action = execute_rule(
        &plan.rules[rule_index],
        rule_index,
        Some(measured),
        &RuleExecutionContext {
            ownership_plan: plan,
            process_snapshot: &process_snapshot,
            source_scope: None,
            empty_directory_authorizations: None,
            operation: &operation,
            dry_run,
        },
        &mut |_, _| {},
    );
    operation.complete();
    assert_eq!(action.failed_item_count, 0, "{action:?}");
    assert_eq!(
        action.status,
        if dry_run {
            CleanupActionStatus::Previewed
        } else {
            CleanupActionStatus::Completed
        },
        "{action:?}"
    );
    action
}

#[test]
fn documented_cache_selection_and_process_policies_are_explicit() {
    let catalog = parse_current_platform_catalog().unwrap();
    for (id, default_selected) in [
        ("browser.zen-cache", true),
        ("browser.helium-cache", true),
        ("dev.puppeteer-cache", false),
        ("dev.electron-builder-cache", false),
    ] {
        let source = catalog.iter().find(|source| source.id == id).unwrap();
        assert_eq!(source.default_selected, default_selected, "{id}");
        assert!(source.execution.requires_app_close(), "{id}");
        assert!(!source.required_stopped_processes.is_empty(), "{id}");
        if !default_selected {
            assert_eq!(source.recommended_selected, Some(false), "{id}");
        }
    }
    #[cfg(windows)]
    assert_eq!(
        catalog
            .iter()
            .find(|source| source.id == "browser.helium-cache")
            .unwrap()
            .required_stopped_processes,
        ["chrome.exe"]
    );
}

fn check_cleanup(
    plan: &ScanPlan,
    selected: &[std::path::PathBuf],
    preserved: &[std::path::PathBuf],
) {
    let expected_bytes = (selected.len() * CACHE.len()) as u64;
    for dry_run in [true, false] {
        let measured = measure_owned_rule(plan, 0, None, &CleanupExclusions::default()).unwrap();
        assert_eq!(measured.bytes, expected_bytes);
        assert_eq!(measured.skipped_count, 0);
        let action = execute_fixture(plan, 0, dry_run);
        assert_eq!(action.failed_item_count, 0, "{action:?}");
        assert_eq!(action.bytes_expected, expected_bytes);
        assert_eq!(
            action.status,
            if dry_run {
                CleanupActionStatus::Previewed
            } else {
                CleanupActionStatus::Completed
            },
            "{action:?}"
        );
        assert_eq!(
            action.released_bytes,
            if dry_run { 0 } else { expected_bytes }
        );
        for path in selected {
            if dry_run {
                assert_eq!(fs::read(path).unwrap(), CACHE);
            } else {
                assert!(!path.exists(), "{path:?}");
            }
        }
        for path in preserved {
            assert_eq!(fs::read(path).unwrap(), STATE, "{path:?}");
        }
    }
    assert_eq!(
        measure_owned_rule(plan, 0, None, &CleanupExclusions::default())
            .unwrap()
            .bytes,
        0
    );
    // A later cache write remains discoverable without broadening ownership.
    write_fixture(&selected[0], CACHE);
    assert_eq!(
        measure_owned_rule(plan, 0, None, &CleanupExclusions::default())
            .unwrap()
            .bytes,
        CACHE.len() as u64
    );
}

#[cfg(windows)]
#[test]
fn opera_layout_cleanup_preserves_profiles_and_has_no_duplicate_roots() {
    let _lock = crate::shared::operation::test_operation_lock();
    let sandbox = tempfile::tempdir().unwrap();
    let mut selected = Vec::new();
    let mut preserved = Vec::new();
    for base in ["home/AppData/Local", "home/AppData/Roaming"] {
        for browser in ["Opera Stable", "Opera GX Stable"] {
            let root = sandbox
                .path()
                .join(base)
                .join("Opera Software")
                .join(browser);
            for profile in [
                "",
                "Default",
                "Guest Profile",
                "System Profile",
                "Profile 2",
            ] {
                for leaf in [
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
                    let path = root.join(profile).join(leaf).join("nested/entry");
                    write_fixture(&path, CACHE);
                    selected.push(path);
                }
                for state in [
                    "Bookmarks",
                    "Cookies",
                    "Login Data",
                    "History",
                    "Preferences",
                    "Local State",
                    "Sessions/session",
                    "Extensions/extension",
                    "IndexedDB/store",
                    "Local Storage/state",
                    "Service Worker/CacheStorage/offline",
                    "Cache-backup/entry",
                ] {
                    preserved.push(root.join(profile).join(state));
                }
            }
            preserved.push(root.join("Default.old/Cache/entry"));
            preserved.push(root.join("unrecognized-profile/Cache/entry"));
            preserved.push(root.join("Profile/Cache/entry"));
        }
    }
    for path in &preserved {
        write_fixture(path, STATE);
    }
    let plan = isolated_plan("browser.opera-cache", sandbox.path());
    let roots = &plan.rules[0].roots;
    assert_eq!(roots.len(), selected.len());
    assert_eq!(
        roots
            .iter()
            .map(|root| root.as_path())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        roots.len(),
        "legacy and migrated layouts must not count any cache twice"
    );
    check_cleanup(&plan, &selected, &preserved);
}

#[cfg(target_os = "macos")]
#[test]
fn opera_legacy_roots_remain_discoverable_without_listing_the_vendor_directory() {
    use std::os::unix::fs::PermissionsExt;

    struct RestorePermissions(std::path::PathBuf, fs::Permissions);
    impl Drop for RestorePermissions {
        fn drop(&mut self) {
            fs::set_permissions(&self.0, self.1.clone()).unwrap();
        }
    }

    let sandbox = tempfile::tempdir().unwrap();
    let vendor = sandbox.path().join("roaming/Opera Software");
    let cache = vendor.join("Opera Stable/Cache");
    write_fixture(&cache.join("entry"), CACHE);
    let original = fs::metadata(&vendor).unwrap().permissions();
    let _restore = RestorePermissions(vendor.clone(), original);
    fs::set_permissions(&vendor, fs::Permissions::from_mode(0o111)).unwrap();
    assert!(
        fs::read_dir(&vendor).is_err(),
        "the fixture must deny directory enumeration"
    );
    assert_eq!(fs::read(cache.join("entry")).unwrap(), CACHE);

    // Exercise the embedded Windows rule's root resolver without changing the
    // account's real AppData or relying on ACL-bypassing Windows SSH tokens.
    let rule = super::parse_catalog(super::EMBEDDED_DECLARATIVE_RULE_SOURCES)
        .unwrap()
        .into_iter()
        .find(|parsed| {
            parsed.rule.id == "browser.opera-cache"
                && parsed.rule.platform == super::SourcePlatform::Windows
        })
        .unwrap()
        .rule;
    let name = sandbox.path().file_name().unwrap().to_str().unwrap();
    let mut discovered = Vec::new();
    for mut root in rule.roots {
        root.template = root
            .template
            .replace("${roaming_app_data}", &format!("${{temp}}/{name}/roaming"))
            .replace("${local_app_data}", &format!("${{temp}}/{name}/local"));
        if let Ok(roots) = super::resolve_root_source(&root, &mut Default::default()) {
            discovered.extend(roots);
        }
    }
    let expected = std::env::temp_dir()
        .join(name)
        .join("roaming/Opera Software/Opera Stable/Cache");
    assert!(
        discovered.iter().any(|root| root.resolved_path == expected),
        "a readable legacy cache must not require listing its vendor parent"
    );
}

#[cfg(windows)]
#[test]
fn firefox_roaming_shader_cleanup_preserves_all_other_roaming_data() {
    let _lock = crate::shared::operation::test_operation_lock();
    let sandbox = tempfile::tempdir().unwrap();
    let mut selected = Vec::new();
    let mut preserved = Vec::new();
    for profile in ["random.default-release", "second.profile"] {
        for base in ["home/AppData/Local", "home/AppData/Roaming"] {
            let root = sandbox
                .path()
                .join(base)
                .join("Mozilla/Firefox/Profiles")
                .join(profile);
            for leaf in ["cache2", "startupCache", "shader-cache"] {
                let path = root.join(leaf).join("nested/entry");
                if base.ends_with("Local") || leaf == "shader-cache" {
                    write_fixture(&path, CACHE);
                    selected.push(path);
                } else {
                    preserved.push(path);
                }
            }
            for state in [
                "cookies.sqlite",
                "places.sqlite",
                "key4.db",
                "logins.json",
                "prefs.js",
                "sessionstore-backups/session.jsonlz4",
                "storage/default/site",
                "shader-cache-backup/entry",
            ] {
                preserved.push(root.join(state));
            }
        }
    }
    for path in &preserved {
        write_fixture(path, STATE);
    }
    let plan = isolated_plan("browser.firefox-cache", sandbox.path());
    assert_eq!(plan.rules[0].roots.len(), selected.len());
    check_cleanup(&plan, &selected, &preserved);
}

#[test]
fn zen_cache_cleanup_preserves_sessions_and_adjacent_profile_data() {
    let _lock = crate::shared::operation::test_operation_lock();
    let sandbox = tempfile::tempdir().unwrap();
    #[cfg(target_os = "macos")]
    let (cache_relative, persistent_relative) = (
        "home/Library/Caches/zen/Profiles",
        "home/Library/Application Support/zen/Profiles/default/cache2/user-file",
    );
    #[cfg(windows)]
    let (cache_relative, persistent_relative) = (
        "home/AppData/Local/zen/Profiles",
        "home/AppData/Roaming/zen/Profiles/default/cache2/user-file",
    );
    let cache = sandbox.path().join(cache_relative);
    let mut selected = Vec::new();
    let mut preserved = Vec::new();
    for profile in ["random.default-release", "second.profile"] {
        for leaf in ["cache2", "startupCache"] {
            let path = cache.join(profile).join(leaf).join("nested/entry");
            write_fixture(&path, CACHE);
            selected.push(path);
        }
        for leaf in [
            "cookies.sqlite",
            "prefs.js",
            "cache2-backup/entry",
            "zen-sessions.jsonlz4",
        ] {
            preserved.push(cache.join(profile).join(leaf));
        }
    }
    preserved.push(sandbox.path().join(persistent_relative));
    for path in &preserved {
        write_fixture(path, STATE);
    }
    let plan = isolated_plan("browser.zen-cache", sandbox.path());
    assert_eq!(plan.rules[0].roots.len(), selected.len());
    check_cleanup(&plan, &selected, &preserved);
}

#[test]
fn chromium_gpu_cache_cleanup_preserves_browser_profiles_and_site_storage() {
    let _lock = crate::shared::operation::test_operation_lock();
    #[cfg(target_os = "macos")]
    let (parent, browsers) = (
        "home/Library/Application Support",
        [
            ("browser.helium-cache", "net.imput.helium"),
            ("browser.chrome-cache", "Google/Chrome"),
            ("browser.brave-cache", "BraveSoftware/Brave-Browser"),
            ("browser.edge-cache", "Microsoft Edge"),
        ],
    );
    #[cfg(windows)]
    let (parent, browsers) = (
        "home/AppData/Local",
        [
            ("browser.helium-cache", "imput/Helium/User Data"),
            ("browser.chrome-cache", "Google/Chrome/User Data"),
            (
                "browser.brave-cache",
                "BraveSoftware/Brave-Browser/User Data",
            ),
            ("browser.edge-cache", "Microsoft/Edge/User Data"),
        ],
    );
    for (id, relative) in browsers {
        let sandbox = tempfile::tempdir().unwrap();
        let support = sandbox.path().join(parent).join(relative);
        let mut selected = Vec::new();
        let mut preserved = Vec::new();
        #[cfg(target_os = "macos")]
        {
            let network = sandbox
                .path()
                .join("home/Library/Caches")
                .join(relative)
                .join("Default/Cache/data");
            write_fixture(&network, CACHE);
            selected.push(network);
        }
        for leaf in [
            "ShaderCache",
            "GrShaderCache",
            "GraphiteDawnCache",
            "GPUPersistentCache",
        ] {
            let path = support.join(leaf).join("data");
            write_fixture(&path, CACHE);
            selected.push(path);
            preserved.push(support.join(format!("{leaf}Backup/data")));
        }
        for profile in ["Default", "Profile 2", "Guest Profile", "System Profile"] {
            for leaf in [
                "Cache",
                "Code Cache",
                "GPUCache",
                "DawnCache",
                "GrShaderCache",
                "DawnWebGPUCache",
                "DawnGraphiteCache",
            ] {
                let path = support.join(profile).join(leaf).join("entry");
                write_fixture(&path, CACHE);
                selected.push(path);
            }
            for leaf in [
                "Cookies",
                "History",
                "Login Data",
                "Bookmarks",
                "Preferences",
                "Extensions/id/data",
                "IndexedDB/data",
                "Local Storage/data",
                "Service Worker/CacheStorage/data",
                "Storage/ext/id/GPUCache/data",
                "DawnGraphiteCacheBackup/data",
            ] {
                preserved.push(support.join(profile).join(leaf));
            }
        }
        preserved.push(support.join("Local State"));
        preserved.push(support.join("Unknown Profile/DawnGraphiteCache/data"));
        for path in &preserved {
            write_fixture(path, STATE);
        }
        let plan = isolated_plan(id, sandbox.path());
        check_cleanup(&plan, &selected, &preserved);
    }
}

#[test]
fn puppeteer_cleanup_only_selects_documented_browser_stores() {
    let _lock = crate::shared::operation::test_operation_lock();
    let sandbox = tempfile::tempdir().unwrap();
    let root = sandbox.path().join("home/.cache/puppeteer");
    let selected: Vec<_> = ["chrome", "chrome-headless-shell", "firefox"]
        .map(|browser| root.join(browser).join("platform-version/runtime"))
        .into();
    let preserved = [
        root.join(".puppeteerrc.cjs"),
        root.join("custom-browser/runtime"),
        root.join("chrome-backup/runtime"),
        sandbox.path().join("home/project/puppeteer.config.js"),
    ];
    for path in &selected {
        write_fixture(path, CACHE);
    }
    for path in &preserved {
        write_fixture(path, STATE);
    }
    check_cleanup(
        &isolated_plan("dev.puppeteer-cache", sandbox.path()),
        &selected,
        &preserved,
    );
}

#[test]
fn electron_builder_cleanup_preserves_projects_credentials_and_sibling_caches() {
    let _lock = crate::shared::operation::test_operation_lock();
    let sandbox = tempfile::tempdir().unwrap();
    #[cfg(target_os = "macos")]
    let (cache_relative, sibling_relative) = (
        "home/Library/Caches/electron-builder",
        "home/Library/Caches/electron/archive.zip",
    );
    #[cfg(windows)]
    let (cache_relative, sibling_relative) = (
        "home/AppData/Local/electron-builder/Cache",
        "home/AppData/Local/electron/Cache/archive.zip",
    );
    let root = sandbox.path().join(cache_relative);
    let selected = [
        root.join("7zip@1.0.1/archive.tar.gz"),
        root.join("toolset/extracted/tool"),
    ];
    let preserved = [
        sandbox.path().join("home/project/electron-builder.yml"),
        sandbox.path().join("home/project/dist/app.dmg"),
        sandbox
            .path()
            .join("home/Library/Keychains/signing.keychain-db"),
        sandbox.path().join(sibling_relative),
        sandbox.path().join("home/custom-builder-cache/archive.zip"),
    ];
    for path in &selected {
        write_fixture(path, CACHE);
    }
    for path in &preserved {
        write_fixture(path, STATE);
    }
    check_cleanup(
        &isolated_plan("dev.electron-builder-cache", sandbox.path()),
        &selected,
        &preserved,
    );
}

#[cfg(windows)]
#[test]
fn puppeteer_has_unique_ownership_outside_the_generic_developer_rule() {
    let _lock = crate::shared::operation::test_operation_lock();
    let sandbox = tempfile::tempdir().unwrap();
    let root = sandbox.path().join("home/.cache/puppeteer");
    let browsers = ["chrome", "chrome-headless-shell", "firefox"]
        .map(|name| root.join(name).join("win64-version/runtime.exe"));
    let generic_cache = sandbox.path().join("home/.cache/ruff/cache");
    for path in browsers.iter().chain([&generic_cache]) {
        write_fixture(path, CACHE);
    }
    let preserved = [
        root.join(".puppeteerrc.cjs"),
        root.join("custom-runtime/data"),
    ];
    for path in &preserved {
        write_fixture(path, STATE);
    }

    let generic_source = parse_current_platform_catalog()
        .unwrap()
        .into_iter()
        .find(|source| source.id == "dev.user-tool-cache")
        .unwrap();
    assert!(generic_source
        .roots
        .iter()
        .all(|root| !root.template.contains("puppeteer")));
    let rules = compile_rules(vec![
        isolated_spec("dev.user-tool-cache", sandbox.path()),
        isolated_spec("dev.puppeteer-cache", sandbox.path()),
    ])
    .unwrap();
    let plan = compile_scan_plan(rules, &[true, true], &[]).unwrap();
    let generic = plan
        .rules
        .iter()
        .position(|rule| rule.id == "dev.user-tool-cache")
        .unwrap();
    let puppeteer = plan
        .rules
        .iter()
        .position(|rule| rule.id == "dev.puppeteer-cache")
        .unwrap();
    let bytes = |index| {
        measure_owned_rule(&plan, index, None, &CleanupExclusions::default())
            .unwrap()
            .bytes
    };
    assert_eq!(bytes(generic), CACHE.len() as u64);
    assert_eq!(bytes(puppeteer), (browsers.len() * CACHE.len()) as u64);
    assert_eq!(bytes(generic) + bytes(puppeteer), (4 * CACHE.len()) as u64);
    for path in &browsers {
        let physical = fs::canonicalize(path).unwrap();
        let metadata = fs::metadata(&physical).unwrap();
        assert!(plan.rule_owns_path(puppeteer, &physical, &metadata));
        assert!(!plan.rule_owns_path(generic, &physical, &metadata));
    }
    assert_eq!(
        execute_fixture(&plan, generic, true).bytes_expected,
        CACHE.len() as u64
    );
    assert!(generic_cache.exists());
    assert_eq!(
        execute_fixture(&plan, generic, false).released_bytes,
        CACHE.len() as u64
    );
    assert!(!generic_cache.exists());
    for path in &browsers {
        assert_eq!(fs::read(path).unwrap(), CACHE);
    }
    assert_eq!(
        execute_fixture(&plan, puppeteer, false).released_bytes,
        (browsers.len() * CACHE.len()) as u64
    );
    assert!(browsers.iter().all(|path| !path.exists()));
    assert_eq!(bytes(generic) + bytes(puppeteer), 0);
    for path in &preserved {
        assert_eq!(fs::read(path).unwrap(), STATE);
    }
}
