use super::*;

#[test]
fn brave_origin_cleanup_preserves_regular_brave_and_account_state() {
    let _guard = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().unwrap();
    let mut candidates = BTreeMap::new();
    let mut items = Vec::new();
    for source in ["brave", "brave-origin"] {
        let root = fixture.path().join(source);
        fs::create_dir(&root).unwrap();
        let history = root.join("History");
        let connection = rusqlite::Connection::open(&history).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT);
             CREATE TABLE visits(id INTEGER PRIMARY KEY, url INTEGER, visit_time INTEGER);
             INSERT INTO urls VALUES (1, 'https://fixture.invalid');
             INSERT INTO visits VALUES (1, 1, 1);",
            )
            .unwrap();
        drop(connection);
        for file in ["Bookmarks", "Login Data", "Cookies", "Preferences"] {
            fs::write(root.join(file), b"preserved state").unwrap();
        }
        let caches = ["Cache", "DawnGraphiteCache", "DawnWebGPUCache"].map(|name| root.join(name));
        for cache in &caches {
            fs::create_dir(cache).unwrap();
            fs::write(cache.join("payload"), b"regenerable cache").unwrap();
        }
        add_directory_candidate(
            "origin-isolation",
            source,
            source,
            &format!("{source}:Default"),
            "Default",
            PrivacyDataKind::BrowserCache,
            caches.to_vec(),
            PrivacyTimeRange::AllTime,
            false,
            &[],
            &PlatformCancellation::new(|| false),
            &mut candidates,
            &mut items,
        );
        add_database_candidate(
            "origin-isolation",
            source,
            source,
            &format!("{source}:Default"),
            "Default",
            &history,
            PlatformPrivacyBrowserKind::Chromium,
            PrivacyDataKind::BrowsingHistory,
            PrivacyTimeRange::AllTime,
            1,
            false,
            &[],
            &mut candidates,
            &mut items,
        );
    }
    assert_eq!(items.len(), 4);
    assert!(items.iter().all(|item| item.item_count
        == if item.kind == PrivacyDataKind::BrowserCache {
            3
        } else {
            1
        }));
    assert_ne!(items[0].token, items[1].token);
    let tokens = items
        .iter()
        .filter(|item| item.source_id == "brave-origin")
        .map(|item| item.token.clone())
        .collect::<Vec<_>>();
    replace_scan_session(PrivacyScanSession {
        #[cfg(windows)]
        browser_application_paths: BTreeMap::new(),
        public_result: PrivacyScanResult {
            schema_version: PRIVACY_SCAN_SCHEMA_VERSION,
            scan_id: "origin-isolation".into(),
            revision: "fixture".into(),
            time_range: PrivacyTimeRange::AllTime,
            scanned_at_ms: 1,
            elapsed_ms: 1,
            items,
            coverage: Vec::new(),
        },
        candidates,
    })
    .unwrap();
    let plan = PrivacyService::prepare(PrivacyExecutionRequest {
        scan_id: "origin-isolation".into(),
        tokens,
    })
    .unwrap();
    assert_eq!(plan.items.len(), 2);
    assert!(plan
        .items
        .iter()
        .all(|item| item.source_id == "brave-origin"));
    let result = PrivacyService::execute(PrivacyExecutionRunRequest {
        plan_id: plan.plan_id,
        excluded_source_ids: Vec::new(),
    })
    .unwrap();
    assert_eq!(result.failed_item_count, 0, "{result:?}");
    assert_eq!(result.items.len(), 2);
    assert!(result.items.iter().all(|item| item.verified), "{result:?}");
    for (source, expected) in [("brave", 1_i64), ("brave-origin", 0)] {
        let root = fixture.path().join(source);
        for cache in ["Cache", "DawnGraphiteCache", "DawnWebGPUCache"] {
            let payload = root.join(cache).join("payload");
            if source == "brave" {
                assert_eq!(fs::read(payload).unwrap(), b"regenerable cache");
            } else {
                assert!(!payload.exists());
            }
        }
        let connection = rusqlite::Connection::open(root.join("History")).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM visits", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, expected);
        for file in ["Bookmarks", "Login Data", "Cookies", "Preferences"] {
            assert_eq!(fs::read(root.join(file)).unwrap(), b"preserved state");
        }
    }
    clear_scan_session().unwrap();
    clear_pending_plan().unwrap();
}

#[test]
#[ignore = "reads initialized Brave and standalone Brave Origin profiles without modifying them"]
fn actual_brave_origin_privacy_scan_returns_independent_evidence() {
    let _guard = crate::shared::operation::test_operation_lock();
    let result = PrivacyService::scan(PrivacyScanRequest {
        time_range: PrivacyTimeRange::AllTime,
    })
    .unwrap();
    for source in ["brave", "brave-origin"] {
        let items = result
            .items
            .iter()
            .filter(|item| item.source_id == source)
            .collect::<Vec<_>>();
        println!(
            "source={source} items={} traces={} running_items={} elapsed_ms={}",
            items.len(),
            items.iter().map(|item| item.item_count).sum::<u64>(),
            items
                .iter()
                .filter(|item| item.capability == PrivacyCapabilityState::BrowserRunning)
                .count(),
            result.elapsed_ms
        );
        assert!(!items.is_empty());
        assert!(items.iter().all(|item| item
            .profile_id
            .as_ref()
            .is_some_and(|id| id.starts_with(&format!("{source}:")))));
        assert!(result
            .coverage
            .iter()
            .any(|entry| entry.source_id == source));
        for item in items.iter().filter(|item| {
            matches!(
                item.kind,
                PrivacyDataKind::Cookies
                    | PrivacyDataKind::SavedPasswords
                    | PrivacyDataKind::AutofillData
                    | PrivacyDataKind::SiteStorage
            )
        }) {
            assert!(!item.selected_by_default);
        }
    }
    clear_scan_session().unwrap();
}
