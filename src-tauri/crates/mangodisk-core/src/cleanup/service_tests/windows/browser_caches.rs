    /// Confirms that the Store-package host process blocks WhatsApp cache
    /// cleanup before MangoDisk traverses any WebView2 profile directory.
    #[test]
    #[ignore = "requires the real WhatsApp application to be running"]
    fn real_whatsapp_rendering_cache_blocks_while_running() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WHATSAPP_CACHE_BLOCK").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WHATSAPP_CACHE_BLOCK=1 for the real process-gate diagnostic"
        );
        let process_snapshot =
            ProcessSnapshot::capture().expect("the Windows process inventory must be available");
        assert!(
            !process_snapshot
                .matching_processes(&["WhatsApp.Root.exe".to_string()])
                .is_empty(),
            "the real WhatsApp application must be running"
        );

        let result = CleanupService::execute(CleanupRequest {
            rule_ids: vec!["app.whatsapp-rendering-cache".to_string()],
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        })
        .expect("the blocked cleanup must return a structured result");
        assert_eq!(result.actions.len(), 1);
        let action = &result.actions[0];
        assert_eq!(action.status, crate::cleanup::CleanupActionStatus::Blocked);
        assert_eq!(
            action.reason_code,
            Some(crate::cleanup::CleanupActionReason::RunningProcesses)
        );
        assert_eq!(action.released_bytes, 0);
        assert_eq!(action.affected_item_count, 0);
        assert!(!action.running_processes.is_empty());
        println!("real_windows_whatsapp_cache_block owner_count=1");
    }
    /// Clears only fixed WebView2 cache leaves in the isolated VM. Full-tree
    /// hashes prove that package state, product extensions, preferences,
    /// browser storage, network data, and Service Worker scripts remain exact.
    #[test]
    #[ignore = "clears real WhatsApp caches in an isolated Windows VM"]
    fn real_whatsapp_rendering_cache_preserves_durable_state() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WHATSAPP_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WHATSAPP_CACHE=1 to authorize this real cache diagnostic"
        );
        let process_snapshot =
            ProcessSnapshot::capture().expect("the Windows process inventory must be available");
        assert!(
            process_snapshot
                .matching_processes(&["WhatsApp.Root.exe".to_string(), "WhatsApp.exe".to_string(),])
                .is_empty(),
            "WhatsApp must be stopped before cleanup"
        );

        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .expect("LOCALAPPDATA must be available");
        let package = local.join("Packages/5319275A.WhatsAppDesktop_cv1g1gvanyjgm");
        let local_cache = package.join("LocalCache");
        let webview = local_cache.join("EBWebView");
        assert!(package.is_dir() && webview.is_dir());

        let mut preserved_paths = Vec::new();
        for relative in [
            "LocalState",
            "Settings",
            "RoamingState",
            "LocalCache/ChromeCodeVerifyExtension",
            "LocalCache/ZoomExtension",
        ] {
            let path = package.join(relative);
            if path.exists() {
                preserved_paths.push(path);
            }
        }
        for relative in [
            "Local State",
            "Last Version",
            "Default/Preferences",
            "Default/Secure Preferences",
            "Default/History",
            "Default/Login Data",
            "Default/Login Data For Account",
            "Default/Local Storage",
            "Default/IndexedDB",
            "Default/Network",
            "Default/Session Storage",
            "Default/WebStorage",
            "Default/Extension Cookies",
            "Default/Web Data",
            "Default/Service Worker/Database",
            "Default/Service Worker/ScriptCache",
        ] {
            let path = webview.join(relative);
            if path.exists() {
                preserved_paths.push(path);
            }
        }
        assert!(
            preserved_paths.len() >= 12,
            "the initialized application must expose durable package and browser state"
        );
        let preserved_before = preserved_paths
            .iter()
            .map(|path| digest_tree(path))
            .collect::<Vec<_>>();

        let mut target_roots = Vec::new();
        for relative in [
            "component_crx_cache",
            "extensions_crx_cache",
            "GPUPersistentCache",
            "GraphiteDawnCache",
            "GrShaderCache",
            "ShaderCache",
            "Crashpad/reports",
            "Default/Cache",
            "Default/Code Cache",
            "Default/GPUCache",
            "Default/DawnCache",
            "Default/DawnGraphiteCache",
            "Default/DawnWebGPUCache",
            "Default/GrShaderCache",
            "Default/GraphiteDawnCache",
            "Default/Shared Dictionary/cache",
            "Default/Service Worker/CacheStorage",
        ] {
            let path = webview.join(relative);
            if path.is_dir() {
                target_roots.push(path);
            }
        }
        assert!(
            target_roots.len() >= 13,
            "WhatsApp must expose the verified WebView2 cache leaves"
        );

        let markers = target_roots
            .iter()
            .map(|root| root.join("mangodisk-rule-validation.bin"))
            .collect::<Vec<_>>();
        for marker in &markers {
            fs::write(marker, b"payload").expect("the cache marker must be created");
        }
        let request = |dry_run| CleanupRequest {
            rule_ids: vec!["app.whatsapp-rendering-cache".to_string()],
            source_selections: Vec::new(),
            dry_run,
            project_roots: Vec::new(),
        };

        let preview =
            CleanupService::execute(request(true)).expect("the real cache dry run must succeed");
        assert_eq!(preview.actions.len(), 1);
        assert_eq!(preview.failed_item_count, 0, "{:?}", preview.actions);
        assert!(preview.expected_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| marker.exists()));

        let result =
            CleanupService::execute(request(false)).expect("the real cache cleanup must succeed");
        assert_eq!(result.actions.len(), 1);
        assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
        assert!(result.released_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| !marker.exists()));
        let preserved_after = preserved_paths
            .iter()
            .map(|path| digest_tree(path))
            .collect::<Vec<_>>();
        assert_eq!(preserved_after, preserved_before);
        println!(
            "real_windows_whatsapp_cache_cleanup expected_bytes={} released_bytes={} affected_item_count={} target_root_count={} preserved_root_count={}",
            preview.expected_bytes,
            result.released_bytes,
            result.affected_item_count,
            target_roots.len(),
            preserved_paths.len()
        );
    }

    /// Confirms that the Store-package host process blocks Codex cache cleanup
    /// before MangoDisk traverses logs or any WebView2 profile directory.
    #[test]
    #[ignore = "requires the real Codex application to be running"]
    fn real_codex_windows_cache_blocks_while_running() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_CODEX_WINDOWS_CACHE_BLOCK").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_CODEX_WINDOWS_CACHE_BLOCK=1 for the real process-gate diagnostic"
        );
        let process_snapshot =
            ProcessSnapshot::capture().expect("the Windows process inventory must be available");
        assert!(
            !process_snapshot
                .matching_processes(&["ChatGPT.exe".to_string()])
                .is_empty(),
            "the real Codex application must be running"
        );

        let result = CleanupService::execute(CleanupRequest {
            rule_ids: vec!["app.chatgpt-cache".to_string()],
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        })
        .expect("the blocked cleanup must return a structured result");
        assert_eq!(result.actions.len(), 1);
        let action = &result.actions[0];
        assert_eq!(action.status, crate::cleanup::CleanupActionStatus::Blocked);
        assert_eq!(
            action.reason_code,
            Some(crate::cleanup::CleanupActionReason::RunningProcesses)
        );
        assert_eq!(action.released_bytes, 0);
        assert_eq!(action.affected_item_count, 0);
        assert!(!action.running_processes.is_empty());
        println!("real_windows_codex_cache_block owner_count=1");
    }

    /// Clears only fixed WebView2 cache leaves and diagnostic logs in the
    /// isolated VM. Full-tree hashes cover durable browser state and product
    /// components, while explicit file hashes protect the user-owned .codex
    /// credentials and configuration without traversing that large tree.
    #[test]
    #[ignore = "clears real Codex caches in an isolated Windows VM"]
    fn real_codex_windows_cache_preserves_projects_and_browser_state() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_CODEX_WINDOWS_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_CODEX_WINDOWS_CACHE=1 to authorize this real cache diagnostic"
        );
        let process_snapshot =
            ProcessSnapshot::capture().expect("the Windows process inventory must be available");
        assert!(
            process_snapshot
                .matching_processes(&["ChatGPT.exe".to_string()])
                .is_empty(),
            "Codex must be stopped before cleanup"
        );

        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .expect("LOCALAPPDATA must be available");
        let user_profile = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .expect("USERPROFILE must be available");
        let package = local.join("Packages/OpenAI.Codex_2p2nqsd0c76g0");
        let webview = package.join("LocalCache/Roaming/Codex/web/Codex");
        let logs = package.join("LocalCache/Local/Codex/Logs");
        assert!(package.is_dir() && webview.is_dir() && logs.is_dir());

        let mut preserved_paths = Vec::new();
        for relative in [
            "Settings",
            "LocalCache/Roaming/Codex/web/Codex/WasmTtsEngine",
            "LocalCache/Roaming/Codex/web/Codex/WidevineCdm",
            "LocalCache/Roaming/Codex/web/Codex/CertificateRevocation",
            "LocalCache/Roaming/Codex/web/Codex/ActorSafetyLists",
            "LocalCache/Roaming/Codex/web/Codex/ZxcvbnData",
        ] {
            let path = package.join(relative);
            if path.exists() {
                preserved_paths.push(path);
            }
        }
        for relative in [
            "Local State",
            "Last Version",
            "owl-feature-bootstrap-cache.json",
            "browser-sidebar-page-states.json",
            "Default/Preferences",
            "Default/Secure Preferences",
            "Default/History",
            "Default/Login Data",
            "Default/Login Data For Account",
            "Default/Local Storage",
            "Default/IndexedDB",
            "Default/Network",
            "Default/Session Storage",
            "Default/WebStorage",
            "Default/Extension Cookies",
            "Default/Web Data",
            "Default/Service Worker/Database",
            "Default/Service Worker/ScriptCache",
        ] {
            let path = webview.join(relative);
            if path.exists() {
                preserved_paths.push(path);
            }
        }

        let partitions = webview.join("Default/Partitions");
        let mut partition_count = 0_usize;
        if partitions.is_dir() {
            let mut children = fs::read_dir(&partitions)
                .expect("the Codex partition root must be readable")
                .map(|entry| entry.expect("the partition entry must be readable").path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            children.sort();
            partition_count = children.len();
            for partition in children {
                for relative in [
                    "Preferences",
                    "Secure Preferences",
                    "History",
                    "Login Data",
                    "Login Data For Account",
                    "Local Storage",
                    "IndexedDB",
                    "Network",
                    "Session Storage",
                    "WebStorage",
                    "Extension Cookies",
                    "Web Data",
                    "Service Worker/Database",
                    "Service Worker/ScriptCache",
                ] {
                    let path = partition.join(relative);
                    if path.exists() {
                        preserved_paths.push(path);
                    }
                }
            }
        }
        for relative in [".codex/auth.json", ".codex/config.toml"] {
            let path = user_profile.join(relative);
            if path.exists() {
                preserved_paths.push(path);
            }
        }
        preserved_paths.sort();
        preserved_paths.dedup();
        assert!(
            preserved_paths.len() >= 25,
            "the initialized app must expose durable package, browser, component, and .codex state"
        );
        let preserved_before = preserved_paths
            .iter()
            .map(|path| digest_tree(path))
            .collect::<Vec<_>>();

        let mut target_roots = Vec::new();
        if logs.is_dir() {
            target_roots.push(logs);
        }
        for relative in [
            "component_crx_cache",
            "extensions_crx_cache",
            "GPUPersistentCache",
            "GraphiteDawnCache",
            "GrShaderCache",
            "ShaderCache",
            "Crashpad/reports",
            "Default/Cache",
            "Default/Code Cache",
            "Default/GPUCache",
            "Default/DawnCache",
            "Default/DawnGraphiteCache",
            "Default/DawnWebGPUCache",
            "Default/GrShaderCache",
            "Default/GraphiteDawnCache",
            "Default/Shared Dictionary/cache",
            "Default/Service Worker/CacheStorage",
        ] {
            let path = webview.join(relative);
            if path.is_dir() {
                target_roots.push(path);
            }
        }
        if partitions.is_dir() {
            let mut children = fs::read_dir(&partitions)
                .expect("the Codex partition root must be readable")
                .map(|entry| entry.expect("the partition entry must be readable").path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            children.sort();
            for partition in children {
                for relative in [
                    "Cache",
                    "Code Cache",
                    "GPUCache",
                    "DawnCache",
                    "DawnGraphiteCache",
                    "DawnWebGPUCache",
                    "GrShaderCache",
                    "GraphiteDawnCache",
                    "Shared Dictionary/cache",
                    "Service Worker/CacheStorage",
                ] {
                    let path = partition.join(relative);
                    if path.is_dir() {
                        target_roots.push(path);
                    }
                }
            }
        }
        target_roots.sort();
        target_roots.dedup();
        assert!(
            target_roots.len() >= 19,
            "Codex must expose the verified log, WebView2, and partition cache leaves"
        );

        let markers = target_roots
            .iter()
            .map(|root| root.join("mangodisk-rule-validation.bin"))
            .collect::<Vec<_>>();
        for marker in &markers {
            fs::write(marker, b"payload").expect("the cache marker must be created");
        }
        let request = |dry_run| CleanupRequest {
            rule_ids: vec!["app.chatgpt-cache".to_string()],
            source_selections: Vec::new(),
            dry_run,
            project_roots: Vec::new(),
        };

        let preview =
            CleanupService::execute(request(true)).expect("the real cache dry run must succeed");
        assert_eq!(preview.actions.len(), 1);
        assert_eq!(preview.failed_item_count, 0, "{:?}", preview.actions);
        assert!(preview.expected_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| marker.exists()));

        let result =
            CleanupService::execute(request(false)).expect("the real cache cleanup must succeed");
        assert_eq!(result.actions.len(), 1);
        assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
        assert!(result.released_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| !marker.exists()));
        let preserved_after = preserved_paths
            .iter()
            .map(|path| digest_tree(path))
            .collect::<Vec<_>>();
        assert_eq!(preserved_after, preserved_before);
        println!(
            "real_windows_codex_cache_cleanup expected_bytes={} released_bytes={} affected_item_count={} partition_count={} target_root_count={} preserved_root_count={}",
            preview.expected_bytes,
            result.released_bytes,
            result.affected_item_count,
            partition_count,
            target_roots.len(),
            preserved_paths.len()
        );
    }

    /// Confirms that UC's browser and proxy processes block cleanup before
    /// the real Chromium profile or downloaded component cache is traversed.
    #[test]
    #[ignore = "requires the real UC browser to be running"]
    fn real_windows_uc_browser_cache_blocks_while_running() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WINDOWS_UC_CACHE_BLOCK").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WINDOWS_UC_CACHE_BLOCK=1 for the real process-gate diagnostic"
        );
        let process_names = ["uc.exe", "uc_proxy.exe"].map(str::to_string);
        let running = ProcessSnapshot::capture()
            .expect("the Windows process inventory must be available")
            .matching_processes(&process_names);
        assert!(!running.is_empty(), "UC browser must be running");

        let result = CleanupService::execute(CleanupRequest {
            rule_ids: vec!["browser.uc-cache".to_string()],
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        })
        .expect("the blocked UC cleanup must return a structured result");
        assert_eq!(result.actions.len(), 1);
        let action = &result.actions[0];
        assert_eq!(action.status, crate::cleanup::CleanupActionStatus::Blocked);
        assert_eq!(
            action.reason_code,
            Some(crate::cleanup::CleanupActionReason::RunningProcesses)
        );
        assert_eq!(action.released_bytes, 0);
        assert_eq!(action.affected_item_count, 0);
        assert!(!action.running_processes.is_empty());
        println!(
            "real_windows_uc_cache_block running_process_count={}",
            running.len()
        );
    }

    /// Clears UC's fixed response, code, GPU, shader, and downloaded component
    /// caches. Full digests protect representative credentials, cookies,
    /// history, bookmarks, extensions, sessions, local storage, Service Worker
    /// state, and browser settings around the production cleanup.
    #[test]
    #[ignore = "clears real UC browser caches in an isolated Windows VM"]
    fn real_windows_uc_browser_cache_preserves_profile_state() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WINDOWS_UC_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WINDOWS_UC_CACHE=1 only in an isolated Windows VM"
        );
        let process_names = ["uc.exe", "uc_proxy.exe"].map(str::to_string);
        let running = ProcessSnapshot::capture()
            .expect("the Windows process inventory must be available")
            .matching_processes(&process_names);
        assert!(running.is_empty(), "UC browser must be completely stopped");

        let local_app_data = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .expect("LOCALAPPDATA must be available");
        let user_data = local_app_data.join("UC/User Data");
        let profile = user_data.join("Default");
        assert!(profile.is_dir(), "UC must complete a first launch");

        let preserved_candidates = [
            user_data.join("Local State"),
            user_data.join("NativeMessagingHosts"),
            user_data.join("user_info"),
            profile.join("Bookmarks"),
            profile.join("History"),
            profile.join("Login Data"),
            profile.join("Network"),
            profile.join("Extensions"),
            profile.join("Local Storage"),
            profile.join("Preferences"),
            profile.join("Service Worker"),
            profile.join("Session Storage"),
            profile.join("Sessions"),
            profile.join("WebStorage"),
        ];
        let preserved_paths = preserved_candidates
            .into_iter()
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        assert!(
            preserved_paths.len() >= 11,
            "the initialized UC profile must expose representative durable state"
        );
        let preserved_before = preserved_paths
            .iter()
            .map(|path| digest_tree(path))
            .collect::<Vec<_>>();

        let target_candidates = [
            user_data.join("ShaderCache"),
            user_data.join("GrShaderCache"),
            user_data.join("GraphiteDawnCache"),
            user_data.join("component_crx_cache"),
            profile.join("Cache"),
            profile.join("Code Cache"),
            profile.join("GPUCache"),
            profile.join("DawnGraphiteCache"),
            profile.join("DawnWebGPUCache"),
        ];
        let expected_target_count = target_candidates.len();
        let target_roots = target_candidates
            .into_iter()
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        assert_eq!(
            target_roots.len(),
            expected_target_count,
            "the initialized UC profile must expose every verified cache root"
        );
        let markers = target_roots
            .iter()
            .map(|root| root.join("mangodisk-rule-validation.bin"))
            .collect::<Vec<_>>();
        for marker in &markers {
            fs::write(marker, b"payload").expect("the UC cache marker must be created");
        }
        let request = |dry_run| CleanupRequest {
            rule_ids: vec!["browser.uc-cache".to_string()],
            source_selections: Vec::new(),
            dry_run,
            project_roots: Vec::new(),
        };

        let preview =
            CleanupService::execute(request(true)).expect("the real UC cache dry run must succeed");
        assert_eq!(preview.failed_item_count, 0, "{:?}", preview.actions);
        assert!(preview.expected_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| marker.exists()));

        let result = CleanupService::execute(request(false))
            .expect("the real UC cache cleanup must succeed");
        assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
        assert!(result.released_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| !marker.exists()));
        let preserved_after = preserved_paths
            .iter()
            .map(|path| digest_tree(path))
            .collect::<Vec<_>>();
        assert_eq!(preserved_after, preserved_before);
        println!(
            "real_windows_uc_cache_cleanup expected_bytes={} released_bytes={} affected_item_count={} target_root_count={} preserved_root_count={}",
            preview.expected_bytes,
            result.released_bytes,
            result.affected_item_count,
            target_roots.len(),
            preserved_paths.len()
        );
    }

    fn assert_windows_browser_cache_blocked(
        rule_id: &str,
        process_names: &[&str],
        browser_name: &str,
    ) {
        let process_names = process_names
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>();
        let running = ProcessSnapshot::capture()
            .expect("the Windows process inventory must be available")
            .matching_processes(&process_names);
        assert!(!running.is_empty(), "{browser_name} must be running");

        let result = CleanupService::execute(CleanupRequest {
            rule_ids: vec![rule_id.to_string()],
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        })
        .expect("the blocked browser cleanup must return a structured result");
        assert_eq!(result.actions.len(), 1);
        let action = &result.actions[0];
        assert_eq!(action.status, crate::cleanup::CleanupActionStatus::Blocked);
        assert_eq!(
            action.reason_code,
            Some(crate::cleanup::CleanupActionReason::RunningProcesses)
        );
        assert_eq!(action.released_bytes, 0);
        assert_eq!(action.affected_item_count, 0);
        assert!(!action.running_processes.is_empty());
        println!(
            "real_windows_browser_cache_block browser={} running_process_count={}",
            browser_name,
            running.len()
        );
    }

    fn validate_real_windows_browser_cache_cleanup(
        rule_id: &str,
        process_names: &[&str],
        profile_environment_variable: &str,
        user_data_relative: &str,
        preserved_relatives: &[&str],
        target_relatives: &[&str],
        minimum_preserved_count: usize,
    ) {
        let process_names = process_names
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>();
        let running = ProcessSnapshot::capture()
            .expect("the Windows process inventory must be available")
            .matching_processes(&process_names);
        assert!(running.is_empty(), "{rule_id} must be completely stopped");

        let profile_base = std::env::var_os(profile_environment_variable)
            .map(PathBuf::from)
            .unwrap_or_else(|| panic!("{profile_environment_variable} must be available"));
        let user_data = profile_base.join(user_data_relative);
        assert!(
            user_data.join("Default").is_dir(),
            "{rule_id} must complete a first launch"
        );

        let preserved_paths = preserved_relatives
            .iter()
            .map(|relative| user_data.join(relative))
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        assert!(
            preserved_paths.len() >= minimum_preserved_count,
            "{rule_id} must expose representative durable profile state"
        );
        let preserved_before = preserved_paths
            .iter()
            .map(|path| digest_tree(path))
            .collect::<Vec<_>>();

        let target_roots = target_relatives
            .iter()
            .map(|relative| user_data.join(relative))
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        assert_eq!(
            target_roots.len(),
            target_relatives.len(),
            "{rule_id} must expose every verified cache root"
        );
        let markers = target_roots
            .iter()
            .map(|root| root.join("mangodisk-rule-validation.bin"))
            .collect::<Vec<_>>();
        for marker in &markers {
            fs::write(marker, b"payload").expect("the browser cache marker must be created");
        }
        let request = |dry_run| CleanupRequest {
            rule_ids: vec![rule_id.to_string()],
            source_selections: Vec::new(),
            dry_run,
            project_roots: Vec::new(),
        };

        let preview = CleanupService::execute(request(true))
            .expect("the real browser cache dry run must succeed");
        assert_eq!(preview.failed_item_count, 0, "{:?}", preview.actions);
        assert!(preview.expected_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| marker.exists()));

        let result = CleanupService::execute(request(false))
            .expect("the real browser cache cleanup must succeed");
        assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
        assert!(result.released_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| !marker.exists()));
        let preserved_after = preserved_paths
            .iter()
            .map(|path| digest_tree(path))
            .collect::<Vec<_>>();
        assert_eq!(preserved_after, preserved_before);
        println!(
            "real_windows_browser_cache_cleanup browser={} expected_bytes={} released_bytes={} affected_item_count={} target_root_count={} preserved_root_count={}",
            rule_id,
            preview.expected_bytes,
            result.released_bytes,
            result.affected_item_count,
            target_roots.len(),
            preserved_paths.len()
        );
    }

    /// Confirms that 360 Extreme Browser X blocks cleanup while any of its
    /// renderer processes still owns the real Chromium profile.
    #[test]
    #[ignore = "requires 360 Extreme Browser X to be running in the Windows VM"]
    fn real_windows_360_speed_browser_cache_blocks_while_running() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WINDOWS_360_CACHE_BLOCK").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WINDOWS_360_CACHE_BLOCK=1 for the real process-gate diagnostic"
        );
        assert_windows_browser_cache_blocked(
            "browser.360-speed-cache",
            &["360ChromeX.exe"],
            "360-speed",
        );
    }

    /// Clears only 360 Extreme Browser X response, code, GPU, Dawn, and shader
    /// caches while hashing durable profile state around the production action.
    #[test]
    #[ignore = "clears real 360 Extreme Browser X caches in the Windows VM"]
    fn real_windows_360_speed_browser_cache_preserves_profile_state() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WINDOWS_360_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WINDOWS_360_CACHE=1 only in the isolated Windows VM"
        );
        validate_real_windows_browser_cache_cleanup(
            "browser.360-speed-cache",
            &["360ChromeX.exe"],
            "LOCALAPPDATA",
            "360ChromeX/Chrome/User Data",
            &[
                "Local State",
                "Default/Login Data",
                "Default/Network",
                "Default/Extensions",
                "Default/Local Storage",
                "Default/Preferences",
                "Default/Session Storage",
                "Default/Sessions",
                "Default/WebStorage",
                "Default/Download Service",
                "Default/Extension State",
            ],
            &[
                "ShaderCache64",
                "GrShaderCache64",
                "GraphiteDawnCache",
                "Default/Cache",
                "Default/Code Cache",
                "Default/GPUCache64",
                "Default/DawnGraphiteCache",
                "Default/DawnWebGPUCache",
            ],
            9,
        );
    }

    /// Confirms that Sogou Explorer blocks cleanup while its browser processes
    /// still own the real profile. Sogou Input processes are intentionally not
    /// part of this browser-specific gate.
    #[test]
    #[ignore = "requires Sogou Explorer to be running in the Windows VM"]
    fn real_windows_sogou_browser_cache_blocks_while_running() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WINDOWS_SOGOU_CACHE_BLOCK").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WINDOWS_SOGOU_CACHE_BLOCK=1 for the real process-gate diagnostic"
        );
        assert_windows_browser_cache_blocked(
            "browser.sogou-cache",
            &["SogouExplorer.exe"],
            "sogou",
        );
    }

    /// Clears only Sogou Explorer response, code, GPU, Dawn, and shader caches
    /// while hashing credentials, history, storage, downloads, and settings.
    #[test]
    #[ignore = "clears real Sogou Explorer caches in the Windows VM"]
    fn real_windows_sogou_browser_cache_preserves_profile_state() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WINDOWS_SOGOU_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WINDOWS_SOGOU_CACHE=1 only in the isolated Windows VM"
        );
        validate_real_windows_browser_cache_cleanup(
            "browser.sogou-cache",
            &["SogouExplorer.exe"],
            "LOCALAPPDATA",
            "Sogou/SogouExplorer/User Data",
            &[
                "Local State",
                "Default/History",
                "Default/Login Data",
                "Default/Network",
                "Default/Extensions",
                "Default/Local Storage",
                "Default/Session Storage",
                "Default/Sessions",
                "Default/WebStorage",
                "Default/IndexedDB",
                "Default/Download Service",
                "Default/Extension State",
            ],
            &[
                "ShaderCache",
                "GrShaderCache",
                "GraphiteDawnCache",
                "Default/Cache",
                "Default/Code Cache",
                "Default/GPUCache",
                "Default/DawnCache",
            ],
            10,
        );
    }

    #[test]
    #[ignore = "requires 360 Safe Browser to be running in the Windows VM"]
    fn real_windows_360_safe_browser_cache_blocks_while_running() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WINDOWS_360_SAFE_CACHE_BLOCK").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WINDOWS_360_SAFE_CACHE_BLOCK=1 for the real process-gate diagnostic"
        );
        assert_windows_browser_cache_blocked("browser.360-safe-cache", &["360se.exe"], "360-safe");
    }

    #[test]
    #[ignore = "clears real 360 Safe Browser caches in the Windows VM"]
    fn real_windows_360_safe_browser_cache_preserves_profile_state() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WINDOWS_360_SAFE_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WINDOWS_360_SAFE_CACHE=1 only in the isolated Windows VM"
        );
        validate_real_windows_browser_cache_cleanup(
            "browser.360-safe-cache",
            &["360se.exe"],
            "APPDATA",
            "360se6/User Data",
            &[
                "Local State",
                "Default/History",
                "Default/Login Data",
                "Default/Network",
                "Default/Extensions",
                "Default/Local Storage",
                "Default/Preferences",
                "Default/Session Storage",
                "Default/Sessions",
                "Default/WebStorage",
            ],
            &[
                "GraphiteDawnCache",
                "Default/Cache",
                "Default/Code Cache",
                "Default/DawnCache",
                "Default/Shared Dictionary/cache",
            ],
            5,
        );
    }

    #[test]
    #[ignore = "requires 2345 Browser to be running in the Windows VM"]
    fn real_windows_2345_browser_cache_blocks_while_running() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WINDOWS_2345_CACHE_BLOCK").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WINDOWS_2345_CACHE_BLOCK=1 for the real process-gate diagnostic"
        );
        assert_windows_browser_cache_blocked("browser.2345-cache", &["2345Explorer.exe"], "2345");
    }

    #[test]
    #[ignore = "clears real 2345 Browser caches in the Windows VM"]
    fn real_windows_2345_browser_cache_preserves_profile_state() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_WINDOWS_2345_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_WINDOWS_2345_CACHE=1 only in the isolated Windows VM"
        );
        validate_real_windows_browser_cache_cleanup(
            "browser.2345-cache",
            &["2345Explorer.exe"],
            "LOCALAPPDATA",
            "2345Explorer/User Data",
            &[
                "Local State",
                "Default/History",
                "Default/Login Data",
                "Default/Network",
                "Default/Extensions",
                "Default/Local Storage",
                "Default/Preferences",
                "Default/Session Storage",
                "Default/Sessions",
                "Default/WebStorage",
            ],
            &[
                "ShaderCache",
                "GrShaderCache",
                "Default/Cache",
                "Default/Code Cache",
                "Default/DawnCache",
                "Default/GPUCache",
            ],
            5,
        );
    }

    fn directories_with_leaf_names(path: &Path, leaf_names: &[&str]) -> Vec<PathBuf> {
        fn collect(path: &Path, leaf_names: &[&str], directories: &mut Vec<PathBuf>) {
            let metadata =
                fs::symlink_metadata(path).expect("the WPS profile path must remain readable");
            assert!(
                !metadata.file_type().is_symlink(),
                "the WPS profile fixture must not cross a symbolic link"
            );
            if !metadata.is_dir() {
                return;
            }
            if path.file_name().is_some_and(|name| {
                leaf_names
                    .iter()
                    .any(|leaf| name.eq_ignore_ascii_case(leaf))
            }) {
                directories.push(path.to_path_buf());
                return;
            }
            let mut children = fs::read_dir(path)
                .expect("the WPS profile directory must remain readable")
                .map(|entry| entry.expect("the directory entry must be readable").path())
                .filter(|child| child.is_dir())
                .collect::<Vec<_>>();
            children.sort();
            for child in children {
                collect(&child, leaf_names, directories);
            }
        }

        let mut directories = Vec::new();
        collect(path, leaf_names, &mut directories);
        directories.sort();
        directories
    }

    fn digest_tree_excluding_segments(path: &Path, excluded_segments: &[&str]) -> String {
        fn collect_files(path: &Path, excluded_segments: &[&str], files: &mut Vec<PathBuf>) {
            let metadata = fs::symlink_metadata(path)
                .expect("the preserved WPS application state must remain readable");
            assert!(
                !metadata.file_type().is_symlink(),
                "the preserved WPS fixture must not contain links"
            );
            if metadata.is_file() {
                files.push(path.to_path_buf());
                return;
            }
            let mut children = fs::read_dir(path)
                .expect("the preserved WPS directory must remain readable")
                .map(|entry| entry.expect("the directory entry must be readable").path())
                .filter(|child| {
                    !child.file_name().is_some_and(|name| {
                        excluded_segments
                            .iter()
                            .any(|segment| name.eq_ignore_ascii_case(segment))
                    })
                })
                .collect::<Vec<_>>();
            children.sort();
            for child in children {
                collect_files(&child, excluded_segments, files);
            }
        }

        let mut files = Vec::new();
        collect_files(path, excluded_segments, &mut files);
        let mut hasher = Sha256::new();
        for file in files {
            let relative = file
                .strip_prefix(path)
                .unwrap_or(file.as_path())
                .to_string_lossy();
            hasher.update(relative.as_bytes());
            hasher.update(fs::read(file).expect("the preserved WPS file must remain readable"));
        }
        hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect::<String>()
    }

    fn digest_tree(path: &Path) -> String {
        fn collect_files(path: &Path, files: &mut Vec<PathBuf>) {
            let metadata =
                fs::symlink_metadata(path).expect("the preserved Signal path must remain readable");
            assert!(
                !metadata.file_type().is_symlink(),
                "the preserved Signal fixture must not contain links"
            );
            if metadata.is_file() {
                files.push(path.to_path_buf());
                return;
            }
            let mut children = fs::read_dir(path)
                .expect("the preserved Signal directory must remain readable")
                .map(|entry| entry.expect("the directory entry must be readable").path())
                .collect::<Vec<_>>();
            children.sort();
            for child in children {
                collect_files(&child, files);
            }
        }

        let mut files = Vec::new();
        collect_files(path, &mut files);
        let mut hasher = Sha256::new();
        for file in files {
            let relative = file
                .strip_prefix(path)
                .unwrap_or(file.as_path())
                .to_string_lossy();
            hasher.update(relative.as_bytes());
            hasher.update(fs::read(file).expect("the preserved Signal file must remain readable"));
        }
        hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect::<String>()
    }
