use std::{
    cmp::{Ordering, Reverse},
    collections::{BinaryHeap, HashMap},
    path::{Path, PathBuf},
};

use super::{
    export::{unavailable, AnalysisExportRequest, AnalysisExportSnapshot},
    query::{AnalysisQuery, ChildMeasurement, KnownChild},
    AnalysisEntrySizeStatus,
};
use crate::{
    filesystem::metadata::display_path,
    reporting::analysis::{AnalysisReportEntry, AnalysisReportRemaining, AnalysisReportTotals},
    storage::index::cache::{DirectoryAggregate, IndexReadView},
    CoreResult,
};

struct Node {
    entry: AnalysisReportEntry,
    path: PathBuf,
    children: Vec<usize>,
}
struct Candidate {
    parent: usize,
    node: Node,
}
impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.node
            .entry
            .bytes
            .cmp(&other.node.entry.bytes)
            .then_with(|| other.node.path.cmp(&self.node.path))
            .then_with(|| self.parent.cmp(&other.parent))
    }
}

fn totals(entry: &AnalysisReportEntry) -> AnalysisReportTotals {
    AnalysisReportTotals {
        bytes: entry.bytes,
        file_count: entry.file_count.unwrap_or(1),
        directory_count: u64::from(entry.is_directory),
    }
}
fn add(bucket: &mut Option<AnalysisReportTotals>, value: AnalysisReportTotals) -> CoreResult<()> {
    if !value.is_empty() {
        bucket.get_or_insert_with(Default::default).add(value)?;
    }
    Ok(())
}
fn subtract(
    total: AnalysisReportTotals,
    removed: AnalysisReportTotals,
) -> CoreResult<AnalysisReportTotals> {
    Ok(AnalysisReportTotals {
        bytes: total
            .bytes
            .checked_sub(removed.bytes)
            .ok_or_else(unavailable)?,
        file_count: total
            .file_count
            .checked_sub(removed.file_count)
            .ok_or_else(unavailable)?,
        directory_count: 0,
    })
}
fn directory(path: PathBuf, aggregate: DirectoryAggregate, full: bool) -> Node {
    Node {
        entry: AnalysisReportEntry {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| display_path(&path)),
            path: full.then(|| display_path(&path)),
            bytes: aggregate.bytes,
            is_directory: true,
            file_count: Some(aggregate.file_count),
            modified_at_ms: None,
            scanned_at_ms: Some(aggregate.scanned_at_ms),
            size_status: super::coverage::entry_size_status(aggregate),
            omissions: aggregate
                .omissions
                .with_unclassified(aggregate.skipped_count),
            children: Some(Vec::new()),
            remaining: AnalysisReportRemaining::default(),
        },
        path,
        children: Vec::new(),
    }
}
fn file(path: PathBuf, bytes: u64, modified: Option<u64>, full: bool) -> Node {
    Node {
        entry: AnalysisReportEntry {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: full.then(|| display_path(&path)),
            bytes,
            is_directory: false,
            file_count: None,
            modified_at_ms: modified,
            scanned_at_ms: None,
            size_status: AnalysisEntrySizeStatus::Complete,
            omissions: Default::default(),
            children: None,
            remaining: Default::default(),
        },
        path,
        children: Vec::new(),
    }
}

/// A bounded heap compares all eligible parents at the same depth. It never
/// accumulates a whole wide directory just to select a small export prefix.
fn retain(
    candidate: Candidate,
    heap: &mut BinaryHeap<Reverse<Candidate>>,
    limit: Option<usize>,
    nodes: &mut [Node],
) -> CoreResult<()> {
    if limit.is_none_or(|limit| heap.len() < limit) {
        heap.push(Reverse(candidate));
    } else if heap.peek().is_some_and(|smallest| candidate > smallest.0) {
        let discarded = heap.pop().expect("a ranked candidate exists").0;
        add(
            &mut nodes[discarded.parent].entry.remaining.entry_limit,
            totals(&discarded.node.entry),
        )?;
        heap.push(Reverse(candidate));
    } else {
        add(
            &mut nodes[candidate.parent].entry.remaining.entry_limit,
            totals(&candidate.node.entry),
        )?;
    }
    Ok(())
}

fn direct_directory_totals(
    query: &AnalysisQuery<'_>,
    path: &Path,
) -> CoreResult<AnalysisReportTotals> {
    let totals = query.directory_totals(path)?;
    Ok(AnalysisReportTotals {
        bytes: totals.bytes,
        file_count: totals.file_count,
        directory_count: totals.directory_count,
    })
}

