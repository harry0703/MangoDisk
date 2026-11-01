use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};

use mangodisk_platform::{current_platform, Platform};

use crate::shared::{CoreError, CoreResult};

use super::{
    AnalysisRemainderPage, AnalysisRemainderParent, AnalysisRemainderRequest, DirectoryEntryInfo,
};

const PAGE_SIZE: usize = 200;
// One dialog is visible at a time. A second slot allows an earlier opening
// request to finish while the user has already opened another directory.
const SNAPSHOT_LIMIT: usize = 2;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static SNAPSHOTS: OnceLock<Mutex<VecDeque<Arc<RemainderSnapshot>>>> = OnceLock::new();

struct RemainderSnapshot {
    id: u64,
    scan_id: u64,
    parent_path: String,
    visible_paths: Vec<String>,
    total_bytes: u64,
    entries: Vec<DirectoryEntryInfo>,
}

impl RemainderSnapshot {
    fn page(&self, offset: usize) -> AnalysisRemainderPage {
        let start = offset.min(self.entries.len());
        let end = start.saturating_add(PAGE_SIZE).min(self.entries.len());
        AnalysisRemainderPage {
            snapshot_id: self.id,
            schema_version: AnalysisRemainderRequest::SCHEMA_VERSION,
            parent_path: self.parent_path.clone(),
            total_bytes: self.total_bytes,
            total_count: self.entries.len(),
            entries: self.entries[start..end].to_vec(),
            next_offset: (end < self.entries.len()).then_some(end),
        }
    }
}

fn snapshots() -> &'static Mutex<VecDeque<Arc<RemainderSnapshot>>> {
    SNAPSHOTS.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub(super) fn list(
    parent: &AnalysisRemainderParent,
    request: &AnalysisRemainderRequest,
) -> CoreResult<AnalysisRemainderPage> {
    if let Some(id) = request.snapshot_id {
        let snapshot = snapshots()
            .lock()
            .map_err(|_| CoreError::operation_failed("the remainder snapshots are unavailable"))?
            .iter()
            .find(|snapshot| snapshot.id == id)
            .cloned();
        if let Some(snapshot) = snapshot {
            if snapshot.scan_id != request.scan_id
                || snapshot.parent_path != parent.path
                || snapshot.visible_paths != request.visible_paths
            {
                return Err(CoreError::invalid_input(
                    "the remainder snapshot scope does not match",
                ));
            }
            return Ok(snapshot.page(request.offset));
        }
        // Evicted lists restart at the first page with a new ID. The UI replaces
        // its previous rows rather than appending a page from a different listing.
    } else if request.offset != 0 {
        return Err(CoreError::invalid_input(
            "a remainder page requires a snapshot",
        ));
    }

    let root = current_platform()
        .canonicalize_no_links(Path::new(&parent.path))
        .map_err(CoreError::from)?;
    let (total_bytes, entries) =
        super::directory_loader::analysis_remainder_entries(&root, parent, request)?;
    let snapshot = Arc::new(RemainderSnapshot {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        scan_id: request.scan_id,
        parent_path: parent.path.clone(),
        visible_paths: request.visible_paths.clone(),
        total_bytes,
        entries,
    });
    let evicted = {
        let mut snapshots = snapshots()
            .lock()
            .map_err(|_| CoreError::operation_failed("the remainder snapshots are unavailable"))?;
        snapshots.push_back(Arc::clone(&snapshot));
        (snapshots.len() > SNAPSHOT_LIMIT).then(|| snapshots.pop_front())
    };
    // Free large listings outside the lock so another page can be served promptly.
    drop(evicted);
    Ok(snapshot.page(0))
}

pub(super) fn release(snapshot_id: u64) -> CoreResult<()> {
    let removed = {
        let mut snapshots = snapshots()
            .lock()
            .map_err(|_| CoreError::operation_failed("the remainder snapshots are unavailable"))?;
        snapshots
            .iter()
            .position(|snapshot| snapshot.id == snapshot_id)
            .and_then(|index| snapshots.remove(index))
    };
    drop(removed);
    Ok(())
}
