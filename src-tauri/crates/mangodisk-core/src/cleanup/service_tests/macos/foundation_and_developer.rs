    #[test]
    fn cleanup_deletes_regular_files_without_following_external_links() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-boundary-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let cleanup_root = sandbox.join("cache");
        let external_root = sandbox.join("external");
        let regular_file = cleanup_root.join("regular.tmp");
        let protected_file = external_root.join("protected.txt");
        let external_link = cleanup_root.join("external-link");
        fs::create_dir_all(&cleanup_root).expect("the isolated cleanup root must be created");
        fs::create_dir_all(&external_root).expect("the external fixture must be created");
        fs::write(&regular_file, b"temporary cache")
            .expect("the regular cache file must be written");
        fs::write(&protected_file, b"must remain").expect("the protected file must be written");
        symlink(&external_root, &external_link).expect("the external symlink must be created");
        let canonical_root = validate_rule_root(&cleanup_root, &MatcherSpec::All)
            .expect("the isolated root must be safe");
        let mut stats = DeleteStats {
            matched_bytes: 0,
            deleted_bytes: 0,
            affected_item_count: 0,
            failed_item_count: 0,
            removed_empty_directory_count: 0,
        };

        delete_root_contents(
            &cleanup_root,
            &canonical_root,
            &MatcherSpec::All,
            &|_, _| true,
            &|| false,
            &mut stats,
        );

        assert!(
            !regular_file.exists(),
            "the regular cache file must be deleted"
        );
        assert!(
            protected_file.exists(),
            "cleanup must not follow links outside the rule root"
        );
        assert!(
            external_link.symlink_metadata().is_ok(),
            "a rejected link must remain unchanged"
        );
        assert_eq!(stats.affected_item_count, 1);
        assert_eq!(stats.failed_item_count, 1);
    }
    #[test]
    #[ignore = "modifies HOME and executes isolated cleanup; run this test alone"]
    fn communication_cache_rule_preserves_message_container_data() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::current_dir()
            .expect("the test process must have a working directory")
            .join("target")
            .join(format!(
                "mangodisk-communication-cache-cleanup-test-{}-{}",
                std::process::id(),
                now_ms()
            ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let home = sandbox.join("home");
        let cache_file = home
            .join("Library/Caches/net.whatsapp.WhatsApp")
            .join("generated-cache.bin");
        let message_database = home
            .join("Library/Containers/net.whatsapp.WhatsApp/Data/Documents")
            .join("messages.db");
        for fixture in [&cache_file, &message_database] {
            fs::create_dir_all(fixture.parent().expect("the fixture must have a parent"))
                .expect("the isolated application directory must be created");
            fs::write(fixture, b"MangoDisk communication cache fixture")
                .expect("the isolated fixture must be written");
        }

        let _restore = EnvironmentRestore(vec![("HOME", std::env::var_os("HOME"))]);
        std::env::set_var("HOME", &home);

        let preview = CleanupService::execute(CleanupRequest {
            rule_ids: vec!["app.whatsapp-cache".to_string()],
            source_selections: Vec::new(),
            dry_run: true,
            project_roots: Vec::new(),
        })
        .expect("isolated communication cache preview must succeed");
        assert_eq!(preview.failed_item_count, 0);
        assert!(
            cache_file.exists(),
            "dry-run must preserve the cache fixture"
        );
        assert!(
            message_database.exists(),
            "dry-run must preserve message container data"
        );

        let result = CleanupService::execute(CleanupRequest {
            rule_ids: vec!["app.whatsapp-cache".to_string()],
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        })
        .expect("isolated communication cache cleanup must succeed");
        assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
        assert_eq!(result.affected_item_count, 1);
        assert!(
            !cache_file.exists(),
            "the rebuildable bundle cache must be deleted"
        );
        assert!(
            message_database.exists(),
            "message container data must remain outside the cleanup boundary"
        );
    }

    #[test]
    #[ignore = "modifies HOME and executes isolated cleanup; run this test alone"]
    fn developer_cache_rules_preserve_tools_configuration_and_project_data() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::current_dir()
            .expect("the test process must have a working directory")
            .join("target")
            .join(format!(
                "mangodisk-developer-cache-cleanup-test-{}-{}",
                std::process::id(),
                now_ms()
            ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let home = sandbox.join("home");
        let cache_files = [
            home.join("Library/Caches/deno/deps/cache.bin"),
            home.join(".bun/install/cache/package/index.js"),
            home.join("Library/Caches/composer/files/package.zip"),
            home.join(".composer/cache/repo/metadata.json"),
            home.join("Library/Caches/mise/node/remote_versions.msgpack.z"),
            home.join("Library/Caches/ccache/a/result"),
            home.join("Library/Caches/Mozilla.sccache/0/compile-result"),
            home.join(".gem/ruby/4.0.0/cache/example-1.0.0.gem"),
            home.join(".hex/cache/registry.ets"),
            home.join("Library/Caches/copilot/marketplace/index.json"),
            home.join(".m2/repository/org/example/demo/1.0/demo-1.0.jar"),
            home.join(".nuget/packages/example/1.0/example.1.0.nupkg"),
            home.join(".gradle/wrapper/dists/gradle-bin/hash/gradle/bin/gradle"),
            home.join(".gradle/.tmp/download.part"),
        ];
        let protected_files = [
            home.join(".deno/bin/deno"),
            home.join(".bun/bin/bun"),
            home.join(".composer/auth.json"),
            home.join(".local/share/mise/installs/node/22/bin/node"),
            home.join("Library/Caches/mise/http-tarballs/tool/extracted/bin/http-backend-tool"),
            home.join("project/vendor/package/source.php"),
            home.join("Library/Preferences/ccache/ccache.conf"),
            home.join("Library/Application Support/Mozilla.sccache/config"),
            home.join(".gem/ruby/4.0.0/gems/example-1.0.0/lib/example.rb"),
            home.join(".hex/hex.config"),
            home.join(".copilot/settings.json"),
            home.join(".m2/settings.xml"),
            home.join(".nuget/NuGet/NuGet.Config"),
            home.join("project/pom.xml"),
            home.join(".gradle/gradle.properties"),
            home.join("project/gradle/wrapper/gradle-wrapper.properties"),
        ];
        for fixture in cache_files.iter().chain(&protected_files) {
            fs::create_dir_all(fixture.parent().expect("the fixture must have a parent"))
                .expect("the isolated developer tool directory must be created");
            fs::write(fixture, b"MangoDisk developer cache fixture")
                .expect("the isolated developer tool fixture must be written");
        }

        let _restore = EnvironmentRestore(vec![("HOME", std::env::var_os("HOME"))]);
        std::env::set_var("HOME", &home);
        let rule_ids = [
            "dev.deno-cache",
            "dev.bun-cache",
            "dev.composer-cache",
            "dev.mise-cache",
            "dev.ccache-cache",
            "dev.sccache-cache",
            "dev.rubygems-cache",
            "dev.hex-cache",
            "dev.copilot-cli-cache",
            "dev.maven-cache",
            "dev.nuget-cache",
            "dev.gradle-cache",
        ]
        .map(str::to_string)
        .to_vec();

        let preview = CleanupService::execute(CleanupRequest {
            rule_ids: rule_ids.clone(),
            source_selections: Vec::new(),
            dry_run: true,
            project_roots: Vec::new(),
        })
        .expect("isolated developer cache preview must succeed");
        assert_eq!(preview.failed_item_count, 0, "{:?}", preview.actions);
        assert!(cache_files.iter().all(|fixture| fixture.exists()));
        assert!(protected_files.iter().all(|fixture| fixture.exists()));

        let result = CleanupService::execute(CleanupRequest {
            rule_ids,
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        })
        .expect("isolated developer cache cleanup must succeed");
        assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
        assert_eq!(result.affected_item_count, 14);
        assert!(cache_files.iter().all(|fixture| !fixture.exists()));
        assert!(protected_files.iter().all(|fixture| fixture.exists()));
    }

    /// Verifies the macOS Chrome rule against an isolated profile. Browser-level
    /// shader caches are deliberately placed beside account and browsing state
    /// so future root changes cannot silently widen the cleanup boundary.
    #[test]
    #[ignore = "modifies HOME and requires Google Chrome to be stopped"]
    fn chrome_cache_rule_preserves_isolated_profile_state() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::current_dir()
            .expect("the test process must have a working directory")
            .join("target")
            .join(format!(
                "mangodisk-chrome-cache-cleanup-test-{}-{}",
                std::process::id(),
                now_ms()
            ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let home = sandbox.join("home");
        let chrome_root = home.join("Library/Application Support/Google/Chrome");
        let cache_files = [
            home.join("Library/Caches/Google/Chrome/http-cache/data.bin"),
            chrome_root.join("ShaderCache/data.bin"),
            chrome_root.join("GrShaderCache/data.bin"),
            chrome_root.join("GraphiteDawnCache/data.bin"),
            chrome_root.join("GPUPersistentCache/data.bin"),
            chrome_root.join("Default/Cache/data.bin"),
            chrome_root.join("Default/Code Cache/data.bin"),
            chrome_root.join("Default/GPUCache/data.bin"),
        ];
        let protected_files = [
            chrome_root.join("Local State"),
            chrome_root.join("Default/Bookmarks"),
            chrome_root.join("Default/Cookies"),
            chrome_root.join("Default/History"),
            chrome_root.join("Default/Login Data"),
            chrome_root.join("Default/Preferences"),
            chrome_root.join("Default/Extensions/example/manifest.json"),
            chrome_root.join("Default/Service Worker/Database/000001.log"),
        ];
        for fixture in cache_files.iter().chain(&protected_files) {
            fs::create_dir_all(fixture.parent().expect("the fixture must have a parent"))
                .expect("the isolated Chrome directory must be created");
            fs::write(fixture, b"MangoDisk Chrome cache fixture")
                .expect("the isolated Chrome fixture must be written");
        }

        let _restore = EnvironmentRestore(vec![("HOME", std::env::var_os("HOME"))]);
        std::env::set_var("HOME", &home);
        let request = |dry_run| CleanupRequest {
            rule_ids: vec!["browser.chrome-cache".to_string()],
            source_selections: Vec::new(),
            dry_run,
            project_roots: Vec::new(),
        };

        let preview = CleanupService::execute(request(true))
            .expect("the isolated Chrome cache preview must succeed");
        assert_eq!(preview.failed_item_count, 0, "{:?}", preview.actions);
        assert!(cache_files.iter().all(|fixture| fixture.exists()));
        assert!(protected_files.iter().all(|fixture| fixture.exists()));

        let result = CleanupService::execute(request(false))
            .expect("the isolated Chrome cache cleanup must succeed");
        assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
        assert_eq!(result.affected_item_count, cache_files.len() as u64);
        assert!(cache_files.iter().all(|fixture| !fixture.exists()));
        assert!(protected_files.iter().all(|fixture| fixture.exists()));
    }

    /// Confirms that a live Chrome process blocks the complete rule before any
    /// browser-level or profile cache is traversed.
    #[test]
    #[ignore = "requires the real Google Chrome application to be running"]
    fn real_chrome_cache_blocks_while_running() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_MACOS_CHROME_CACHE_BLOCK").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_MACOS_CHROME_CACHE_BLOCK=1 for the real process-gate diagnostic"
        );
        let running = ProcessSnapshot::capture()
            .expect("the macOS process inventory must be available")
            .matching_processes(&["Google Chrome".to_string()]);
        assert!(!running.is_empty(), "Google Chrome must be running");

        let result = CleanupService::execute(CleanupRequest {
            rule_ids: vec!["browser.chrome-cache".to_string()],
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        })
        .expect("the blocked Chrome cleanup must return a structured result");
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
            "real_macos_chrome_cache_block running_process_count={}",
            action.running_processes.len()
        );
    }

    /// Runs the production Chrome rule against the initialized local profile.
    /// The test records representative durable-state metadata before mutation
    /// and adds a marker to every discovered cache root so dry-run and deletion
    /// coverage remain observable when Chrome has already pruned a cache itself.
    #[test]
    #[ignore = "permanently clears real Google Chrome caches"]
    fn real_chrome_cache_preserves_profile_state() {
        fn tree_metadata_signature(root: &Path) -> (u64, u64, u128) {
            fn visit(root: &Path, path: &Path, signature: &mut (u64, u64, u128)) {
                let metadata = fs::symlink_metadata(path)
                    .expect("the preserved Chrome metadata must remain readable");
                signature.0 = signature.0.saturating_add(1);
                signature.1 = signature.1.saturating_add(metadata.len());
                signature.2 = signature.2.saturating_add(
                    metadata
                        .modified()
                        .ok()
                        .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
                        .map(|duration| duration.as_nanos())
                        .unwrap_or_default(),
                );
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return;
                }
                for entry in fs::read_dir(path)
                    .expect("the preserved Chrome metadata directory must be readable")
                {
                    let child = entry
                        .expect("the preserved Chrome metadata entry must be readable")
                        .path();
                    assert!(child.starts_with(root));
                    visit(root, &child, signature);
                }
            }

            let mut signature = (0u64, 0u64, 0u128);
            visit(root, root, &mut signature);
            signature
        }

        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_MACOS_CHROME_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_MACOS_CHROME_CACHE=1 to authorize this real cache diagnostic"
        );
        let running = ProcessSnapshot::capture()
            .expect("the macOS process inventory must be available")
            .matching_processes(&["Google Chrome".to_string()]);
        assert!(
            running.is_empty(),
            "Google Chrome must be completely stopped"
        );

        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .expect("HOME must be available");
        let chrome_root = home.join("Library/Application Support/Google/Chrome");
        let profile = chrome_root.join("Default");
        assert!(
            profile.is_dir(),
            "Google Chrome must have an initialized profile"
        );

        let preserved_candidates = [
            chrome_root.join("Local State"),
            profile.join("Bookmarks"),
            profile.join("Cookies"),
            profile.join("History"),
            profile.join("Login Data"),
            profile.join("Network"),
            profile.join("Preferences"),
            profile.join("Extensions"),
            profile.join("Local Storage"),
            profile.join("Service Worker"),
            profile.join("Sessions"),
            profile.join("WebStorage"),
        ];
        let preserved_paths = preserved_candidates
            .into_iter()
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        assert!(
            preserved_paths.len() >= 9,
            "the initialized Chrome profile must expose representative durable state"
        );
        let preserved_before = preserved_paths
            .iter()
            .map(|path| tree_metadata_signature(path))
            .collect::<Vec<_>>();

        let mut target_roots = [
            home.join("Library/Caches/Google/Chrome"),
            chrome_root.join("ShaderCache"),
            chrome_root.join("GrShaderCache"),
            chrome_root.join("GraphiteDawnCache"),
            chrome_root.join("GPUPersistentCache"),
        ]
        .into_iter()
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
        for profile_name in fs::read_dir(&chrome_root)
            .expect("the Chrome profile root must be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    name == "Default"
                        || name == "Guest Profile"
                        || name == "System Profile"
                        || name.to_string_lossy().starts_with("Profile ")
                })
            })
        {
            for suffix in [
                "Cache",
                "Code Cache",
                "GPUCache",
                "DawnCache",
                "GrShaderCache",
            ] {
                let candidate = profile_name.join(suffix);
                if candidate.is_dir() {
                    target_roots.push(candidate);
                }
            }
        }
        target_roots.sort();
        target_roots.dedup();
        assert!(
            target_roots
                .iter()
                .any(|path| path == &chrome_root.join("GraphiteDawnCache")),
            "the real profile must expose the newly covered Graphite cache"
        );

        let markers = target_roots
            .iter()
            .map(|root| root.join("mangodisk-rule-validation.bin"))
            .collect::<Vec<_>>();
        let _marker_cleanup = FileCleanup(markers.clone());
        for marker in &markers {
            fs::write(marker, b"payload").expect("the Chrome cache marker must be created");
        }
        let request = |dry_run| CleanupRequest {
            rule_ids: vec!["browser.chrome-cache".to_string()],
            source_selections: Vec::new(),
            dry_run,
            project_roots: Vec::new(),
        };

        let preview = CleanupService::execute(request(true))
            .expect("the real Chrome cache preview must succeed");
        assert_eq!(preview.failed_item_count, 0, "{:?}", preview.actions);
        assert!(preview.expected_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| marker.exists()));

        let result = CleanupService::execute(request(false))
            .expect("the real Chrome cache cleanup must succeed");
        assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
        assert!(result.released_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| !marker.exists()));
        let preserved_after = preserved_paths
            .iter()
            .map(|path| tree_metadata_signature(path))
            .collect::<Vec<_>>();
        assert_eq!(preserved_after, preserved_before);
        println!(
            "real_macos_chrome_cache_cleanup expected_bytes={} released_bytes={} affected_item_count={} target_root_count={} preserved_root_count={}",
            preview.expected_bytes,
            result.released_bytes,
            result.affected_item_count,
            target_roots.len(),
            preserved_paths.len()
        );
    }

    /// Runs the complete production Gradle rule so the new Wrapper and temp
    /// roots are validated together with the existing cache family. Sibling
    /// Gradle metadata remains hashed to detect accidental boundary expansion.
    #[test]
    #[ignore = "permanently clears real Gradle caches and downloaded Wrapper distributions"]
    fn real_gradle_cache_preserves_non_cache_state() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_MACOS_GRADLE_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_MACOS_GRADLE_CACHE=1 to authorize this real cache diagnostic"
        );
        let running = ProcessSnapshot::capture()
            .expect("the macOS process inventory must be available")
            .matching_processes(&["gradle".to_string(), "java".to_string()]);
        assert!(
            running.is_empty(),
            "Gradle and Java must be completely stopped"
        );

        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .expect("HOME must be available");
        let gradle_home = home.join(".gradle");
        let required_new_roots = [gradle_home.join("wrapper/dists"), gradle_home.join(".tmp")];
        assert!(
            required_new_roots.iter().all(|path| path.is_dir()),
            "both newly covered Gradle roots must exist"
        );
        let target_roots = [
            gradle_home.join("caches"),
            gradle_home.join("daemon"),
            gradle_home.join("workers"),
            gradle_home.join("notifications"),
            required_new_roots[0].clone(),
            required_new_roots[1].clone(),
        ]
        .into_iter()
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
        let preserved_paths = [
            gradle_home.join("android"),
            gradle_home.join("kotlin-profile"),
            gradle_home.join("native"),
            gradle_home.join("gradle.properties"),
            gradle_home.join("init.gradle"),
            gradle_home.join("init.d"),
            gradle_home.join("jdks"),
        ]
        .into_iter()
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
        assert!(
            !preserved_paths.is_empty(),
            "the Gradle home must expose non-cache state for boundary verification"
        );
        let preserved_before = preserved_paths
            .iter()
            .map(|path| digest_macos_tree_without_following_links(path))
            .collect::<Vec<_>>();

        let markers = target_roots
            .iter()
            .map(|root| root.join("mangodisk-rule-validation.bin"))
            .collect::<Vec<_>>();
        let _marker_cleanup = FileCleanup(markers.clone());
        for marker in &markers {
            fs::write(marker, b"payload").expect("the Gradle cache marker must be created");
        }
        let request = |dry_run| CleanupRequest {
            rule_ids: vec!["dev.gradle-cache".to_string()],
            source_selections: Vec::new(),
            dry_run,
            project_roots: Vec::new(),
        };

        let preview = CleanupService::execute(request(true))
            .expect("the real Gradle cache preview must succeed");
        assert_eq!(preview.failed_item_count, 0, "{:?}", preview.actions);
        assert!(preview.expected_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| marker.exists()));

        let result = CleanupService::execute(request(false))
            .expect("the real Gradle cache cleanup must succeed");
        assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
        assert!(result.released_bytes >= markers.len() as u64 * 7);
        assert!(markers.iter().all(|marker| !marker.exists()));
        let preserved_after = preserved_paths
            .iter()
            .map(|path| digest_macos_tree_without_following_links(path))
            .collect::<Vec<_>>();
        assert_eq!(preserved_after, preserved_before);
        println!(
            "real_macos_gradle_cache_cleanup expected_bytes={} released_bytes={} affected_item_count={} target_root_count={} preserved_root_count={}",
            preview.expected_bytes,
            result.released_bytes,
            result.affected_item_count,
            target_roots.len(),
            preserved_paths.len()
        );
    }

    /// Exercises the reference-derived macOS rules against isolated layouts
    /// that mirror the verified applications. Poetry's virtual environment,
    /// PyInstaller siblings, Ollama models, VS Code settings, and Docker state
    /// deliberately sit beside the selected cache data to guard each boundary.
    #[test]
    #[ignore = "modifies HOME and requires VS Code and Docker Desktop to be stopped"]
    fn reference_cache_rules_preserve_durable_developer_and_application_state() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::current_dir()
            .expect("the test process must have a working directory")
            .join("target")
            .join(format!(
                "mangodisk-reference-cache-cleanup-test-{}-{}",
                std::process::id(),
                now_ms()
            ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let home = sandbox.join("home");
        let cache_files = [
            home.join("Library/Caches/pypoetry/artifacts/aa/package.whl"),
            home.join("Library/Caches/pypoetry/cache/repositories/PyPI/index.json"),
            home.join("Library/Application Support/pyinstaller/bincache00py31364bit/arm64/adhoc/no-entitlements/index.dat"),
            home.join("Library/Application Support/Code/CachedExtensionVSIXs/extension.vsix"),
            home.join("Library/Caches/ollama/updates/hash/Ollama-darwin.zip"),
            home.join("Library/Containers/com.docker.docker/Data/log/vm/init.log.1"),
        ];
        let recent_ollama_update =
            home.join("Library/Caches/ollama/updates/recent/Ollama-darwin.zip");
        let recent_docker_log =
            home.join("Library/Containers/com.docker.docker/Data/log/vm/init.log");
        let protected_files = [
            home.join("Library/Caches/pypoetry/virtualenvs/project-py3.13/pyvenv.cfg"),
            home.join("Library/Application Support/pyinstaller/state/keep.json"),
            home.join(".ollama/models/blobs/sha256-model"),
            home.join("Library/Application Support/Code/User/settings.json"),
            home.join("Library/Group Containers/group.com.docker/settings-store.json"),
            home.join("Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw"),
        ];
        for fixture in cache_files
            .iter()
            .chain([&recent_ollama_update, &recent_docker_log])
            .chain(&protected_files)
        {
            fs::create_dir_all(fixture.parent().expect("the fixture must have a parent"))
                .expect("the isolated reference cache directory must be created");
            fs::write(fixture, b"MangoDisk reference cache fixture")
                .expect("the isolated reference cache fixture must be written");
        }
        let stale_time = SystemTime::now()
            .checked_sub(Duration::from_secs(8 * 86_400))
            .expect("the test time must move back by eight days");
        for fixture in [&cache_files[4], &cache_files[5]] {
            fs::File::options()
                .write(true)
                .open(fixture)
                .expect("the stale reference fixture must open")
                .set_times(fs::FileTimes::new().set_modified(stale_time))
                .expect("the stale reference fixture timestamp must be set");
        }

        let _restore = EnvironmentRestore(vec![("HOME", std::env::var_os("HOME"))]);
        std::env::set_var("HOME", &home);
        let rule_ids = [
            "dev.python-cache",
            "dev.pyinstaller-cache",
            "dev.vscode-cache",
            "app.ollama-update-cache",
            "container.docker-desktop-diagnostic-cache",
        ]
        .map(str::to_string)
        .to_vec();

        let preview = CleanupService::execute(CleanupRequest {
            rule_ids: rule_ids.clone(),
            source_selections: Vec::new(),
            dry_run: true,
            project_roots: Vec::new(),
        })
        .expect("the isolated reference cache preview must succeed");
        assert_eq!(preview.failed_item_count, 0, "{:?}", preview.actions);
        assert!(cache_files.iter().all(|fixture| fixture.exists()));
        assert!(protected_files.iter().all(|fixture| fixture.exists()));

        let result = CleanupService::execute(CleanupRequest {
            rule_ids,
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        })
        .expect("the isolated reference cache cleanup must succeed");
        assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
        assert_eq!(result.affected_item_count, 6, "{:?}", result.actions);
        assert!(cache_files.iter().all(|fixture| !fixture.exists()));
        assert!(recent_ollama_update.exists());
        assert!(recent_docker_log.exists());
        assert!(protected_files.iter().all(|fixture| fixture.exists()));
    }

    /// Permanently clears the verified real cache families after their owners
    /// stop. Durable state is recorded from disjoint Poetry, Ollama, VS Code,
    /// and Docker locations; the large Ollama model store and Docker VM disk
    /// use metadata signatures so validation never reads multi-gigabyte data.
    #[test]
    #[ignore = "permanently clears real Poetry, PyInstaller, Ollama, VS Code, and Docker caches"]
    fn real_reference_cache_rules_preserve_environments_models_and_vm_state() {
        fn tree_metadata_signature(root: &Path) -> (u64, u64, u128) {
            fn visit(path: &Path, signature: &mut (u64, u64, u128)) {
                let metadata = fs::symlink_metadata(path)
                    .expect("the preserved metadata entry must remain readable");
                signature.0 = signature.0.saturating_add(1);
                signature.1 = signature.1.saturating_add(metadata.len());
                signature.2 = signature.2.saturating_add(
                    metadata
                        .modified()
                        .ok()
                        .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
                        .map(|duration| duration.as_nanos())
                        .unwrap_or_default(),
                );
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return;
                }
                for entry in
                    fs::read_dir(path).expect("the preserved metadata directory must be readable")
                {
                    visit(
                        &entry
                            .expect("the preserved metadata entry must be readable")
                            .path(),
                        signature,
                    );
                }
            }

            let mut signature = (0u64, 0u64, 0u128);
            visit(root, &mut signature);
            signature
        }

        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_MACOS_REFERENCE_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_MACOS_REFERENCE_CACHE=1 to authorize this real cache diagnostic"
        );
        let process_snapshot =
            ProcessSnapshot::capture().expect("the macOS process inventory must be available");
        for process_name in [
            "Visual Studio Code",
            "Code",
            "Docker",
            "Docker Desktop",
            "com.docker.backend",
            "com.docker.virtualization",
        ] {
            assert!(
                process_snapshot
                    .matching_processes(&[process_name.to_string()])
                    .is_empty(),
                "every reference cache owner must be stopped before cleanup"
            );
        }

        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .expect("HOME must be available");
        let poetry_virtualenvs = home.join("Library/Caches/pypoetry/virtualenvs");
        let ollama_models = home.join(".ollama/models");
        let vscode_user = home.join("Library/Application Support/Code/User");
        let docker_group = home.join("Library/Group Containers/group.com.docker");
        let docker_disk =
            home.join("Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw");
        for path in [
            &poetry_virtualenvs,
            &ollama_models,
            &vscode_user,
            &docker_group,
            &docker_disk,
        ] {
            assert!(path.exists(), "every durable-state fixture must exist");
        }
        let poetry_before = digest_macos_tree_without_following_links(&poetry_virtualenvs);
        let ollama_before = tree_metadata_signature(&ollama_models);
        let vscode_before = digest_macos_tree_without_following_links(&vscode_user);
        let docker_group_before = digest_macos_tree_without_following_links(&docker_group);
        let docker_disk_before = tree_metadata_signature(&docker_disk);

        let rule_ids = [
            "dev.python-cache",
            "dev.pyinstaller-cache",
            "dev.vscode-cache",
            "app.ollama-update-cache",
            "container.docker-desktop-diagnostic-cache",
        ]
        .map(str::to_string)
        .to_vec();
        let preview = CleanupService::execute(CleanupRequest {
            rule_ids: rule_ids.clone(),
            source_selections: Vec::new(),
            dry_run: true,
            project_roots: Vec::new(),
        })
        .expect("the real reference cache preview must succeed");
        assert_eq!(preview.failed_item_count, 0, "{:?}", preview.actions);
        assert!(
            preview.expected_bytes > 350 * 1024 * 1024,
            "the real reference cache baseline must provide material benefit"
        );
        assert_eq!(
            digest_macos_tree_without_following_links(&poetry_virtualenvs),
            poetry_before
        );
        assert_eq!(tree_metadata_signature(&ollama_models), ollama_before);
        assert_eq!(
            digest_macos_tree_without_following_links(&vscode_user),
            vscode_before
        );
        assert_eq!(
            digest_macos_tree_without_following_links(&docker_group),
            docker_group_before
        );
        assert_eq!(tree_metadata_signature(&docker_disk), docker_disk_before);

        let result = CleanupService::execute(CleanupRequest {
            rule_ids,
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        })
        .expect("the real reference cache cleanup must succeed");
        assert_eq!(result.failed_item_count, 0, "{:?}", result.actions);
        assert!(result.released_bytes > 350 * 1024 * 1024);
        assert!(result.affected_item_count > 100);
        assert_eq!(
            digest_macos_tree_without_following_links(&poetry_virtualenvs),
            poetry_before
        );
        assert_eq!(tree_metadata_signature(&ollama_models), ollama_before);
        assert_eq!(
            digest_macos_tree_without_following_links(&vscode_user),
            vscode_before
        );
        assert_eq!(
            digest_macos_tree_without_following_links(&docker_group),
            docker_group_before
        );
        assert_eq!(tree_metadata_signature(&docker_disk), docker_disk_before);
        println!(
            "real_macos_reference_cache_cleanup expected_bytes={} released_bytes={} affected_item_count={} preserved_root_count=5",
            preview.expected_bytes, result.released_bytes, result.affected_item_count
        );
    }

    #[test]
    #[ignore = "modifies HOME and executes isolated cleanup; run this test alone"]
    fn ai_cache_rules_clean_only_rebuildable_data_and_preserve_models() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        // HOME cannot be nested under the system temporary directory because
        // the real system.user-temp rule would correctly own its parent. Keep
        // the isolated home under target to avoid user data and preserve the
        // same non-overlapping relationship as a real home directory.
        let sandbox = std::env::current_dir()
            .expect("the test process must have a working directory")
            .join("target")
            .join(format!(
                "mangodisk-ai-cache-cleanup-test-{}-{}",
                std::process::id(),
                now_ms()
            ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let home = sandbox.join("home");
        let downloads = home.join("Downloads");
        let huggingface_hub = home.join(".cache/huggingface/hub/models--fixture/blobs");
        let xet_environment = home.join(".cache/huggingface/xet/environment");
        let xet_chunk_cache = xet_environment.join("chunk_cache");
        let xet_shard_cache = xet_environment.join("shard_cache");
        let xet_staging = xet_environment.join("staging");
        let project = home.join("project");
        for directory in [
            &downloads,
            &huggingface_hub,
            &xet_chunk_cache,
            &xet_shard_cache,
            &xet_staging,
            &project,
        ] {
            fs::create_dir_all(directory).expect("the isolated rule directory must be created");
        }

        let stale_partial = downloads.join("old-model.crdownload");
        let recent_partial = downloads.join("active-model.crdownload");
        let completed_download = downloads.join("archive.zip");
        let downloaded_model = huggingface_hub.join("downloaded-model.bin");
        let xet_chunk = xet_chunk_cache.join("chunk.bin");
        let xet_shard = xet_shard_cache.join("shard.mdb");
        let resumable_upload = xet_staging.join("upload.mdb");
        let project_model = project.join("model.bin");
        for fixture in [
            &stale_partial,
            &recent_partial,
            &completed_download,
            &downloaded_model,
            &xet_chunk,
            &xet_shard,
            &resumable_upload,
            &project_model,
        ] {
            fs::write(fixture, b"MangoDisk AI cache fixture")
                .expect("the isolated cleanup fixture must be written");
        }
        let stale_time = SystemTime::now()
            .checked_sub(Duration::from_secs(8 * 86_400))
            .expect("the fixture timestamp must support an eight-day offset");
        fs::File::options()
            .write(true)
            .open(&stale_partial)
            .expect("the stale download fixture must open")
            .set_times(fs::FileTimes::new().set_modified(stale_time))
            .expect("the stale download timestamp must be updated");

        let _restore = EnvironmentRestore(vec![("HOME", std::env::var_os("HOME"))]);
        std::env::set_var("HOME", &home);

        assert!(
            validate_rule_root(&downloads, &MatcherSpec::All).is_err(),
            "Downloads must not be authorized as a broad cleanup root"
        );
        assert!(
            validate_rule_root(
                &downloads,
                &MatcherSpec::AllOf(vec![
                    MatcherSpec::OlderThanDays(6),
                    MatcherSpec::ExtensionIn(vec!["crdownload".to_string()]),
                ]),
            )
            .is_err(),
            "a recent partial download must not be authorized"
        );
        assert!(
            validate_rule_root(
                &downloads,
                &MatcherSpec::AllOf(vec![
                    MatcherSpec::OlderThanDays(7),
                    MatcherSpec::ExtensionIn(vec!["zip".to_string()]),
                ]),
            )
            .is_err(),
            "a regular downloaded file must not be authorized"
        );

        let retired_rule = CleanupService::execute(CleanupRequest {
            rule_ids: vec![
                "system.stale-partial-downloads".to_string(),
                "ai.model-cache".to_string(),
            ],
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        });
        assert!(
            retired_rule.is_err(),
            "a retired unsafe rule ID must be rejected before deletion"
        );
        assert!(
            stale_partial.exists(),
            "an unknown rule must prevent all deletion in the request"
        );
        assert!(
            CleanupService::execute(CleanupRequest {
                rule_ids: vec!["ai.gemini-temp-files".to_string()],
                source_selections: Vec::new(),
                dry_run: false,
                project_roots: Vec::new(),
            })
            .is_err(),
            "the retired rule that covered Gemini sessions must stay unavailable"
        );

        let result = CleanupService::execute(CleanupRequest {
            rule_ids: vec![
                "system.stale-partial-downloads".to_string(),
                "ai.huggingface-xet-cache".to_string(),
            ],
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        })
        .expect("isolated stale-download and AI transfer-cache cleanup must succeed");

        assert_eq!(
            result.failed_item_count, 0,
            "isolated cleanup must not fail: {:?}",
            result.actions
        );
        assert_eq!(result.affected_item_count, 3);
        assert!(
            !stale_partial.exists(),
            "a partial download older than seven days must be deleted"
        );
        assert!(
            !xet_chunk.exists(),
            "the Xet download transfer cache must be deleted"
        );
        assert!(
            !xet_shard.exists(),
            "the Xet upload transfer cache must be deleted"
        );
        assert!(downloaded_model.exists(), "Hugging Face models must remain");
        assert!(
            resumable_upload.exists(),
            "resumable Xet uploads must remain"
        );
        assert!(
            recent_partial.exists(),
            "recent partial downloads must remain"
        );
        assert!(
            completed_download.exists(),
            "completed downloads must remain"
        );
        assert!(
            project_model.exists(),
            "models inside projects must remain unchanged"
        );
    }

    /// Verifies the executable-name gates for the newly absorbed VS Code and
    /// Docker Desktop rules against the real signed applications. Both rules
    /// must stop before filesystem traversal while their owners are running.
    #[test]
    #[ignore = "requires real VS Code and Docker Desktop processes"]
    fn real_reference_cache_rules_block_running_owners() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_MACOS_REFERENCE_CACHE_BLOCK").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_MACOS_REFERENCE_CACHE_BLOCK=1 for the real process-gate diagnostic"
        );
        let cases = [
            (
                "container.docker-desktop-diagnostic-cache",
                vec![
                    "Docker".to_string(),
                    "Docker Desktop".to_string(),
                    "com.docker.backend".to_string(),
                ],
            ),
            ("dev.vscode-cache", vec!["Code".to_string()]),
        ];
        let process_snapshot =
            ProcessSnapshot::capture().expect("the macOS process inventory must be available");

        for (rule_id, process_names) in &cases {
            assert!(
                !process_snapshot
                    .matching_processes(process_names)
                    .is_empty(),
                "every reference cache owner must be running"
            );
            let result = CleanupService::execute(CleanupRequest {
                rule_ids: vec![(*rule_id).to_string()],
                source_selections: Vec::new(),
                dry_run: false,
                project_roots: Vec::new(),
            })
            .expect("the blocked reference cache cleanup must return a structured result");
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
        }
        println!(
            "real_macos_reference_cache_block owner_count={}",
            cases.len()
        );
    }

    #[test]
    fn stale_codex_reports_match_only_direct_report_files() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        // A checkout can live under protected Projects/Documents directories. Keep this
        // disposable fixture in the canonical OS temporary directory so the production root
        // policy still applies. Canonicalization also avoids the macOS /var alias when the
        // direct-child matcher compares a file's parent with the canonical cleanup root.
        let sandbox = fs::canonicalize(std::env::temp_dir())
            .expect("the OS temporary directory must resolve")
            .join(format!(
                "mangodisk-codex-report-boundary-test-{}-{}",
                std::process::id(),
                now_ms()
            ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let pending_root = sandbox.join("pending");
        let nested_root = pending_root.join("attachments");
        fs::create_dir_all(&nested_root).expect("the isolated pending queue must be created");

        let stale_report = pending_root.join("stale.json");
        let stale_dump = pending_root.join("stale.dmp");
        let stale_metadata = pending_root.join("stale.meta");
        let recent_report = pending_root.join("recent.json");
        let unrelated_file = pending_root.join("stale.txt");
        let nested_report = nested_root.join("stale.json");
        for fixture in [
            &stale_report,
            &stale_dump,
            &stale_metadata,
            &recent_report,
            &unrelated_file,
            &nested_report,
        ] {
            fs::write(fixture, b"MangoDisk crash report fixture")
                .expect("the isolated report fixture must be written");
        }

        let stale_time = SystemTime::now()
            .checked_sub(Duration::from_secs(8 * 86_400))
            .expect("the fixture timestamp must support an eight-day offset");
        for fixture in [
            &stale_report,
            &stale_dump,
            &stale_metadata,
            &unrelated_file,
            &nested_report,
        ] {
            fs::File::options()
                .write(true)
                .open(fixture)
                .expect("the stale report fixture must open")
                .set_times(fs::FileTimes::new().set_modified(stale_time))
                .expect("the stale report timestamp must be updated");
        }

        // Load the production matcher so a future TOML change cannot widen the
        // destructive boundary while this regression test continues to pass
        // against an independently copied matcher.
        let rules = crate::cleanup::rules::registry()
            .expect("the production cleanup catalog must compile");
        let rule = rules
            .iter()
            .find(|rule| rule.id == "app.codex-diagnostic-cache")
            .expect("the Codex diagnostic cleanup rule must be registered");
        assert_eq!(rule.required_stopped_processes, ["Codex", "ChatGPT"]);
        let canonical_root = validate_rule_root(&pending_root, &rule.matcher)
            .expect("the isolated pending queue must be a valid cleanup root");
        let mut stats = DeleteStats {
            matched_bytes: 0,
            deleted_bytes: 0,
            affected_item_count: 0,
            failed_item_count: 0,
            removed_empty_directory_count: 0,
        };

        delete_root_contents(
            &pending_root,
            &canonical_root,
            &rule.matcher,
            &|_, _| true,
            &|| false,
            &mut stats,
        );

        assert!(!stale_report.exists(), "the stale direct report must be deleted");
        assert!(!stale_dump.exists(), "the stale direct dump must be deleted");
        assert!(
            !stale_metadata.exists(),
            "the stale direct metadata must be deleted"
        );
        assert!(recent_report.exists(), "recent reports must remain");
        assert!(unrelated_file.exists(), "unrelated extensions must remain");
        assert!(nested_report.exists(), "nested attachment data must remain");
        assert_eq!(stats.affected_item_count, 3);
        assert_eq!(stats.failed_item_count, 0);
    }

    #[test]
    #[ignore = "scans the real Codex Crashpad queue without deleting reports"]
    fn real_codex_diagnostic_cache_is_visible_and_process_gated() {
        assert_eq!(
            std::env::var("MANGODISK_TEST_REAL_CODEX_DIAGNOSTIC_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_TEST_REAL_CODEX_DIAGNOSTIC_CACHE=1 for the real preview diagnostic"
        );
        let scan = crate::cleanup::CleanupScanService::scan_with_progress(|_| {})
            .expect("the real cleanup preview must succeed");
        let rule = scan
            .rules
            .iter()
            .find(|rule| rule.rule_id == "app.codex-diagnostic-cache")
            .expect("the real Codex diagnostic rule must be present");

        assert!(rule.file_count > 0, "the real pending queue must expose stale reports");
        assert!(rule.bytes > 0, "the real stale reports must have measurable size");
        assert!(rule.requires_app_close, "Codex must close before cleanup");
        assert!(
            !rule.running_processes.is_empty(),
            "the active Codex application must block cleanup"
        );
        let blocked = CleanupService::execute(CleanupRequest {
            rule_ids: vec!["app.codex-diagnostic-cache".to_string()],
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        })
        .expect("the process-gated cleanup must return a structured result");
        let action = blocked
            .actions
            .first()
            .expect("the Codex diagnostic cleanup must report one action");
        assert_eq!(action.status, crate::cleanup::CleanupActionStatus::Blocked);
        assert_eq!(
            action.reason_code,
            Some(crate::cleanup::CleanupActionReason::RunningProcesses)
        );
        assert_eq!(action.released_bytes, 0);
        assert_eq!(action.affected_item_count, 0);
        println!(
            "real_macos_codex_diagnostic_cache files={} bytes={} running_process_count={}",
            rule.file_count,
            rule.bytes,
            rule.running_processes.len()
        );
    }
