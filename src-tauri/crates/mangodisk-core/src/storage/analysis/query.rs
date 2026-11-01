use std::{
    borrow::Cow,
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use crate::{
    filesystem::metadata::{display_fingerprint, display_path},
    storage::index::{
        cache::{DirectoryAggregate, IndexReadView, IndexedFile},
        navigation::NavigationIndex,
    },
    CoreError, CoreResult,
};

use super::{AnalysisIndexSnapshot, DirectoryEntryInfo, ANALYSIS_VISIBLE_ENTRY_LIMIT};

/// A borrowed query boundary over retained facts, never a filesystem reader or a second cache.
#[derive(Clone, Copy)]
pub(super) struct AnalysisQuery<'a> {
    pub directories: &'a HashMap<PathBuf, DirectoryAggregate>,
    pub files: &'a HashMap<PathBuf, IndexedFile>,
    navigation: Option<&'a NavigationIndex>,
}

#[derive(Clone, Copy)]
pub(super) enum ChildMeasurement<'a> {
    Directory(&'a DirectoryAggregate),
    File(&'a IndexedFile),
}

pub(super) struct KnownChild<'a> {
    pub path: Cow<'a, Path>,
    pub measurement: ChildMeasurement<'a>,
}

impl KnownChild<'_> {
    pub fn bytes(&self) -> u64 {
        match self.measurement {
            ChildMeasurement::Directory(value) => value.bytes,
            ChildMeasurement::File(value) => value.bytes,
        }
    }

    pub fn entry(&self) -> DirectoryEntryInfo {
        match self.measurement {
            ChildMeasurement::Directory(value) => directory_entry(&self.path, *value),
            ChildMeasurement::File(value) => file_entry(&self.path, *value),
        }
    }
}

pub(super) struct DirectDirectoryTotals {
    pub bytes: u64,
    pub file_count: u64,
    pub directory_count: u64,
}

pub(super) enum CachedList {
    Ready {
        entries: Vec<DirectoryEntryInfo>,
        total_entry_count: usize,
    },
    NeedsDirectoryRead(DirectoryReadReason),
}

#[derive(Clone, Copy)]
pub(super) enum DirectoryReadReason {
    IncompleteDetail,
    LargestRowsUnavailable,
}

impl DirectoryReadReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IncompleteDetail => "incomplete_detail",
            Self::LargestRowsUnavailable => "largest_rows_unavailable",
        }
    }
}

impl<'a> AnalysisQuery<'a> {
    pub fn from_snapshot(
        directories: &'a HashMap<PathBuf, DirectoryAggregate>,
        files: &'a HashMap<PathBuf, IndexedFile>,
    ) -> Self {
        Self {
            directories,
            files,
            navigation: None,
        }
    }

    pub fn with_navigation(
        directories: &'a HashMap<PathBuf, DirectoryAggregate>,
        files: &'a HashMap<PathBuf, IndexedFile>,
        navigation: &'a NavigationIndex,
    ) -> Self {
        Self {
            directories,
            files,
            navigation: Some(navigation),
        }
    }

    pub fn from_cache(cache: &IndexReadView<'a>) -> Self {
        Self::with_navigation(cache.directories, cache.files, cache.navigation)
    }

    pub fn navigation(&self) -> Option<&'a NavigationIndex> {
        self.navigation
    }

    pub fn directories<'b>(
        &'b self,
        parent: &Path,
    ) -> impl Iterator<Item = CoreResult<KnownChild<'b>>> + 'b {
        let names = self
            .navigation
            .and_then(|index| index.directory_names(parent));
        direct_records(self.directories, names, parent, self.navigation.is_some()).map(|value| {
            value.map(|(path, value)| KnownChild {
                path,
                measurement: ChildMeasurement::Directory(value),
            })
        })
    }

    pub fn files<'b>(
        &'b self,
        parent: &Path,
    ) -> impl Iterator<Item = CoreResult<KnownChild<'b>>> + 'b {
        let names = self.navigation.and_then(|index| index.file_names(parent));
        direct_records(self.files, names, parent, self.navigation.is_some()).map(|value| {
            value.map(|(path, value)| KnownChild {
                path,
                measurement: ChildMeasurement::File(value),
            })
        })
    }

    pub fn children<'b>(
        &'b self,
        parent: &Path,
    ) -> impl Iterator<Item = CoreResult<KnownChild<'b>>> + 'b {
        self.directories(parent).chain(self.files(parent))
    }

    pub fn directory_totals(&self, parent: &Path) -> CoreResult<DirectDirectoryTotals> {
        let (mut bytes, mut files, mut directories) = (0_u64, 0_u64, 0_u64);
        for child in self.directories(parent) {
            let child = child?;
            if let ChildMeasurement::Directory(value) = child.measurement {
                let overflow =
                    || CoreError::invalid_input("analysis directory accounting overflowed");
                bytes = bytes.checked_add(value.bytes).ok_or_else(overflow)?;
                files = files.checked_add(value.file_count).ok_or_else(overflow)?;
                directories = directories.checked_add(1).ok_or_else(overflow)?;
            }
        }
        Ok(DirectDirectoryTotals {
            bytes,
            file_count: files,
            directory_count: directories,
        })
    }

    pub fn positive_directory_count(&self, parent: &Path) -> u64 {
        if let Some(index) = self.navigation {
            index.directory_count(parent, self.directories)
        } else {
            self.directories
                .iter()
                .filter(|(path, value)| path.parent() == Some(parent) && value.bytes > 0)
                .count() as u64
        }
    }

    /// Preserve the existing proof of top-list completeness after global candidate eviction.
    pub fn cached_list(
        &self,
        root: &Path,
        aggregate: DirectoryAggregate,
    ) -> CoreResult<CachedList> {
        let children = self.children(root).collect::<CoreResult<Vec<_>>>()?;
        let direct_files = children.iter().filter_map(|child| match child.measurement {
            ChildMeasurement::File(file) => Some(file),
            ChildMeasurement::Directory(_) => None,
        });
        let mut retained_count = 0_u64;
        let mut shared_owners = 0_u64;
        let mut retained_ordinary = 0_usize;
        for file in direct_files {
            retained_count += 1;
            if file.bytes > 0 {
                if file.shared_identity.is_some() {
                    shared_owners += 1;
                } else {
                    retained_ordinary += 1;
                }
            }
        }
        let total_entry_count = aggregate.direct_file_count as usize
            + children.iter().filter(|child| matches!(child.measurement, ChildMeasurement::Directory(value) if value.bytes > 0)).count();
        if aggregate.file_count != retained_count
            && total_entry_count <= ANALYSIS_VISIBLE_ENTRY_LIMIT
        {
            return Ok(CachedList::NeedsDirectoryRead(
                DirectoryReadReason::IncompleteDetail,
            ));
        }
        let ordinary_count = aggregate.direct_file_count.saturating_sub(shared_owners);
        if retained_ordinary < ordinary_count.min(ANALYSIS_VISIBLE_ENTRY_LIMIT as u64) as usize
            || (retained_ordinary < ordinary_count as usize
                && aggregate.retained_file_limit < ANALYSIS_VISIBLE_ENTRY_LIMIT)
        {
            return Ok(CachedList::NeedsDirectoryRead(
                DirectoryReadReason::LargestRowsUnavailable,
            ));
        }
        Ok(CachedList::Ready {
            entries: children.iter().map(KnownChild::entry).collect(),
            total_entry_count,
        })
    }
}