fn known_node(child: KnownChild<'_>, full: bool) -> Node {
    match child.measurement {
        ChildMeasurement::Directory(aggregate) => {
            directory(child.path.into_owned(), *aggregate, full)
        }
        ChildMeasurement::File(measured) => file(
            child.path.into_owned(),
            measured.bytes,
            measured.modified_at_ms,
            full,
        ),
    }
}

fn take_visible(
    rows: &mut HashMap<&str, Vec<usize>>,
    path: &str,
    entries: &[super::DirectoryEntryInfo],
    bytes: u64,
    files: Option<u64>,
) -> Option<usize> {
    let matching = rows.get_mut(path)?;
    let offset = matching.iter().position(|&index| {
        entries[index].bytes == bytes
            && files.is_none_or(|count| entries[index].file_count == count)
    })?;
    Some(matching.swap_remove(offset))
}

fn expand_root(
    query: &AnalysisQuery<'_>,
    snapshot: &AnalysisExportSnapshot,
    nodes: &mut [Node],
    heap: &mut BinaryHeap<Reverse<Candidate>>,
    limit: Option<usize>,
    full: bool,
) -> CoreResult<()> {
    let native_root = nodes[0].path.clone();
    // Native names can be non-UTF-8. Match displayed directory rows to native keys
    // instead of reconstructing potentially lossy paths and looking them up again.
    let mut visible_directories: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut visible_files: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, entry) in snapshot.entries.iter().enumerate() {
        Path::new(&entry.path)
            .strip_prefix(Path::new(&snapshot.root))
            .map_err(|_| unavailable())?;
        if entry.is_directory {
            visible_directories
                .entry(&entry.path)
                .or_default()
                .push(index);
        } else {
            visible_files.entry(&entry.path).or_default().push(index);
        }
    }
    let mut file_budget = subtract(
        totals(&nodes[0].entry),
        direct_directory_totals(query, &native_root)?,
    )?;
    for child in query.directories(&native_root) {
        let child = child?;
        let mut node = known_node(child, full);
        let measured_bytes = node.entry.bytes;
        let measured_files = node.entry.file_count.unwrap_or(0);
        let displayed = display_path(&node.path);
        if let Some(index) = take_visible(
            &mut visible_directories,
            &displayed,
            &snapshot.entries,
            measured_bytes,
            Some(measured_files),
        ) {
            let row = &snapshot.entries[index];
            if row.bytes != measured_bytes || row.file_count != measured_files {
                return Err(unavailable());
            }
            node.entry.modified_at_ms = row.modified_at_ms;
            retain(Candidate { parent: 0, node }, heap, limit, nodes)?;
        } else {
            add(
                &mut nodes[0].entry.remaining.list_limit,
                totals(&node.entry),
            )?;
        }
    }
    if visible_directories.values().any(|rows| !rows.is_empty()) {
        return Err(unavailable());
    }
    // The current list remains authoritative even for file rows not retained in
    // the supplemental index. Each distinct published row consumes one file count.
    for row in snapshot.entries.iter().filter(|row| !row.is_directory) {
        let relative = Path::new(&row.path)
            .strip_prefix(Path::new(&snapshot.root))
            .map_err(|_| unavailable())?;
        let mut node = file(
            native_root.join(relative),
            row.bytes,
            row.modified_at_ms,
            full,
        );
        node.entry.name.clone_from(&row.name);
        node.entry.size_status = row.size_status;
        if query
            .files
            .get(&node.path)
            .is_some_and(|cached| cached.bytes != row.bytes)
        {
            return Err(unavailable());
        }
        file_budget = subtract(file_budget, totals(&node.entry))?;
        retain(Candidate { parent: 0, node }, heap, limit, nodes)?;
    }
    for child in query.files(&native_root) {
        let child = child?;
        let displayed = display_path(&child.path);
        let measured_bytes = child.bytes();
        if take_visible(
            &mut visible_files,
            &displayed,
            &snapshot.entries,
            measured_bytes,
            None,
        )
        .is_none()
        {
            let value = AnalysisReportTotals {
                bytes: measured_bytes,
                file_count: 1,
                directory_count: 0,
            };
            file_budget = subtract(file_budget, value)?;
            add(&mut nodes[0].entry.remaining.list_limit, value)?;
        }
    }
    add(
        &mut nodes[0].entry.remaining.details_not_retained,
        file_budget,
    )
}

