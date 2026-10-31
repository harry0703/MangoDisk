use super::*;
use std::{collections::HashSet, path::PathBuf, time::Instant};

use crate::{
    reporting::analysis::{
        AnalysisReport, AnalysisReportDirectory, AnalysisReportExclusions, AnalysisReportScan,
    },
    storage::analysis::{AnalysisEntrySizeStatus, AnalysisScanMode},
};

fn tree(entries: &[(&str, u64, u64)]) -> Vec<(PathBuf, DirectoryAggregate)> {
    let mut rows: Vec<_> = entries
        .iter()
        .map(|(path, bytes, files)| {
            (
                PathBuf::from(path),
                DirectoryAggregate {
                    bytes: *bytes,
                    file_count: *files,
                    scanned_at_ms: 100,
                    ..Default::default()
                },
            )
        })
        .collect();
    aggregate(&mut rows);
    rows
}

fn aggregate(rows: &mut [(PathBuf, DirectoryAggregate)]) {
    let by_path: HashMap<_, _> = rows
        .iter()
        .enumerate()
        .map(|(index, (path, _))| (path.clone(), index))
        .collect();
    let mut indices: Vec<_> = (0..rows.len()).collect();
    indices.sort_by_key(|index| Reverse(rows[*index].0.components().count()));
    for index in indices {
        if let Some(parent) = rows[index].0.parent().and_then(|path| by_path.get(path)) {
            let child = rows[index].1;
            rows[*parent].1.bytes += child.bytes;
            rows[*parent].1.file_count += child.file_count;
            rows[*parent].1.omissions.permission_denied += child.omissions.permission_denied;
        }
    }
}

fn select_rows(rows: &[(PathBuf, DirectoryAggregate)], limit: usize) -> Vec<SelectedDirectory> {
    let borrowed: Vec<_> = rows
        .iter()
        .map(|(path, value)| (path.as_path(), value))
        .collect();
    select(Path::new("root"), &borrowed, limit, 0).unwrap()
}

fn verify_accounting(rows: &[(PathBuf, DirectoryAggregate)], selected: &[SelectedDirectory]) {
    let paths: HashSet<_> = selected.iter().map(|row| &rows[row.index].0).collect();
    let hidden_count: usize = selected
        .iter()
        .flat_map(|row| row.collapsed.iter().chain(row.filtered.iter()))
        .map(|bucket| bucket.directory_count)
        .sum();
    assert_eq!(rows.len(), selected.len() + hidden_count);
    for row in selected {
        let (path, total) = &rows[row.index];
        if path != Path::new("root") {
            assert!(
                paths.contains(&path.parent().unwrap().to_path_buf()),
                "selected rows retain their ancestors"
            );
        }
        let mut hidden_bytes = 0;
        let mut hidden_files = 0;
        let mut hidden_directories = 0;
        for (child, aggregate) in rows {
            if child.parent() == Some(path) && !paths.contains(child) {
                hidden_bytes += aggregate.bytes;
                hidden_files += aggregate.file_count;
                hidden_directories += rows
                    .iter()
                    .filter(|(descendant, _)| descendant.starts_with(child))
                    .count();
            }
        }
        let buckets = || row.collapsed.iter().chain(row.filtered.iter());
        assert_eq!(
            buckets().map(|bucket| bucket.bytes).sum::<u64>(),
            hidden_bytes
        );
        assert_eq!(
            buckets().map(|bucket| bucket.file_count).sum::<u64>(),
            hidden_files
        );
        assert_eq!(
            buckets()
                .map(|bucket| bucket.directory_count)
                .sum::<usize>(),
            hidden_directories
        );
        assert!(hidden_bytes <= total.bytes);
        assert!(hidden_files <= total.file_count);
    }
}