fn direct_records<'a, T: 'a>(
    records: &'a HashMap<PathBuf, T>,
    names: Option<&'a [OsString]>,
    parent: &Path,
    indexed: bool,
) -> impl Iterator<Item = CoreResult<(Cow<'a, Path>, &'a T)>> + 'a {
    let native_parent = parent.to_path_buf();
    let map_parent = native_parent.clone();
    let indexed_rows = names.into_iter().flatten().map(move |name| {
        let path = native_parent.join(name);
        let value = records.get(&path).ok_or_else(|| {
            CoreError::invalid_input(format!(
                "the analysis child index is inconsistent: {}",
                crate::filesystem::metadata::diagnostic_path(&path)
            ))
        })?;
        Ok((Cow::Owned(path), value))
    });
    let snapshot_rows = (!indexed)
        .then_some(records)
        .into_iter()
        .flat_map(HashMap::iter)
        .filter(move |(path, _)| path.parent() == Some(map_parent.as_path()))
        .map(|(path, value)| Ok((Cow::Borrowed(path.as_path()), value)));
    indexed_rows.chain(snapshot_rows)
}

pub(super) fn directory_entry(path: &Path, value: DirectoryAggregate) -> DirectoryEntryInfo {
    DirectoryEntryInfo {
        size_status: super::coverage::entry_size_status(value),
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: display_path(path),
        bytes: value.bytes,
        logical_bytes: value.logical_bytes,
        file_count: value.file_count,
        is_directory: true,
        modified_at_ms: None,
        content_fingerprint: value.fingerprint.map(display_fingerprint),
    }
}

pub(super) fn file_entry(path: &Path, value: IndexedFile) -> DirectoryEntryInfo {
    DirectoryEntryInfo {
        size_status: Default::default(),
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: display_path(path),
        bytes: value.bytes,
        logical_bytes: value.logical_bytes,
        file_count: 1,
        is_directory: false,
        modified_at_ms: value.modified_at_ms,
        content_fingerprint: None,
    }
}

pub(super) fn rank_entries(entries: &mut Vec<DirectoryEntryInfo>) {
    let compare = |left: &DirectoryEntryInfo, right: &DirectoryEntryInfo| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.path.cmp(&right.path))
    };
    if entries.len() > ANALYSIS_VISIBLE_ENTRY_LIMIT {
        entries.select_nth_unstable_by(ANALYSIS_VISIBLE_ENTRY_LIMIT, compare);
        entries.truncate(ANALYSIS_VISIBLE_ENTRY_LIMIT);
    }
    entries.sort_unstable_by(compare);
}

fn latest_generation(cache: &IndexReadView<'_>, root: &Path) -> Option<u64> {
    cache
        .publish_generations
        .iter()
        .filter(|(scope, _)| root.starts_with(scope) || scope.starts_with(root))
        .map(|(_, generation)| *generation)
        .max()
}

pub(in crate::storage) fn index_snapshot(
    cache: &IndexReadView<'_>,
    root: &Path,
) -> Option<AnalysisIndexSnapshot> {
    Some(AnalysisIndexSnapshot {
        root: root.into(),
        generation: latest_generation(cache, root)?,
        mutation_revision: cache.mutation_revision,
    })
}

pub(super) fn matches_snapshot(
    cache: &IndexReadView<'_>,
    identity: &AnalysisIndexSnapshot,
) -> bool {
    cache.mutation_revision == identity.mutation_revision
        && latest_generation(cache, &identity.root) == Some(identity.generation)
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