fn expand_directory(
    query: &AnalysisQuery<'_>,
    parent: usize,
    nodes: &mut [Node],
    heap: &mut BinaryHeap<Reverse<Candidate>>,
    limit: Option<usize>,
    full: bool,
) -> CoreResult<()> {
    let path = nodes[parent].path.clone();
    let mut file_budget = subtract(
        totals(&nodes[parent].entry),
        direct_directory_totals(query, &path)?,
    )?;
    for child in query.children(&path) {
        let child = child?;
        let is_file = matches!(child.measurement, ChildMeasurement::File(_));
        let node = known_node(child, full);
        if is_file {
            file_budget = subtract(file_budget, totals(&node.entry))?;
        }
        retain(Candidate { parent, node }, heap, limit, nodes)?;
    }
    add(
        &mut nodes[parent].entry.remaining.details_not_retained,
        file_budget,
    )
}

pub(super) fn select(
    cache: &IndexReadView<'_>,
    snapshot: &AnalysisExportSnapshot,
    request: AnalysisExportRequest,
) -> CoreResult<(AnalysisReportEntry, usize)> {
    let query = AnalysisQuery::from_cache(cache);
    let root_path = snapshot
        .index_snapshot
        .as_ref()
        .ok_or_else(unavailable)?
        .root
        .clone();
    let aggregate = *query.directories.get(&root_path).ok_or_else(unavailable)?;
    let mut root = directory(root_path, aggregate, request.include_full_paths);
    root.entry.omissions = snapshot.coverage.omissions;
    let mut nodes = vec![root];
    let mut frontier = vec![0];
    for _ in 1..=request.max_depth {
        let limit =
            (request.max_entries != 0).then(|| request.max_entries.saturating_sub(nodes.len() - 1));
        let mut heap = BinaryHeap::new();
        for parent in frontier {
            let expanded = if parent == 0 {
                expand_root(
                    &query,
                    snapshot,
                    &mut nodes,
                    &mut heap,
                    limit,
                    request.include_full_paths,
                )
            } else {
                expand_directory(
                    &query,
                    parent,
                    &mut nodes,
                    &mut heap,
                    limit,
                    request.include_full_paths,
                )
            };
            if let Err(error) = expanded {
                log::warn!("analysis_export_directory_rejected scan_id={} directory={} stage=accounting outcome=not_exported error={}", request.scan_id, crate::filesystem::metadata::diagnostic_path(&nodes[parent].path), mangodisk_platform::diagnostics::text(&error));
                return Err(error);
            }
        }
        let mut selected: Vec<_> = heap.into_iter().map(|value| value.0).collect();
        selected.sort_unstable_by(|left, right| right.cmp(left));
        frontier = Vec::new();
        for candidate in selected {
            let index = nodes.len();
            nodes[candidate.parent].children.push(index);
            if candidate.node.entry.is_directory {
                frontier.push(index);
            }
            nodes.push(candidate.node);
        }
        if frontier.is_empty() {
            break;
        }
    }
    // Only the final frontier is depth-limited. Earlier directories may instead
    // have exhausted the shared entry budget, which their own buckets explain.
    for &index in &frontier {
        let node = &mut nodes[index];
        let counts = direct_directory_totals(&query, &node.path)?;
        let value = AnalysisReportTotals {
            bytes: node.entry.bytes,
            file_count: node.entry.file_count.unwrap_or(0),
            directory_count: counts.directory_count,
        };
        add(&mut node.entry.remaining.depth_limit, value)?;
        if !value.is_empty() || node.entry.size_status != AnalysisEntrySizeStatus::Complete {
            node.entry.children = None;
        }
    }
    let count = nodes.len() - 1;
    fn finish(index: usize, nodes: &mut [Option<Node>], scanned_at_ms: u64) -> AnalysisReportEntry {
        let mut node = nodes[index]
            .take()
            .expect("each selected node has exactly one parent");
        if node.entry.scanned_at_ms == Some(scanned_at_ms) {
            node.entry.scanned_at_ms = None;
        }
        if node.entry.children.is_some() {
            node.entry.children = Some(
                node.children
                    .into_iter()
                    .map(|child| finish(child, nodes, scanned_at_ms))
                    .collect(),
            );
        }
        node.entry
    }
    let mut owned: Vec<_> = nodes.into_iter().map(Some).collect();
    Ok((finish(0, &mut owned, snapshot.scanned_at_ms), count))
}