#[test]
fn summary_retains_exact_totals_and_disjoint_collapsed_subtrees() {
    let rows = tree(&[
        ("root", 7, 2),
        ("root/A", 13, 1),
        ("root/A/deep", 80, 8),
        ("root/B", 20, 4),
        ("root/B/empty", 0, 0),
    ]);
    let selected = select_rows(&rows, 3);
    verify_accounting(&rows, &selected);
    assert_eq!(rows[selected[0].index].1.bytes, 120);
    let collapsed = selected[0].collapsed.as_ref().unwrap();
    assert_eq!(
        (
            collapsed.bytes,
            collapsed.file_count,
            collapsed.directory_count
        ),
        (20, 4, 2)
    );
    assert_eq!(
        120 - 93 - collapsed.bytes,
        7,
        "direct file usage remains derivable without a duplicate total field"
    );
    let complete = select_rows(&rows, 500);
    assert_eq!(complete.len(), rows.len());
    assert!(complete.iter().all(|row| row.collapsed.is_none()));
}

#[test]
fn summary_expands_deep_space_heavy_and_file_heavy_branches() {
    let rows = tree(&[
        ("root", 0, 0),
        ("root/bytes", 0, 0),
        ("root/bytes/deep", 10000, 1),
        ("root/files", 0, 0),
        ("root/files/deep", 1, 10000),
        ("root/small", 1, 1),
    ]);
    let selected = select_rows(&rows, 5);
    let paths: HashSet<_> = selected
        .iter()
        .map(|row| rows[row.index].0.as_path())
        .collect();
    assert!(paths.contains(Path::new("root/bytes/deep")));
    assert!(paths.contains(Path::new("root/files/deep")));
    verify_accounting(&rows, &selected);
}

#[test]
fn summary_keeps_zero_byte_folds_separate_from_scan_omissions() {
    let mut rows = tree(&[
        ("root", 0, 0),
        ("root/empty", 0, 0),
        ("root/unreadable", 0, 0),
        ("root/empty/deep", 0, 0),
    ]);
    rows[0].1.omissions.permission_denied = 1;
    rows[2].1.omissions.permission_denied = 1;
    let selected = select_rows(&rows, 2);
    assert_eq!(rows[selected[1].index].0, Path::new("root/unreadable"));
    let collapsed = selected[0].collapsed.as_ref().unwrap();
    assert_eq!(collapsed.directory_count, 2);
    assert_eq!(collapsed.bytes, 0);
    assert_eq!(
        rows[0].1.omissions.permission_denied, 1,
        "folding does not increase scan omissions"
    );
    verify_accounting(&rows, &selected);
}

