mod cleanup_matcher_tests {
    use std::path::PathBuf;

    use mangodisk_platform::{current_platform, Platform};

    use super::*;

    #[test]
    fn preflight_measurement_is_limited_to_preview_and_source_scoped_requests() {
        assert!(!requires_preflight_measurement(false, false));
        assert!(requires_preflight_measurement(true, false));
        assert!(requires_preflight_measurement(false, true));
        assert!(requires_preflight_measurement(true, true));
        assert_eq!(preparation_stage(0), CleanupExecutionStage::Cleaning);
        assert_eq!(preparation_stage(1), CleanupExecutionStage::Validating);
    }

    #[test]
    fn cleanup_service_cancels_the_active_cleanup_operation() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup)
            .expect("the isolated cleanup operation must start");

        CleanupService::cancel();

        assert!(
            operation.ensure_not_cancelled().is_err(),
            "the public cleanup cancellation contract must reach the active operation"
        );
    }

    fn service_custom_rule(root: &Path) -> CustomCleanupRule {
        CustomCleanupRule {
            schema_version: crate::cleanup::CUSTOM_CLEANUP_RULE_SCHEMA_VERSION,
            id: "service-safety-fixture".to_string(),
            name: "Service safety fixture".to_string(),
            roots: vec![root.to_string_lossy().into_owned()],
            name_patterns: vec!["*.tmp".to_string()],
            minimum_bytes: None,
            maximum_bytes: None,
            modified_time: crate::cleanup::CustomCleanupModifiedTime::Any,
            recursive: true,
            remove_empty_directories: false,
        }
    }

    fn custom_cleanup_request(dry_run: bool) -> CleanupRequest {
        CleanupRequest {
            rule_ids: vec!["custom.service-safety-fixture".to_string()],
            source_selections: Vec::new(),
            dry_run,
            project_roots: Vec::new(),
        }
    }

    #[test]
    fn custom_only_cleanup_ignores_project_artifact_exclusions() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-custom-exclusion-{}-{}",
            std::process::id(), now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let excluded = sandbox.join("excluded");
        let included = sandbox.join("included");
        fs::create_dir_all(&excluded).expect("create excluded fixture directory");
        fs::create_dir_all(&included).expect("create included fixture directory");
        fs::write(excluded.join("keep.tmp"), [1_u8; 11]).expect("write excluded fixture");
        fs::write(included.join("remove.tmp"), [2_u8; 7]).expect("write included fixture");
        let rule = service_custom_rule(&sandbox);
        let excluded_paths = vec![excluded.to_string_lossy().into_owned()];

        let scan = crate::cleanup::CleanupScanService::scan_with_custom_rules_and_excluded_paths(
            vec![rule.clone()], false, excluded_paths.clone(), |_| {}
        ).expect("scan with exclusions");
        assert_eq!(scan.rules[0].bytes, 18);
        assert_eq!(scan.rules[0].file_count, 2);

        let result = CleanupService::execute_deep_cleanup_step_with_custom_rules_and_excluded_paths_and_progress(
            custom_cleanup_request(false),
            format!("deep-cleanup-exclusion-{}", now_ms()),
            scan.custom_scan_id.expect("custom scan session"),
            vec![rule], false, excluded_paths, |_| {},
        ).expect("execute with exclusions");
        assert_eq!(result.released_bytes, 18);
        assert!(!excluded.join("keep.tmp").exists());
        assert!(!included.join("remove.tmp").exists());
    }

    #[test]
    fn custom_scan_all_missing_roots_returns_empty_and_restored_roots_scan_again() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-custom-all-missing-{}-{}", std::process::id(), now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let root = sandbox.join("missing/nested");
        let rule = service_custom_rule(&root);
        let scan = crate::cleanup::CleanupScanService::scan_with_custom_rules(
            vec![rule.clone()], false, |_| {}
        ).expect("all missing roots should produce an empty successful scan");
        assert!(scan.rules.is_empty());
        assert_eq!(scan.missing_custom_root_count, 1);
        let json = serde_json::to_value(&scan).expect("serialize missing-root diagnostics");
        assert_eq!(json["missingCustomRootCount"], 1);
        assert_eq!(json["schemaVersion"], "1.9");
        fs::create_dir_all(&root).expect("restore the saved directory");
        fs::write(root.join("cache.tmp"), b"restored").expect("write restored fixture");
        let restored = crate::cleanup::CleanupScanService::scan_with_custom_rules(
            vec![rule], false, |_| {}
        ).expect("restored directories should participate in the next scan");
        assert_eq!(restored.missing_custom_root_count, 0);
        assert_eq!(restored.rules[0].file_count, 1);
    }

    #[test]
    fn custom_scan_skips_deleted_roots_and_does_not_authorize_restored_roots() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-custom-missing-{}-{}", std::process::id(), now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let available = sandbox.join("available");
        let deleted = sandbox.join("deleted");
        fs::create_dir_all(&available).expect("create the available root");
        fs::create_dir_all(&deleted).expect("create the saved root");
        fs::write(available.join("cache.tmp"), b"cache").expect("write the matching file");
        let mut rule = service_custom_rule(&available);
        rule.roots.push(deleted.to_string_lossy().into_owned());
        fs::remove_dir(&deleted).expect("delete the previously saved root");

        let scan = crate::cleanup::CleanupScanService::scan_with_custom_rules(
            vec![rule.clone()], false, |_| {}
        ).expect("a deleted saved root must not block the available root");
        assert_eq!(scan.rules[0].file_count, 1);
        assert_eq!(scan.missing_custom_root_count, 1);

        fs::create_dir_all(&deleted).expect("restore the skipped root after scanning");
        let unscanned = deleted.join("unscanned.tmp");
        fs::write(&unscanned, b"unscanned").expect("write the unscanned file");
        CleanupService::execute_deep_cleanup_step_with_custom_rules_and_progress(
            custom_cleanup_request(false), "custom-missing-root".to_string(),
            scan.custom_scan_id.expect("retain the scan authorization"),
            vec![rule], false, |_| {}
        ).expect("clean the available scanned root");
        assert!(!available.join("cache.tmp").exists());
        assert!(unscanned.exists(), "restored roots require another scan before cleanup");
    }

    #[test]
    fn custom_cleanup_service_previews_then_deletes_only_the_authorized_match() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        HistoryService::clear().expect("the cleanup test history should start empty");
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-service-flow-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let matching = sandbox.join("generated.tmp");
        let retained = sandbox.join("project.txt");
        fs::create_dir_all(&sandbox).expect("the cleanup service fixture should be created");
        fs::write(&matching, b"rebuildable cache")
            .expect("the matching cleanup fixture should be written");
        fs::write(&retained, b"user content")
            .expect("the retained cleanup fixture should be written");
        let rules = vec![service_custom_rule(&sandbox)];
        let scan_id = crate::cleanup::custom_session::publish(rules.clone(), rules.clone(), false, Vec::new(), HashMap::new())
            .expect("the authoritative custom cleanup session should be published");
        let mut preview_progress = Vec::new();

        let preview = CleanupService::execute_deep_cleanup_step_with_custom_rules_and_progress(
            custom_cleanup_request(true),
            "cleanup-service-preview".to_string(),
            scan_id,
            rules.clone(),
            false,
            |progress| preview_progress.push(progress),
        )
        .expect("the custom cleanup preview should succeed");

        assert_eq!(preview.actions.len(), 1);
        assert_eq!(
            preview.actions[0].status,
            crate::cleanup::CleanupActionStatus::Previewed
        );
        assert_eq!(preview.affected_item_count, 0);
        assert!(preview.expected_bytes > 0);
        assert!(matching.exists(), "a preview must not delete the match");
        assert!(retained.exists());
        assert_eq!(
            preview_progress
                .first()
                .expect("preview progress should start")
                .stage,
            CleanupExecutionStage::Validating
        );
        assert_eq!(
            preview_progress
                .last()
                .expect("preview progress should finish")
                .stage,
            CleanupExecutionStage::Finalizing
        );

        let mut execution_progress = Vec::new();
        let result = CleanupService::execute_deep_cleanup_step_with_custom_rules_and_progress(
            custom_cleanup_request(false),
            "cleanup-service-execution".to_string(),
            scan_id,
            rules,
            false,
            |progress| execution_progress.push(progress),
        )
        .expect("the authorized custom cleanup should succeed");

        assert_eq!(result.actions.len(), 1);
        assert_eq!(
            result.actions[0].status,
            crate::cleanup::CleanupActionStatus::Completed
        );
        assert_eq!(result.affected_item_count, 1);
        assert!(!matching.exists());
        assert!(retained.exists(), "an unmatched user file must remain");
        assert!(result.history_saved);
        assert_eq!(
            execution_progress
                .first()
                .expect("execution progress should start")
                .stage,
            CleanupExecutionStage::Cleaning
        );
        assert_eq!(
            execution_progress
                .last()
                .expect("execution progress should finish")
                .stage,
            CleanupExecutionStage::Finalizing
        );
        assert_eq!(
            HistoryService::list()
                .expect("the cleanup test history should load")
                .len(),
            2
        );
        HistoryService::clear().expect("the cleanup test history should be removed");
    }

    #[test]
    fn custom_cleanup_removes_only_scan_authorized_empty_directories() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-empty-authorization-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let child_root = sandbox.join("protected-child");
        let authorized_empty = sandbox.join("authorized-empty");
        let replaced_empty = sandbox.join("replaced-empty");
        let protected_empty = child_root.join("empty");
        let matching_file = sandbox.join("generated.tmp");
        fs::create_dir_all(&authorized_empty).expect("create an authorized empty directory");
        fs::create_dir_all(&replaced_empty).expect("create an empty directory to replace later");
        fs::create_dir_all(&protected_empty).expect("create a nested-rule empty directory");
        fs::write(&matching_file, b"generated cache").expect("write the matching cache file");
        fs::write(child_root.join("keep.cache"), b"child-owned")
            .expect("write a nested-rule fixture");

        let mut parent_rule = service_custom_rule(&sandbox);
        parent_rule.id = "empty-parent".to_string();
        parent_rule.remove_empty_directories = true;
        let child_rule = CustomCleanupRule {
            schema_version: crate::cleanup::CUSTOM_CLEANUP_RULE_SCHEMA_VERSION,
            id: "empty-child".to_string(),
            name: "Nested ownership fixture".to_string(),
            roots: vec![child_root.to_string_lossy().into_owned()],
            name_patterns: vec!["*.cache".to_string()],
            minimum_bytes: None,
            maximum_bytes: None,
            modified_time: crate::cleanup::CustomCleanupModifiedTime::Any,
            recursive: true,
            remove_empty_directories: false,
        };
        let rules = vec![parent_rule, child_rule];
        let scan = crate::cleanup::CleanupScanService::scan_with_custom_rules(
            rules.clone(),
            false,
            |_| {},
        )
        .expect("scan the custom empty-directory rules");
        let scan_id = scan
            .custom_scan_id
            .expect("the custom scan must retain an authoritative session");

        // Reusing the same path string must not authorize a different physical
        // directory, and directories created after the scan are outside the
        // user's reviewed snapshot.
        fs::remove_dir(&replaced_empty).expect("remove the scanned empty directory");
        fs::create_dir(&replaced_empty).expect("replace it with a new physical directory");
        let post_scan_empty = sandbox.join("post-scan-empty");
        fs::create_dir(&post_scan_empty).expect("create a directory after the scan");

        let result = CleanupService::execute_deep_cleanup_step_with_custom_rules_and_progress(
            CleanupRequest {
                rule_ids: vec!["custom.empty-parent".to_string()],
                source_selections: Vec::new(),
                dry_run: false,
                project_roots: Vec::new(),
            },
            "cleanup-empty-authorization".to_string(),
            scan_id,
            rules,
            false,
            |_| {},
        )
        .expect("execute the custom empty-directory cleanup");

        assert_eq!(result.actions.len(), 1);
        assert!(!matching_file.exists(), "the matching file must be removed");
        #[cfg(not(target_os = "linux"))]
        assert!(
            !authorized_empty.exists(),
            "an unchanged empty directory authorized by the scan may be removed"
        );
        #[cfg(target_os = "linux")]
        assert!(
            authorized_empty.exists(),
            "Linux must retain empty directories until non-reusable identity is available"
        );
        assert!(
            replaced_empty.exists(),
            "a same-path replacement must fail the physical identity check"
        );
        assert!(
            post_scan_empty.exists(),
            "a directory created after scanning must remain"
        );
        assert!(
            protected_empty.exists(),
            "a nested rule must retain ownership of its empty subtree"
        );
        assert!(sandbox.exists(), "the selected root must always remain");
    }

    #[test]
    fn cancellation_during_custom_cleanup_validation_preserves_every_file() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-service-cancel-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let matching = sandbox.join("generated.tmp");
        fs::create_dir_all(&sandbox).expect("the cancelled cleanup fixture should be created");
        fs::write(&matching, b"preserve after cancellation")
            .expect("the cancelled cleanup fixture should be written");
        let rules = vec![service_custom_rule(&sandbox)];
        let scan_id = crate::cleanup::custom_session::publish(rules.clone(), rules.clone(), false, Vec::new(), HashMap::new())
            .expect("the cancelled cleanup session should be published");
        let mut cancelled = false;

        let error = CleanupService::execute_deep_cleanup_step_with_custom_rules_and_progress(
            custom_cleanup_request(true),
            "cleanup-service-cancelled".to_string(),
            scan_id,
            rules,
            false,
            |progress| {
                if !cancelled && progress.stage == CleanupExecutionStage::Validating {
                    cancelled = true;
                    CleanupService::cancel();
                }
            },
        )
        .expect_err("cancellation before measurement must stop the cleanup");

        assert!(cancelled);
        assert_eq!(
            error.code(),
            crate::shared::CoreErrorCode::OperationCancelled
        );
        assert!(
            matching.exists(),
            "early cancellation must preserve the match"
        );
    }

    #[test]
    fn cleanup_service_rejects_invalid_requests_before_any_side_effect() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let empty = CleanupRequest {
            rule_ids: Vec::new(),
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        };
        assert!(CleanupService::execute_deep_cleanup_step(
            empty,
            "invalid-empty-selection".to_string()
        )
        .is_err());

        let unknown = CleanupRequest {
            rule_ids: vec!["unknown.rule".to_string()],
            source_selections: Vec::new(),
            dry_run: false,
            project_roots: Vec::new(),
        };
        assert!(
            CleanupService::execute_deep_cleanup_step(unknown.clone(), "   ".to_string()).is_err()
        );
        assert!(CleanupService::execute_deep_cleanup_step(
            CleanupRequest {
                rule_ids: vec!["unknown.rule".to_string(), "unknown.rule".to_string()],
                ..unknown.clone()
            },
            "invalid-duplicate-selection".to_string()
        )
        .is_err());
        assert!(CleanupService::execute(unknown).is_err());

        for rule_ids in [
            Vec::new(),
            vec!["unknown.rule".to_string()],
            vec!["unknown.rule".to_string(), "unknown.rule".to_string()],
        ] {
            assert!(
                CleanupService::close_applications(CleanupApplicationCloseRequest {
                    rule_ids,
                    mode: crate::ApplicationCloseMode::Graceful,
                })
                .is_err()
            );
        }
    }

    #[test]
    fn execution_progress_preserves_stage_order_and_final_totals() {
        let mut snapshots = Vec::new();
        let action = CleanupActionResult {
            rule_id: "system.fixture".to_string(),
            action_kind: CleanupActionKind::Delete,
            status: crate::cleanup::CleanupActionStatus::Completed,
            reason_code: None,
            bytes_expected: 120,
            released_bytes: 96,
            affected_item_count: 2,
            failed_item_count: 0,
            running_processes: Vec::new(),
        };
        {
            let mut reporter =
                ExecutionProgressReporter::new(vec!["system.fixture".to_string()], |progress| {
                    snapshots.push(progress)
                });
            reporter.emit(CleanupExecutionStage::Validating, None);
            reporter.record_validation(3, 120);
            reporter.emit(CleanupExecutionStage::Validating, Some("system.fixture"));
            reporter.finish_validation();
            reporter.emit(CleanupExecutionStage::Cleaning, Some(&action.rule_id));
            reporter.record_action(&action);
            reporter.emit(CleanupExecutionStage::Cleaning, Some(&action.rule_id));
            reporter.emit(CleanupExecutionStage::Finalizing, None);
        }

        let stages = snapshots
            .iter()
            .map(|progress| progress.stage)
            .collect::<Vec<_>>();
        assert_eq!(
            stages,
            vec![
                CleanupExecutionStage::Validating,
                CleanupExecutionStage::Validating,
                CleanupExecutionStage::Cleaning,
                CleanupExecutionStage::Cleaning,
                CleanupExecutionStage::Finalizing,
            ]
        );
        assert!(snapshots
            .iter()
            .all(|progress| progress.completed_rule_count <= progress.total_rule_count));
        assert!(snapshots
            .iter()
            .all(|progress| progress.planned_rule_ids.as_slice() == ["system.fixture"]));
        let final_snapshot = snapshots.last().expect("final progress must be emitted");
        let serialized = serde_json::to_value(final_snapshot)
            .expect("cleanup execution progress must serialize for desktop events");
        assert_eq!(serialized["plannedRuleIds"][0], "system.fixture");
        assert_eq!(
            final_snapshot.affected_item_count,
            action.affected_item_count
        );
        assert_eq!(final_snapshot.released_bytes, action.released_bytes);
    }

    #[test]
    fn execution_progress_reports_live_items_and_completed_rule_results() {
        let mut snapshots = Vec::new();
        let action = CleanupActionResult {
            rule_id: "system.fixture".to_string(),
            action_kind: CleanupActionKind::Delete,
            status: crate::cleanup::CleanupActionStatus::Completed,
            reason_code: None,
            bytes_expected: 120,
            released_bytes: 64,
            affected_item_count: 1,
            failed_item_count: 0,
            running_processes: Vec::new(),
        };
        {
            let mut reporter =
                ExecutionProgressReporter::new(vec![action.rule_id.clone()], |progress| {
                    snapshots.push(progress)
                });
            reporter.begin_rule();
            reporter.record_item(
                &action.rule_id,
                Path::new("fixture/cache.tmp"),
                &DeleteStats {
                    matched_bytes: 64,
                    deleted_bytes: 64,
                    affected_item_count: 1,
                    failed_item_count: 0,
                    removed_empty_directory_count: 0,
            logged_failure_count: 0,
                },
            );
            reporter.record_action(&action);
            reporter.emit(CleanupExecutionStage::Cleaning, Some(&action.rule_id));
        }

        let live_snapshot = snapshots
            .first()
            .expect("the first deleted item must produce live progress");
        assert_eq!(
            live_snapshot.current_item_path.as_deref(),
            Some("fixture/cache.tmp")
        );
        assert_eq!(live_snapshot.current_rule_affected_item_count, 1);
        assert_eq!(live_snapshot.current_rule_released_bytes, 64);
        assert_eq!(live_snapshot.affected_item_count, 1);
        assert_eq!(live_snapshot.released_bytes, 64);

        let completed_snapshot = snapshots
            .last()
            .expect("the completed rule must produce a summary");
        assert_eq!(completed_snapshot.current_item_path, None);
        assert_eq!(completed_snapshot.completed_rule_count, 1);
        assert_eq!(completed_snapshot.completed_rule_results.len(), 1);
        assert_eq!(
            completed_snapshot.completed_rule_results[0].rule_id,
            action.rule_id
        );
        assert_eq!(
            completed_snapshot.completed_rule_results[0].status,
            action.status
        );
        assert_eq!(
            completed_snapshot.completed_rule_results[0].affected_item_count,
            action.affected_item_count
        );
        assert_eq!(
            completed_snapshot.completed_rule_results[0].released_bytes,
            action.released_bytes
        );
    }

    #[test]
    fn whole_rule_cleanup_derives_expected_bytes_during_single_pass() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-single-pass-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let cleanup_root = sandbox.join("cache");
        let cache_file = cleanup_root.join("generated.tmp");
        let cache_bytes = b"single-pass cleanup fixture";
        fs::create_dir_all(&cleanup_root).expect("the isolated cleanup root must be created");
        fs::write(&cache_file, cache_bytes).expect("the cleanup fixture must be written");
        let plan = compile_scan_plan(
            vec![CompiledRule::fixture(
                "system.single-pass-fixture",
                cleanup_root,
                crate::cleanup::CleanupCategory::System,
                MatcherSpec::ExtensionIn(vec!["tmp".to_string()]),
            )],
            &[true],
            &[],
        )
        .expect("the isolated rule must compile");
        let process_snapshot = ProcessSnapshot::default();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup)
            .expect("the isolated cleanup operation must start");

        let action = execute_rule(
            &plan.rules[0],
            0,
            None,
            &RuleExecutionContext {
                ownership_plan: &plan,
                process_snapshot: &process_snapshot,
                source_scope: None,
                empty_directory_authorizations: None,
                operation: &operation,
                dry_run: false,
            },
            &mut |_, _| {},
        );

        operation.complete();
        assert_eq!(
            action.status,
            crate::cleanup::CleanupActionStatus::Completed
        );
        assert_eq!(action.bytes_expected, cache_bytes.len() as u64);
        assert_eq!(action.released_bytes, cache_bytes.len() as u64);
        assert_eq!(action.affected_item_count, 1);
        assert!(!cache_file.exists());
    }

    #[test]
    fn custom_cleanup_deletes_only_matching_files() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-custom-cleanup-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let cleanup_root = sandbox.join("downloads");
        let nested_root = cleanup_root.join("nested");
        let matching_file = cleanup_root.join("download.tmp");
        let nested_matching_file = nested_root.join("nested.tmp");
        let retained_file = cleanup_root.join("keep.txt");
        let retained_directory = cleanup_root.join("folder.tmp");
        fs::create_dir_all(&nested_root).expect("create the nested custom cleanup fixture");
        fs::create_dir_all(&retained_directory)
            .expect("create the directory that resembles a matching file");
        fs::write(&matching_file, b"temporary download")
            .expect("write the matching custom cleanup fixture");
        fs::write(&nested_matching_file, b"nested temporary download")
            .expect("write the nested matching custom cleanup fixture");
        fs::write(&retained_file, b"retain this file")
            .expect("write the retained custom cleanup fixture");
        let rules =
            crate::cleanup::rules::compile_custom_rules(&[crate::cleanup::CustomCleanupRule {
                schema_version: crate::cleanup::CUSTOM_CLEANUP_RULE_SCHEMA_VERSION,
                id: "fixture-rule".to_string(),
                name: "Fixture files".to_string(),
                roots: vec![cleanup_root.to_string_lossy().into_owned()],
                name_patterns: vec!["*.tmp".to_string()],
                minimum_bytes: Some(0),
                maximum_bytes: None,
                modified_time: crate::cleanup::CustomCleanupModifiedTime::Any,
                recursive: true,
                remove_empty_directories: false,
            }])
            .expect("compile the custom cleanup fixture");
        let plan = compile_scan_plan(rules, &[true], &[])
            .expect("compile the custom cleanup ownership plan");
        let process_snapshot = ProcessSnapshot::default();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup)
            .expect("start the isolated custom cleanup operation");

        let action = execute_rule(
            &plan.rules[0],
            0,
            None,
            &RuleExecutionContext {
                ownership_plan: &plan,
                process_snapshot: &process_snapshot,
                source_scope: None,
                empty_directory_authorizations: None,
                operation: &operation,
                dry_run: false,
            },
            &mut |_, _| {},
        );

        operation.complete();
        assert_eq!(
            action.status,
            crate::cleanup::CleanupActionStatus::Completed
        );
        assert_eq!(action.affected_item_count, 2);
        assert!(!matching_file.exists());
        assert!(!nested_matching_file.exists());
        assert!(retained_file.exists());
        assert!(retained_directory.exists());
    }

    #[test]
    fn custom_cleanup_source_selection_does_not_expand_to_sibling_directories() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-custom-source-scope-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let cleanup_root = sandbox.join("workspace");
        let selected_source = cleanup_root.join("selected");
        let retained_source = cleanup_root.join("retained");
        let selected_file = selected_source.join("selected.tmp");
        let retained_file = retained_source.join("retained.tmp");
        fs::create_dir_all(&selected_source).expect("create the selected source fixture");
        fs::create_dir_all(&retained_source).expect("create the retained source fixture");
        fs::write(&selected_file, b"selected").expect("write the selected source fixture");
        fs::write(&retained_file, b"retained").expect("write the retained source fixture");
        let rules =
            crate::cleanup::rules::compile_custom_rules(&[crate::cleanup::CustomCleanupRule {
                schema_version: crate::cleanup::CUSTOM_CLEANUP_RULE_SCHEMA_VERSION,
                id: "source-scope-rule".to_string(),
                name: "Source scope".to_string(),
                roots: vec![
                    selected_source.to_string_lossy().into_owned(),
                    cleanup_root.to_string_lossy().into_owned(),
                ],
                name_patterns: vec!["*.tmp".to_string()],
                minimum_bytes: None,
                maximum_bytes: None,
                modified_time: crate::cleanup::CustomCleanupModifiedTime::Any,
                recursive: true,
                remove_empty_directories: false,
            }])
            .expect("compile the source-scoped custom rule");
        let plan = compile_scan_plan(rules, &[true], &[])
            .expect("compile the source-scoped ownership plan");
        let selected_canonical_source = plan.rules[0].roots[0].join("selected");
        let selected_rule_ids = HashSet::from(["custom.source-scope-rule".to_string()]);
        let source_policy = SourceSelectionPolicy::from_request(
            &selected_rule_ids,
            &[crate::cleanup::CleanupSourceSelection {
                rule_id: "custom.source-scope-rule".to_string(),
                mode: crate::cleanup::CleanupSourceSelectionMode::Include,
                paths: vec![current_platform().display_path(&selected_canonical_source)],
            }],
        )
        .expect("build the selected source policy");
        let process_snapshot = ProcessSnapshot::default();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup)
            .expect("start the source-scoped cleanup operation");

        let action = execute_rule(
            &plan.rules[0],
            0,
            None,
            &RuleExecutionContext {
                ownership_plan: &plan,
                process_snapshot: &process_snapshot,
                source_scope: source_policy.scope("custom.source-scope-rule"),
                empty_directory_authorizations: None,
                operation: &operation,
                dry_run: false,
            },
            &mut |_, _| {},
        );

        operation.complete();
        assert_eq!(action.affected_item_count, 1);
        assert!(!selected_file.exists());
        assert!(retained_file.exists());
    }

    #[test]
    fn complete_root_cleanup_reduces_per_file_deletion_transactions() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-whole-root-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let cleanup_root = sandbox.join("cache");
        let nested = cleanup_root.join("many-small-files");
        let generic_root = sandbox.join("generic-cache");
        let generic_nested = generic_root.join("many-small-files");
        let generic_empty = generic_root.join("empty-scaffold/nested");
        fs::create_dir_all(&nested).expect("the isolated cleanup root must be created");
        fs::create_dir_all(&generic_nested).expect("the generic comparison root must be created");
        fs::create_dir_all(&generic_empty).expect("create the empty comparison directory");
        let generic_root_identity = physical_path_identity_snapshot(&generic_root)
            .expect("capture the comparison root identity");
        let generic_root_permissions = fs::metadata(&generic_root)
            .expect("read the comparison root metadata")
            .permissions();
        let generic_empty_identity = physical_path_identity_snapshot(&generic_empty)
            .expect("capture the empty comparison directory identity");
        let file_count = 128_u64;
        for index in 0..file_count {
            fs::write(nested.join(format!("{index}.cache")), b"cache")
                .expect("the small cache fixture must be written");
            fs::write(generic_nested.join(format!("{index}.cache")), b"cache")
                .expect("the comparison cache fixture must be written");
        }
        let whole_root_plan = compile_scan_plan(
            vec![CompiledRule::whole_root_fixture(
                "development.whole-root-fixture",
                cleanup_root.clone(),
                crate::cleanup::CleanupCategory::Development,
            )],
            &[true],
            &[],
        )
        .expect("the isolated whole-root rule must compile");
        let generic_plan = compile_scan_plan(
            vec![CompiledRule::fixture(
                "development.generic-fixture",
                generic_root.clone(),
                crate::cleanup::CleanupCategory::Development,
                MatcherSpec::All,
            )],
            &[true],
            &[],
        )
        .expect("the generic comparison rule must compile");
        let process_snapshot = ProcessSnapshot::default();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup)
            .expect("the isolated cleanup operation must start");
        let mut reported_paths = Vec::new();
        let mut generic_report_count = 0_u64;

        let generic_action = execute_rule(
            &generic_plan.rules[0],
            0,
            None,
            &RuleExecutionContext {
                ownership_plan: &generic_plan,
                process_snapshot: &process_snapshot,
                source_scope: None,
                empty_directory_authorizations: None,
                operation: &operation,
                dry_run: false,
            },
            &mut |_, _| generic_report_count = generic_report_count.saturating_add(1),
        );

        let action = execute_rule(
            &whole_root_plan.rules[0],
            0,
            None,
            &RuleExecutionContext {
                ownership_plan: &whole_root_plan,
                process_snapshot: &process_snapshot,
                source_scope: None,
                empty_directory_authorizations: None,
                operation: &operation,
                dry_run: false,
            },
            &mut |path, _| reported_paths.push(path.to_path_buf()),
        );

        operation.complete();
        assert_eq!(
            generic_action.status,
            crate::cleanup::CleanupActionStatus::Completed
        );
        assert_eq!(generic_report_count, 1);
        assert!(
            generic_root.exists(),
            "content cleanup must retain its root"
        );
        assert_eq!(
            physical_path_identity_snapshot(&generic_root)
                .expect("read the retained comparison root identity"),
            generic_root_identity,
            "content cleanup must retain the physical root directory"
        );
        assert_eq!(
            fs::metadata(&generic_root)
                .expect("read the retained root metadata")
                .permissions(),
            generic_root_permissions,
            "content cleanup must retain the root permissions"
        );
        assert_eq!(
            fs::read_dir(&generic_root)
                .expect("read the retained cache root")
                .count(),
            1,
            "content cleanup must retain only the preexisting empty scaffold"
        );
        assert_eq!(
            physical_path_identity_snapshot(&generic_empty)
                .expect("read the retained empty directory identity"),
            generic_empty_identity
        );
        assert_eq!(
            action.status,
            crate::cleanup::CleanupActionStatus::Completed
        );
        assert_eq!(action.bytes_expected, file_count * 5);
        assert_eq!(action.released_bytes, file_count * 5);
        assert_eq!(action.affected_item_count, file_count);
        assert_eq!(reported_paths, vec![cleanup_root.clone()]);
        assert!(
            !cleanup_root.exists(),
            "the complete cache root must be removed"
        );
    }

    #[test]
    fn source_scoped_cleanup_keeps_unselected_complete_root_content() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-scoped-root-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let cleanup_root = sandbox.join("cache");
        let selected_source = cleanup_root.join("selected");
        let retained_source = cleanup_root.join("retained");
        let retained_empty = cleanup_root.join("retained-empty/nested");
        let selected_file = selected_source.join("selected.cache");
        let retained_file = retained_source.join("retained.cache");
        fs::create_dir_all(&selected_source).expect("create the selected cache source");
        fs::create_dir_all(&retained_source).expect("create the retained cache source");
        fs::create_dir_all(&retained_empty).expect("create the retained empty source");
        fs::write(&selected_file, b"selected").expect("write the selected cache fixture");
        fs::write(&retained_file, b"retained").expect("write the retained cache fixture");
        let rule_id = "development.scoped-complete-root";
        let mut rule = CompiledRule::fixture(
            rule_id,
            cleanup_root.clone(),
            crate::cleanup::CleanupCategory::Development,
            MatcherSpec::All,
        );
        rule.remove_empty_directories = true;
        let plan = compile_scan_plan(vec![rule], &[true], &[])
            .expect("compile the source-scoped cleanup plan");
        let policy = SourceSelectionPolicy::from_request(
            &HashSet::from([rule_id.to_string()]),
            &[crate::cleanup::CleanupSourceSelection {
                rule_id: rule_id.to_string(),
                mode: crate::cleanup::CleanupSourceSelectionMode::Include,
                paths: vec![selected_source.to_string_lossy().into_owned()],
            }],
        )
        .expect("compile the source selection");
        let process_snapshot = ProcessSnapshot::default();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup)
            .expect("start the isolated cleanup operation");

        let action = execute_rule(
            &plan.rules[0],
            0,
            None,
            &RuleExecutionContext {
                ownership_plan: &plan,
                process_snapshot: &process_snapshot,
                source_scope: policy.scope(rule_id),
                empty_directory_authorizations: None,
                operation: &operation,
                dry_run: false,
            },
            &mut |_, _| {},
        );

        operation.complete();
        assert_eq!(
            action.status,
            crate::cleanup::CleanupActionStatus::Completed
        );
        assert!(!selected_file.exists());
        assert!(retained_file.exists());
        assert!(retained_empty.exists());
        assert!(cleanup_root.exists());
    }

    /// Compares the previous per-entry traversal with both bulk strategies.
    ///
    /// The benchmark is ignored by default to keep normal tests independent of
    /// disk variance. The file and bucket environment variables shape the
    /// workload, while `MANGODISK_CLEANUP_BENCHMARK_PARALLEL_FIRST=1` reverses
    /// the bulk-strategy order to expose cache bias. Output contains only
    /// counts and timings, never private paths.
    #[test]
    #[ignore = "filesystem performance benchmark"]
    fn benchmark_complete_root_cleanup_strategies() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let file_count = std::env::var("MANGODISK_CLEANUP_BENCHMARK_FILE_COUNT")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|count| *count > 0)
            .unwrap_or(5_000);
        let bucket_count = std::env::var("MANGODISK_CLEANUP_BENCHMARK_BUCKET_COUNT")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|count| *count > 0)
            .unwrap_or(128)
            .min(file_count);
        let available_parallelism = std::thread::available_parallelism().map_or(1, usize::from);
        let parallel_first =
            std::env::var("MANGODISK_CLEANUP_BENCHMARK_PARALLEL_FIRST").as_deref() == Ok("1");
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-whole-root-benchmark-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let per_entry_root = sandbox.join("per-entry-cache");
        let serial_contents_root = sandbox.join("serial-contents-cache");
        let parallel_contents_root = sandbox.join("parallel-contents-cache");
        let whole_root = sandbox.join("whole-root-cache");
        fs::create_dir_all(&per_entry_root).expect("create the per-entry benchmark root");
        fs::create_dir_all(&serial_contents_root)
            .expect("create the serial contents benchmark root");
        fs::create_dir_all(&parallel_contents_root)
            .expect("create the parallel contents benchmark root");
        fs::create_dir_all(&whole_root).expect("create the whole-root benchmark root");
        let payload = [b'x'; 64];
        for index in 0..file_count {
            let bucket = format!("{:03}", index % bucket_count);
            let per_entry_bucket = per_entry_root.join(&bucket);
            let serial_contents_bucket = serial_contents_root.join(&bucket);
            let parallel_contents_bucket = parallel_contents_root.join(&bucket);
            let whole_bucket = whole_root.join(&bucket);
            fs::create_dir_all(&per_entry_bucket).expect("create the per-entry benchmark bucket");
            fs::create_dir_all(&serial_contents_bucket)
                .expect("create the serial contents benchmark bucket");
            fs::create_dir_all(&parallel_contents_bucket)
                .expect("create the parallel contents benchmark bucket");
            fs::create_dir_all(&whole_bucket).expect("create the whole-root benchmark bucket");
            let name = format!("{index:08}.cache");
            fs::write(per_entry_bucket.join(&name), payload)
                .expect("write the per-entry benchmark file");
            fs::write(serial_contents_bucket.join(&name), payload)
                .expect("write the serial contents benchmark file");
            fs::write(parallel_contents_bucket.join(&name), payload)
                .expect("write the parallel contents benchmark file");
            fs::write(whole_bucket.join(name), payload)
                .expect("write the whole-root benchmark file");
        }
        for bucket in 0..bucket_count {
            let bucket = format!("{bucket:03}");
            fs::create_dir_all(per_entry_root.join(&bucket).join("empty-scaffold"))
                .expect("create the per-entry empty scaffold");
            fs::create_dir_all(serial_contents_root.join(&bucket).join("empty-scaffold"))
                .expect("create the serial contents empty scaffold");
            fs::create_dir_all(parallel_contents_root.join(&bucket).join("empty-scaffold"))
                .expect("create the parallel contents empty scaffold");
            fs::create_dir_all(whole_root.join(&bucket).join("empty-scaffold"))
                .expect("create the whole-root empty scaffold");
        }
        let parallel_contents_plan = compile_scan_plan(
            vec![CompiledRule::fixture(
                "development.parallel-contents-benchmark",
                parallel_contents_root,
                crate::cleanup::CleanupCategory::Development,
                MatcherSpec::All,
            )],
            &[true],
            &[],
        )
        .expect("compile the parallel contents benchmark plan");
        let whole_root_plan = compile_scan_plan(
            vec![CompiledRule::whole_root_fixture(
                "development.whole-root-benchmark",
                whole_root,
                crate::cleanup::CleanupCategory::Development,
            )],
            &[true],
            &[],
        )
        .expect("compile the whole-root benchmark plan");
        let process_snapshot = ProcessSnapshot::default();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup)
            .expect("start the benchmark cleanup operation");

        let per_entry_canonical = validate_rule_root(&per_entry_root, &MatcherSpec::All)
            .expect("validate the per-entry benchmark root");
        let mut per_entry_stats = DeleteStats::default();
        let per_entry_started = Instant::now();
        delete_root_contents_with_progress(
            &per_entry_root,
            &per_entry_canonical,
            &MatcherSpec::All,
            DeleteRootContentsPolicy {
                owns_path: &|_, _| true,
                is_cancelled: &|| false,
                bulk_complete_directories: false,
                authorize_empty_directory: &|_, _| false,
            },
            &mut per_entry_stats,
            &mut |_, _| {},
        );
        let per_entry_ms = per_entry_started.elapsed().as_secs_f64() * 1_000.0;

        let run_serial_contents = || {
            let started = Instant::now();
            let mut file_count = 0_u64;
            for entry in fs::read_dir(&serial_contents_root)
                .expect("read the serial contents benchmark root")
            {
                let path = entry
                    .expect("read a serial contents benchmark entry")
                    .path();
                let prepared = prepare_path_for_permanent_delete(&path)
                    .expect("prepare a serial contents benchmark directory");
                let outcome = delete_directory_contents_permanently_with_cancellation_serial(
                    prepared,
                    &|| false,
                )
                .expect("delete a serial contents benchmark directory");
                file_count = file_count.saturating_add(outcome.affected_item_count());
            }
            (file_count, started.elapsed().as_secs_f64() * 1_000.0)
        };
        let run_parallel_contents = || {
            let started = Instant::now();
            let action = execute_rule(
                &parallel_contents_plan.rules[0],
                0,
                None,
                &RuleExecutionContext {
                    ownership_plan: &parallel_contents_plan,
                    process_snapshot: &process_snapshot,
                    source_scope: None,
                    empty_directory_authorizations: None,
                    operation: &operation,
                    dry_run: false,
                },
                &mut |_, _| {},
            );
            (action, started.elapsed().as_secs_f64() * 1_000.0)
        };
        let (
            (serial_contents_file_count, serial_contents_ms),
            (parallel_contents_action, parallel_contents_ms),
        ) = if parallel_first {
            let parallel = run_parallel_contents();
            let serial = run_serial_contents();
            (serial, parallel)
        } else {
            (run_serial_contents(), run_parallel_contents())
        };

        let whole_started = Instant::now();
        let whole_action = execute_rule(
            &whole_root_plan.rules[0],
            0,
            None,
            &RuleExecutionContext {
                ownership_plan: &whole_root_plan,
                process_snapshot: &process_snapshot,
                source_scope: None,
                empty_directory_authorizations: None,
                operation: &operation,
                dry_run: false,
            },
            &mut |_, _| {},
        );
        let whole_ms = whole_started.elapsed().as_secs_f64() * 1_000.0;
        operation.complete();

        assert_eq!(
            parallel_contents_action.status,
            crate::cleanup::CleanupActionStatus::Completed
        );
        assert_eq!(
            whole_action.status,
            crate::cleanup::CleanupActionStatus::Completed
        );
        assert_eq!(per_entry_stats.affected_item_count, file_count);
        assert_eq!(serial_contents_file_count, file_count);
        assert_eq!(parallel_contents_action.affected_item_count, file_count);
        assert_eq!(whole_action.affected_item_count, file_count);
        println!(
            "cleanup_complete_root_benchmark file_count={file_count} bucket_count={bucket_count} available_parallelism={available_parallelism} parallel_first={parallel_first} per_entry_ms={per_entry_ms:.2} serial_contents_ms={serial_contents_ms:.2} parallel_contents_ms={parallel_contents_ms:.2} whole_root_ms={whole_ms:.2} serial_speedup={:.2} parallel_speedup={:.2} incremental_speedup={:.2}",
            per_entry_ms / serial_contents_ms.max(0.01),
            per_entry_ms / parallel_contents_ms.max(0.01),
            serial_contents_ms / parallel_contents_ms.max(0.01)
        );
    }

    #[test]
    fn complete_root_cleanup_falls_back_for_nested_rule_ownership() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-whole-root-fallback-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let cleanup_root = sandbox.join("cache");
        let child_root = cleanup_root.join("owned-by-child");
        let parent_file = cleanup_root.join("parent.cache");
        let child_file = child_root.join("child.cache");
        fs::create_dir_all(&child_root).expect("the nested rule root must be created");
        fs::write(&parent_file, b"parent cache").expect("the parent fixture must be written");
        fs::write(&child_file, b"child cache").expect("the child fixture must be written");
        let plan = compile_scan_plan(
            vec![
                CompiledRule::fixture(
                    "development.parent-fixture",
                    cleanup_root,
                    crate::cleanup::CleanupCategory::Development,
                    MatcherSpec::All,
                ),
                CompiledRule::fixture(
                    "development.child-fixture",
                    child_root,
                    crate::cleanup::CleanupCategory::Development,
                    MatcherSpec::All,
                ),
            ],
            &[true, true],
            &[],
        )
        .expect("nested ownership must compile");
        let process_snapshot = ProcessSnapshot::default();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup)
            .expect("the isolated cleanup operation must start");

        let action = execute_rule(
            &plan.rules[0],
            0,
            None,
            &RuleExecutionContext {
                ownership_plan: &plan,
                process_snapshot: &process_snapshot,
                source_scope: None,
                empty_directory_authorizations: None,
                operation: &operation,
                dry_run: false,
            },
            &mut |_, _| {},
        );

        operation.complete();
        assert_eq!(
            action.status,
            crate::cleanup::CleanupActionStatus::Completed
        );
        assert!(
            !parent_file.exists(),
            "the parent-owned cache must be removed"
        );
        assert!(
            child_file.exists(),
            "fallback traversal must preserve a nested rule boundary"
        );
    }

    struct DirectoryCleanup(PathBuf);

    impl Drop for DirectoryCleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn filtered_cleanup_preserves_unmatched_empty_directories() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-matcher-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let cleanup_root = sandbox.join("cache");
        let unmatched_empty = cleanup_root.join("user-created-empty");
        let matched_directory = cleanup_root.join("generated");
        let matched_file = matched_directory.join("cache.tmp");
        fs::create_dir_all(&unmatched_empty).expect("the unmatched directory must be created");
        fs::create_dir_all(&matched_directory).expect("the matched directory must be created");
        fs::write(&matched_file, b"temporary cache").expect("the matched file must be written");
        let canonical_root = validate_rule_root(&cleanup_root, &MatcherSpec::All)
            .expect("the isolated root must be safe");
        let mut stats = DeleteStats {
            matched_bytes: 0,
            deleted_bytes: 0,
            affected_item_count: 0,
            failed_item_count: 0,
            removed_empty_directory_count: 0,
            logged_failure_count: 0,
        };
        let mut item_progress = Vec::new();

        delete_root_contents_with_progress(
            &cleanup_root,
            &canonical_root,
            &MatcherSpec::ExtensionIn(vec!["tmp".to_string()]),
            DeleteRootContentsPolicy {
                owns_path: &|_, _| true,
                is_cancelled: &|| false,
                bulk_complete_directories: false,
                authorize_empty_directory: &|_, _| false,
            },
            &mut stats,
            &mut |path, stats| {
                item_progress.push((
                    path.to_path_buf(),
                    stats.affected_item_count,
                    stats.deleted_bytes,
                ));
            },
        );

        assert!(
            !matched_file.exists(),
            "the matched cache file must be deleted"
        );
        assert!(
            !matched_directory.exists(),
            "a directory emptied by this operation may be pruned"
        );
        assert!(
            unmatched_empty.exists(),
            "a pre-existing empty directory is outside the matcher scope"
        );
        assert_eq!(stats.affected_item_count, 1);
        assert_eq!(stats.matched_bytes, b"temporary cache".len() as u64);
        assert_eq!(stats.failed_item_count, 0);
        assert_eq!(item_progress.len(), 1);
        assert_eq!(item_progress[0].0, matched_file);
        assert_eq!(item_progress[0].1, 1);
        assert_eq!(item_progress[0].2, b"temporary cache".len() as u64);
    }

    #[test]
    fn custom_cleanup_can_remove_empty_descendants_without_removing_the_selected_root() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-empty-folders-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let cleanup_root = sandbox.join("cache");
        let preexisting_empty = cleanup_root.join("empty/nested");
        let matched_directory = cleanup_root.join("generated/deep");
        let matched_file = matched_directory.join("cache.tmp");
        let retained_file = cleanup_root.join("keep/notes.txt");
        fs::create_dir_all(&preexisting_empty).expect("create the pre-existing empty subtree");
        fs::create_dir_all(&matched_directory).expect("create the matched subtree");
        fs::create_dir_all(retained_file.parent().expect("retained file has a parent"))
            .expect("create the retained subtree");
        fs::write(&matched_file, b"temporary cache").expect("write the matching file");
        fs::write(&retained_file, b"keep").expect("write the retained file");
        let canonical_root = validate_rule_root(&cleanup_root, &MatcherSpec::All)
            .expect("the isolated root must be safe");
        let authorized_identity = physical_path_identity_snapshot(&preexisting_empty)
            .expect("capture the scan-authorized empty directory identity");
        let authorize_empty_directory =
            |path: &Path, identity| path == preexisting_empty && identity == authorized_identity;
        let mut stats = DeleteStats::default();

        delete_root_contents_with_progress(
            &cleanup_root,
            &canonical_root,
            &MatcherSpec::ExtensionIn(vec!["tmp".to_string()]),
            DeleteRootContentsPolicy {
                owns_path: &|_, _| true,
                is_cancelled: &|| false,
                bulk_complete_directories: false,
                authorize_empty_directory: &authorize_empty_directory,
            },
            &mut stats,
            &mut |_, _| {},
        );

        assert!(
            cleanup_root.exists(),
            "the user-selected root must be retained"
        );
        assert!(
            !preexisting_empty.exists(),
            "pre-existing empty descendants may be removed"
        );
        assert!(
            !matched_directory.exists(),
            "directories emptied by cleanup must be removed"
        );
        assert!(
            retained_file.exists(),
            "unmatched files and their parent must remain"
        );
        assert_eq!(stats.affected_item_count, 1);
        assert_eq!(stats.failed_item_count, 0);
        assert_eq!(stats.removed_empty_directory_count, 4);
    }

    #[test]
    fn cancelled_cleanup_stops_before_removing_the_next_entry() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-cancel-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let cleanup_root = sandbox.join("cache");
        let cache_file = cleanup_root.join("cache.tmp");
        fs::create_dir_all(&cleanup_root).expect("the isolated cleanup root must be created");
        fs::write(&cache_file, b"temporary cache").expect("the cache fixture must be written");
        let canonical_root = validate_rule_root(&cleanup_root, &MatcherSpec::All)
            .expect("the isolated root must be safe");
        let mut stats = DeleteStats {
            matched_bytes: 0,
            deleted_bytes: 0,
            affected_item_count: 0,
            failed_item_count: 0,
            removed_empty_directory_count: 0,
            logged_failure_count: 0,
        };

        delete_root_contents(
            &cleanup_root,
            &canonical_root,
            &MatcherSpec::All,
            &|_, _| true,
            &|| true,
            &mut stats,
        );

        assert!(
            cache_file.exists(),
            "a cancellation observed before traversal must preserve the file"
        );
        assert_eq!(stats.affected_item_count, 0);
        assert_eq!(stats.failed_item_count, 1);
    }

    #[test]
    fn overlapping_cleanup_respects_unselected_child_rule_ownership() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let sandbox = std::env::temp_dir().join(format!(
            "mangodisk-cleanup-ownership-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(sandbox.clone());
        let parent_root = sandbox.join("cache");
        let child_root = parent_root.join("specialized");
        let parent_file = parent_root.join("general.bin");
        let child_file = child_root.join("owned.tmp");
        fs::create_dir_all(&child_root).expect("the overlapping roots must be created");
        fs::write(&parent_file, b"general cache").expect("the parent-owned file must be written");
        fs::write(&child_file, b"specialized cache").expect("the child-owned file must be written");

        let plan = compile_scan_plan(
            vec![
                CompiledRule::fixture(
                    "system.parent",
                    parent_root.clone(),
                    crate::cleanup::CleanupCategory::System,
                    MatcherSpec::All,
                ),
                CompiledRule::fixture(
                    "application.child",
                    child_root.clone(),
                    crate::cleanup::CleanupCategory::Application,
                    MatcherSpec::ExtensionIn(vec!["tmp".to_string()]),
                ),
            ],
            &[true, true],
            &[],
        )
        .expect("overlapping cleanup rules must produce stable ownership");

        let canonical_parent = validate_rule_root(&parent_root, &MatcherSpec::All)
            .expect("the parent root must be safe");
        let mut parent_stats = DeleteStats {
            matched_bytes: 0,
            deleted_bytes: 0,
            affected_item_count: 0,
            failed_item_count: 0,
            removed_empty_directory_count: 0,
            logged_failure_count: 0,
        };
        delete_root_contents(
            &parent_root,
            &canonical_parent,
            &MatcherSpec::All,
            &|path, metadata| plan.rule_owns_path(0, path, metadata),
            &|| false,
            &mut parent_stats,
        );

        assert!(
            !parent_file.exists(),
            "the parent-owned file must be deleted"
        );
        assert!(
            child_file.exists(),
            "the unselected child rule must retain ownership of its file"
        );
        assert_eq!(parent_stats.affected_item_count, 1);
        assert_eq!(parent_stats.failed_item_count, 0);

        let canonical_child = validate_rule_root(
            &child_root,
            &MatcherSpec::ExtensionIn(vec!["tmp".to_string()]),
        )
        .expect("the child rule root must be safe");
        let mut child_stats = DeleteStats {
            matched_bytes: 0,
            deleted_bytes: 0,
            affected_item_count: 0,
            failed_item_count: 0,
            removed_empty_directory_count: 0,
            logged_failure_count: 0,
        };
        delete_root_contents(
            &child_root,
            &canonical_child,
            &MatcherSpec::ExtensionIn(vec!["tmp".to_string()]),
            &|path, metadata| plan.rule_owns_path(1, path, metadata),
            &|| false,
            &mut child_stats,
        );

        assert!(
            !child_file.exists(),
            "the selected child rule may delete its file"
        );
        assert_eq!(child_stats.affected_item_count, 1);
        assert_eq!(child_stats.failed_item_count, 0);
    }
}
