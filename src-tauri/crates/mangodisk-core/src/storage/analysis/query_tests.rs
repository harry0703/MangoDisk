use super::*;

fn file(bytes: u64, shared: bool) -> IndexedFile {
    IndexedFile {
        shared_identity: shared.then_some(mangodisk_platform::PhysicalFileIdentity {
            volume: 1,
            index: 2,
        }),
        bytes,
        logical_bytes: bytes.max(4096),
        modified_at_ms: Some(123),
    }
}

fn rows(
    query: &AnalysisQuery<'_>,
    root: &Path,
    aggregate: DirectoryAggregate,
) -> Vec<DirectoryEntryInfo> {
    let CachedList::Ready { mut entries, .. } = query.cached_list(root, aggregate).unwrap() else {
        panic!("the retained prefix must satisfy the published list");
    };
    rank_entries(&mut entries);
    entries
}

#[test]
fn private_and_published_queries_share_the_same_top_list_without_source_io() {
    // The path need not exist: both query adapters operate exclusively on retained facts.
    let root = Path::new("/mangodisk-query-no-source");
    let aggregate = DirectoryAggregate {
        bytes: 1_000_000_000,
        file_count: 504,
        direct_file_count: 501,
        retained_file_limit: 1000,
        ..Default::default()
    };
    let directories = HashMap::from([
        (root.into(), aggregate),
        (
            root.join("small-folder"),
            DirectoryAggregate {
                bytes: 4096,
                file_count: 3,
                ..Default::default()
            },
        ),
        (root.join("empty-folder"), DirectoryAggregate::default()),
    ]);
    // The globally discarded smallest file is not available; the top 500 still are.
    let files: HashMap<_, _> = (1..=500)
        .map(|index| {
            (
                root.join(format!("file-{index:03}")),
                file((index + 1) * 4096, false),
            )
        })
        .collect();
    let _lock = crate::shared::operation::test_operation_lock();
    use crate::storage::index::cache;
    cache::clear_all().unwrap();
    let expected = rows(
        &AnalysisQuery::from_snapshot(&directories, &files),
        root,
        aggregate,
    );
    let publication = cache::SnapshotPublication::new(
        mangodisk_platform::ScanPurpose::Analysis,
        false,
        None,
        1,
        cache::mutation_revision().unwrap(),
    );
    assert!(cache::store_memory_only(
        root,
        aggregate,
        directories.clone(),
        files.clone(),
        publication
    )
    .unwrap());
    let (actual, totals) = cache::read_snapshot(|cache| {
        let query = AnalysisQuery::from_cache(&cache);
        (
            rows(&query, root, aggregate),
            query.directory_totals(root).unwrap(),
        )
    })
    .unwrap();
    cache::clear_all().unwrap();
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(actual.len(), ANALYSIS_VISIBLE_ENTRY_LIMIT);
    assert_eq!(actual[0].name, "file-500");
    assert_eq!(
        (totals.bytes, totals.file_count, totals.directory_count),
        (4096, 3, 2)
    );
    assert_eq!(
        files.len(),
        500,
        "queries must not extend retained metadata"
    );
}

#[test]
fn global_eviction_cannot_masquerade_as_a_complete_largest_prefix() {
    let root = Path::new("/fixture");
    let aggregate = DirectoryAggregate {
        direct_file_count: 600,
        file_count: 600,
        retained_file_limit: 1000,
        ..Default::default()
    };
    let directories = HashMap::from([(root.into(), aggregate)]);
    let files: HashMap<_, _> = (0..200)
        .map(|index| (root.join(format!("file-{index:03}")), file(4096, false)))
        .collect();
    assert!(matches!(
        AnalysisQuery::from_snapshot(&directories, &files)
            .cached_list(root, aggregate)
            .unwrap(),
        CachedList::NeedsDirectoryRead(DirectoryReadReason::LargestRowsUnavailable)
    ));
}

#[test]
fn zero_charge_aliases_and_empty_files_remain_distinct_from_unretained_detail() {
    let root = Path::new("/fixture");
    let aggregate = DirectoryAggregate {
        file_count: 2,
        retained_file_limit: 1000,
        ..Default::default()
    };
    let directories = HashMap::from([(root.into(), aggregate)]);
    let mut files = HashMap::from([
        (root.join("alias"), file(0, true)),
        (root.join("empty"), file(0, false)),
    ]);
    let entries = rows(
        &AnalysisQuery::from_snapshot(&directories, &files),
        root,
        aggregate,
    );
    assert_eq!(entries.len(), 2);
    assert!(entries
        .iter()
        .all(|entry| entry.bytes == 0 && entry.file_count == 1));
    files.remove(&root.join("empty"));
    assert!(matches!(
        AnalysisQuery::from_snapshot(&directories, &files)
            .cached_list(root, aggregate)
            .unwrap(),
        CachedList::NeedsDirectoryRead(DirectoryReadReason::IncompleteDetail)
    ));
}