#[test]
fn summary_selection_is_deterministic_and_handles_exact_u64_counts() {
    let rows = tree(&[
        ("root", 1, 0),
        ("root/a", 9007199254740993, 1),
        ("root/b", 9007199254740992, 1),
        ("root/c", 0, 0),
    ]);
    let selected = select_rows(&rows, 2);
    assert_eq!(rows[selected[1].index].0, Path::new("root/a"));
    let mut reversed = rows.clone();
    reversed.reverse();
    let reordered = select_rows(&reversed, 2);
    let paths = |rows: &[(PathBuf, DirectoryAggregate)], selected: &[SelectedDirectory]| {
        selected
            .iter()
            .map(|row| rows[row.index].0.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(paths(&rows, &selected), paths(&reversed, &reordered));
    verify_accounting(&rows, &selected);
    assert_eq!(
        select_rows(&rows, 0).len(),
        1,
        "the root is always retained"
    );
}

#[test]
fn inconsistent_directory_totals_or_missing_parents_fail_closed() {
    let rows = tree(&[("root", 0, 0), ("root/child", 1, 1)]);
    for files in [false, true] {
        let mut invalid = rows.clone();
        if files {
            invalid[0].1.file_count = 0;
        } else {
            invalid[0].1.bytes = 0;
        }
        let borrowed: Vec<_> = invalid
            .iter()
            .map(|(path, aggregate)| (path.as_path(), aggregate))
            .collect();
        assert!(select(Path::new("root"), &borrowed, 500, 0).is_err());
    }
    assert!(select(
        Path::new("root"),
        &[
            (Path::new("root"), &rows[0].1),
            (Path::new("root/missing/child"), &rows[1].1)
        ],
        500,
        0
    )
    .is_err());
}

fn report(
    rows: &[(PathBuf, DirectoryAggregate)],
    selected: Vec<SelectedDirectory>,
) -> AnalysisReport {
    let root = rows
        .iter()
        .find(|(path, _)| path == Path::new("root"))
        .unwrap()
        .1;
    AnalysisReport::new(
        AnalysisReportScan {
            id: 1,
            root_name: "root".into(),
            root: "root".into(),
            scanned_at_ms: 100,
            scan_mode: AnalysisScanMode::Standard,
            total_bytes: root.bytes,
            subtree_of_original_scan: false,
        },
        selected
            .into_iter()
            .map(|row| {
                let (path, aggregate) = &rows[row.index];
                let relative = path.strip_prefix("root").unwrap();
                AnalysisReportDirectory {
                    path: if relative.as_os_str().is_empty() {
                        ".".into()
                    } else {
                        relative
                            .components()
                            .map(|part| part.as_os_str().to_string_lossy())
                            .collect::<Vec<_>>()
                            .join("/")
                    },
                    bytes: aggregate.bytes,
                    file_count: aggregate.file_count,
                    scanned_at_ms: None,
                    size_status: AnalysisEntrySizeStatus::Complete,
                    omissions: aggregate.omissions,
                    path_is_lossy: false,
                    collapsed: row.collapsed,
                    filtered: row.filtered,
                }
            })
            .collect(),
        AnalysisReportExclusions::default(),
        false,
        None,
        0,
    )
}

#[test]
fn summary_scale_25253_directories_preserves_statistics_with_bounded_output() {
    let mut rows = vec![(
        PathBuf::from("root"),
        DirectoryAggregate {
            bytes: 1024,
            file_count: 2,
            ..Default::default()
        },
    )];
    for group in 0..100 {
        rows.push((
            PathBuf::from(format!("root/worktrees-{group:03}")),
            DirectoryAggregate::default(),
        ));
    }
    for leaf in 0..25152 {
        rows.push((
            PathBuf::from(format!(
                "root/worktrees-{:03}/project-{leaf:05}",
                leaf / 252
            )),
            DirectoryAggregate {
                bytes: ((leaf % 100) * 4096) as u64,
                file_count: (leaf % 50) as u64,
                ..Default::default()
            },
        ));
    }
    aggregate(&mut rows);
    assert_eq!(rows.len(), 25253);
    let started = Instant::now();
    let selected = select_rows(&rows, SUMMARY_DIRECTORY_LIMIT);
    let elapsed = started.elapsed();
    assert_eq!(selected.len(), 500);
    let summary = report(&rows, selected);
    let full = report(
        &rows,
        (0..rows.len())
            .map(|index| SelectedDirectory {
                index,
                collapsed: None,
                filtered: None,
            })
            .collect(),
    );
    let summary_json = serde_json::to_vec(&summary).unwrap();
    let full_json = serde_json::to_vec(&full).unwrap();
    assert!(summary_json.len() < full_json.len() / 10);
    assert_eq!(summary.directories[0].bytes, full.directories[0].bytes);
    assert_eq!(
        summary.directories[0].file_count,
        full.directories[0].file_count
    );
    let folded: usize = summary
        .directories
        .iter()
        .filter_map(|row| row.collapsed.as_ref())
        .map(|row| row.directory_count)
        .sum();
    assert_eq!(folded + summary.directories.len(), full.directories.len());
    println!(
        "summary_scale directories={} retained={} full_bytes={} summary_bytes={} selection_us={}",
        rows.len(),
        summary.directories.len(),
        full_json.len(),
        summary_json.len(),
        elapsed.as_micros()
    );
}

#[test]
fn size_filter_preserves_incomplete_measurements_and_their_native_ancestors() {
    let mut rows = tree(&[
        ("root", 7, 1),
        ("root/big", 0, 0),
        ("root/big/match", 200, 1),
        ("root/small", 0, 0),
        ("root/small/tiny", 20, 3),
        ("root/bridge", 0, 0),
        ("root/bridge/unknown", 0, 0),
        ("root/bridge/known", 1, 1),
    ]);
    rows[6].1.omissions.permission_denied = 1;
    let refs: Vec<_> = rows
        .iter()
        .map(|(path, aggregate)| (path.as_path(), aggregate))
        .collect();
    for limit in [500, usize::MAX] {
        let selected = select(Path::new("root"), &refs, limit, 200).unwrap();
        let paths: HashSet<_> = selected
            .iter()
            .map(|row| rows[row.index].0.as_path())
            .collect();
        assert_eq!(selected.len(), 5);
        for path in [
            "root",
            "root/big",
            "root/big/match",
            "root/bridge",
            "root/bridge/unknown",
        ] {
            assert!(paths.contains(Path::new(path)), "{path} is retained");
        }
        let root_filtered = selected
            .iter()
            .find(|row| rows[row.index].0 == Path::new("root"))
            .unwrap()
            .filtered
            .as_ref()
            .unwrap();
        assert_eq!(
            (
                root_filtered.directory_count,
                root_filtered.bytes,
                root_filtered.file_count
            ),
            (2, 20, 3)
        );
        assert!(selected.iter().all(|row| row.collapsed.is_none()));
        verify_accounting(&rows, &selected);
    }
}

#[test]
fn size_filter_runs_before_the_summary_budget_and_full_has_no_count_limit() {
    let rows = tree(&[
        ("root", 0, 0),
        ("root/small-many-files", 1, 50000),
        ("root/large-A", 200, 1),
        ("root/large-B", 100, 1),
    ]);
    let refs: Vec<_> = rows
        .iter()
        .map(|(path, aggregate)| (path.as_path(), aggregate))
        .collect();
    let selected = select(Path::new("root"), &refs, 2, 100).unwrap();
    assert_eq!(rows[selected[1].index].0, Path::new("root/large-A"));
    assert_eq!(selected[0].filtered.as_ref().unwrap().file_count, 50000);
    assert_eq!(selected[0].collapsed.as_ref().unwrap().bytes, 100);
    verify_accounting(&rows, &selected);
    let mut many = vec![(PathBuf::from("root"), DirectoryAggregate::default())];
    for index in 0..650 {
        many.push((
            PathBuf::from(format!("root/{index:03}")),
            DirectoryAggregate {
                bytes: if index < 600 { 10 } else { 1 },
                file_count: 1,
                ..Default::default()
            },
        ));
    }
    aggregate(&mut many);
    let refs: Vec<_> = many
        .iter()
        .map(|(path, aggregate)| (path.as_path(), aggregate))
        .collect();
    let full = select(Path::new("root"), &refs, usize::MAX, 10).unwrap();
    assert_eq!(full.len(), 601);
    assert!(full.iter().all(|row| row.collapsed.is_none()));
    let summary = select(Path::new("root"), &refs, 500, 10).unwrap();
    assert_eq!(summary.len(), 500);
    verify_accounting(&many, &full);
    verify_accounting(&many, &summary);
}

#[test]
fn minimum_bytes_are_inclusive_exact_u64_and_do_not_remove_the_root() {
    let rows = tree(&[
        ("root", 0, 0),
        ("root/a", 9007199254740993, 1),
        ("root/b", 9007199254740992, 1),
    ]);
    let refs: Vec<_> = rows
        .iter()
        .map(|(path, aggregate)| (path.as_path(), aggregate))
        .collect();
    let selected = select(Path::new("root"), &refs, usize::MAX, 9007199254740993).unwrap();
    assert_eq!(selected.len(), 2);
    assert_eq!(rows[selected[1].index].0, Path::new("root/a"));
    assert_eq!(
        selected[0].filtered.as_ref().unwrap().bytes,
        9007199254740992
    );
    let root_only = select(Path::new("root"), &refs, usize::MAX, u64::MAX).unwrap();
    assert_eq!(root_only.len(), 1);
    assert_eq!(root_only[0].filtered.as_ref().unwrap().directory_count, 2);
    assert!(root_only[0].collapsed.is_none());
    verify_accounting(&rows, &root_only);
}
