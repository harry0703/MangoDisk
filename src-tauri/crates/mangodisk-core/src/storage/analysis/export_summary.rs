use std::{cmp::Ordering, cmp::Reverse, collections::BinaryHeap, collections::HashMap, path::Path};

use crate::{
    reporting::analysis::AnalysisReportSubtreeTotals, storage::index::cache::DirectoryAggregate,
    CoreError, CoreErrorReason, CoreResult,
};

pub(super) const SUMMARY_DIRECTORY_LIMIT: usize = 500;

pub(super) struct SelectedDirectory {
    pub index: usize,
    pub collapsed: Option<AnalysisReportSubtreeTotals>,
    pub filtered: Option<AnalysisReportSubtreeTotals>,
}

/// Compare normalized importance without rounding large native byte or file counts.
#[derive(Clone, Copy, Eq)]
struct Priority(u64, u64);

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> Ordering {
        (u128::from(self.0) * u128::from(other.1)).cmp(&(u128::from(other.0) * u128::from(self.1)))
    }
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Priority {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

fn inconsistent(path: &Path, reason: &str) -> CoreError {
    CoreError::invalid_input(format!(
        "analysis export index is inconsistent: {reason} at {}",
        path.display()
    ))
    .with_reason(CoreErrorReason::AnalysisExportUnavailable)
}

/// Selected nodes are ancestor-closed. Each collapsed bucket is therefore a disjoint union
/// of unselected immediate-child subtrees, never a sum of overlapping recursive rows.
/// Size filtering retains incomplete measurements and all ancestors needed to reach matches.
pub(super) fn select(
    root: &Path,
    directories: &[(&Path, &DirectoryAggregate)],
    limit: usize,
    min_directory_bytes: u64,
) -> CoreResult<Vec<SelectedDirectory>> {
    let by_path: HashMap<_, _> = directories
        .iter()
        .enumerate()
        .map(|(index, (path, _))| (*path, index))
        .collect();
    let root_index = *by_path
        .get(root)
        .ok_or_else(|| inconsistent(root, "missing_root"))?;
    let mut children = vec![Vec::new(); directories.len()];
    let mut depths = vec![0; directories.len()];
    for (index, (path, _)) in directories.iter().enumerate() {
        depths[index] = path.components().count();
        if index != root_index {
            let parent = path.parent().and_then(|parent| by_path.get(parent));
            children[*parent.ok_or_else(|| inconsistent(path, "missing_parent"))?].push(index);
        }
    }
    let mut deepest_first: Vec<_> = (0..directories.len()).collect();
    deepest_first.sort_unstable_by_key(|index| Reverse(depths[*index]));
    let mut subtree_counts = vec![1usize; directories.len()];
    let mut eligible: Vec<_> = directories
        .iter()
        .map(|(_, aggregate)| {
            aggregate.bytes >= min_directory_bytes
                || super::coverage::entry_size_status(**aggregate)
                    != super::AnalysisEntrySizeStatus::Complete
        })
        .collect();
    for &index in &deepest_first {
        let mut child_bytes = 0u64;
        let mut child_files = 0u64;
        for &child in &children[index] {
            eligible[index] |= eligible[child];
            subtree_counts[index] = subtree_counts[index]
                .checked_add(subtree_counts[child])
                .ok_or_else(|| inconsistent(directories[index].0, "child_totals_overflow"))?;
            child_bytes = child_bytes
                .checked_add(directories[child].1.bytes)
                .ok_or_else(|| inconsistent(directories[index].0, "child_totals_overflow"))?;
            child_files = child_files
                .checked_add(directories[child].1.file_count)
                .ok_or_else(|| inconsistent(directories[index].0, "child_totals_overflow"))?;
        }
        if child_bytes > directories[index].1.bytes || child_files > directories[index].1.file_count
        {
            return Err(inconsistent(
                directories[index].0,
                "child_totals_exceed_parent",
            ));
        }
    }

    let total = directories[root_index].1;
    let priority = |index: usize| {
        let aggregate = directories[index].1;
        [
            Priority(aggregate.bytes, total.bytes.max(1)),
            Priority(aggregate.file_count, total.file_count.max(1)),
            Priority(
                aggregate
                    .omissions
                    .with_unclassified(aggregate.skipped_count)
                    .total(),
                total
                    .omissions
                    .with_unclassified(total.skipped_count)
                    .total()
                    .max(1),
            ),
        ]
        .into_iter()
        .max()
        .expect("importance dimensions are nonempty")
    };
    let mut selected = vec![false; directories.len()];
    let mut candidates = BinaryHeap::new();
    candidates.push((
        priority(root_index),
        Reverse(depths[root_index]),
        Reverse(root),
        root_index,
    ));
    let mut result = Vec::new();
    while let Some((_, _, _, index)) = candidates.pop() {
        selected[index] = true;
        result.push(SelectedDirectory {
            index,
            collapsed: None,
            filtered: None,
        });
        if result.len() >= limit.max(1) {
            break;
        }
        for &child in &children[index] {
            if !eligible[child] {
                continue;
            }
            candidates.push((
                priority(child),
                Reverse(depths[child]),
                Reverse(directories[child].0),
                child,
            ));
        }
    }
    for row in &mut result {
        let mut collapsed = AnalysisReportSubtreeTotals::default();
        let mut filtered = AnalysisReportSubtreeTotals::default();
        for &child in &children[row.index] {
            if !selected[child] {
                let bucket = if eligible[child] {
                    &mut collapsed
                } else {
                    &mut filtered
                };
                bucket.directory_count += subtree_counts[child];
                bucket.bytes += directories[child].1.bytes;
                bucket.file_count += directories[child].1.file_count;
            }
        }
        row.collapsed = (collapsed.directory_count > 0).then_some(collapsed);
        row.filtered = (filtered.directory_count > 0).then_some(filtered);
    }
    Ok(result)
}

#[cfg(test)]
#[path = "export_summary_tests.rs"]
mod tests;
