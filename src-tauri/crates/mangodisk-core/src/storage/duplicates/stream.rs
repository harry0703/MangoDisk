use std::time::Instant;

use crate::storage::duplicates::{DuplicateGroup, DuplicateGroupBatch};

const VISIBLE_GROUP_LIMIT: usize = 40;
const BATCH_LIMIT: usize = 16;
const EMIT_INTERVAL_MS: u64 = 100;

/// Batches early duplicate groups for responsive UI updates without making the
/// WebView event stream the authoritative result store. The session remains the
/// complete paginated source after scanning finishes.
pub(super) struct DuplicateGroupStream {
    operation_id: u64,
    callback: Box<dyn Fn(DuplicateGroupBatch) + Send + Sync>,
    started_at: Instant,
    last_emit_ms: u64,
    sequence: u64,
    visible_group_count: usize,
    pending: Vec<DuplicateGroup>,
    emitted_batch_count: u64,
    emitted_group_count: u64,
    first_group_ms: Option<u64>,
    found_group_count: u64,
    found_file_count: u64,
    found_total_bytes: u64,
    found_reclaimable_bytes: u64,
}

impl DuplicateGroupStream {
    pub(super) fn new(
        operation_id: u64,
        callback: impl Fn(DuplicateGroupBatch) + Send + Sync + 'static,
    ) -> Self {
        Self {
            operation_id,
            callback: Box::new(callback),
            started_at: Instant::now(),
            last_emit_ms: 0,
            sequence: 0,
            visible_group_count: 0,
            pending: Vec::new(),
            emitted_batch_count: 0,
            emitted_group_count: 0,
            first_group_ms: None,
            found_group_count: 0,
            found_file_count: 0,
            found_total_bytes: 0,
            found_reclaimable_bytes: 0,
        }
    }

    pub(super) fn push(&mut self, groups: Vec<DuplicateGroup>) {
        if groups.is_empty() {
            return;
        }
        let current_ms = self.elapsed_ms();
        self.first_group_ms.get_or_insert(current_ms);
        for group in groups {
            self.found_group_count = self.found_group_count.saturating_add(1);
            self.found_file_count = self.found_file_count.saturating_add(
                u64::try_from(group.entries.len())
                    .unwrap_or(u64::MAX)
                    .saturating_mul(group.file_count_per_entry),
            );
            self.found_total_bytes = self
                .found_total_bytes
                .saturating_add(group.total_allocated_bytes());
            self.found_reclaimable_bytes = self
                .found_reclaimable_bytes
                .saturating_add(group.reclaimable_bytes);
            if self.visible_group_count < VISIBLE_GROUP_LIMIT {
                self.visible_group_count = self.visible_group_count.saturating_add(1);
                self.pending.push(group);
            }
        }

        // Deliver the first group immediately. Later updates are bounded and
        // throttled so many small groups cannot overwhelm the WebView.
        let first_batch = self.sequence == 0;
        let interval_elapsed = current_ms.saturating_sub(self.last_emit_ms) >= EMIT_INTERVAL_MS;
        if first_batch || interval_elapsed {
            self.emit_pending(current_ms);
        }
    }

    pub(super) fn finish(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let current_ms = self.elapsed_ms();
        if self.sequence == 0 || current_ms.saturating_sub(self.last_emit_ms) >= EMIT_INTERVAL_MS {
            self.emit_pending(current_ms);
        } else {
            // The final command response already contains the authoritative
            // first page, so a duplicate event burst has no visible benefit.
            self.pending.clear();
        }
    }

    pub(super) const fn metrics(&self) -> (u64, u64, Option<u64>) {
        (
            self.emitted_batch_count,
            self.emitted_group_count,
            self.first_group_ms,
        )
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn emit_pending(&mut self, current_ms: u64) {
        if self.pending.is_empty() {
            return;
        }
        self.sequence = self.sequence.saturating_add(1);
        self.last_emit_ms = current_ms;
        let take_count = self.pending.len().min(BATCH_LIMIT);
        let groups = self.pending.drain(..take_count).collect::<Vec<_>>();
        self.emitted_batch_count = self.emitted_batch_count.saturating_add(1);
        self.emitted_group_count = self
            .emitted_group_count
            .saturating_add(u64::try_from(groups.len()).unwrap_or(u64::MAX));
        (self.callback)(DuplicateGroupBatch {
            operation_id: self.operation_id,
            sequence: self.sequence,
            groups,
            found_group_count: self.found_group_count,
            found_file_count: self.found_file_count,
            found_total_bytes: self.found_total_bytes,
            found_reclaimable_bytes: self.found_reclaimable_bytes,
            elapsed_ms: current_ms,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::storage::duplicates::{DuplicateFileEntry, DuplicateGroupKind};

    fn group_fixture(index: u64) -> DuplicateGroup {
        let hash = format!("{index:064x}");
        DuplicateGroup {
            id: hash.chars().take(16).collect(),
            hash,
            kind: DuplicateGroupKind::File,
            bytes_per_file: 1,
            file_count_per_entry: 1,
            reclaimable_bytes: 1,
            entries: ["a", "b"]
                .into_iter()
                .map(|suffix| DuplicateFileEntry {
                    name: format!("{index}-{suffix}.bin"),
                    parent_path: "/fixture".to_string(),
                    path: format!("/fixture/{index}-{suffix}.bin"),
                    bytes: 1,
                    allocated_bytes: 1,
                    modified_at_ms: Some(0),
                    delete_policy:
                        crate::storage::duplicates::DuplicateEntryDeletePolicy::Cleanable,
                })
                .collect(),
        }
    }

    #[test]
    fn batch_and_first_group_times_are_operation_relative() {
        let batches = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&batches);
        let mut stream = DuplicateGroupStream::new(7, move |batch| {
            captured.lock().unwrap().push(batch);
        });
        stream.started_at = Instant::now() - std::time::Duration::from_secs(18);
        stream.push(vec![group_fixture(1)]);
        let batches = batches.lock().unwrap();
        assert_eq!(batches.len(), 1);
        assert!((18_000..19_000).contains(&batches[0].elapsed_ms));
        assert!((18_000..19_000).contains(&stream.metrics().2.unwrap()));
    }

    #[test]
    fn batches_bound_the_first_page_without_forcing_a_final_flush() {
        let batches = Arc::new(Mutex::new(Vec::<DuplicateGroupBatch>::new()));
        let batches_for_callback = Arc::clone(&batches);
        let mut stream = DuplicateGroupStream::new(7, move |batch| {
            batches_for_callback
                .lock()
                .expect("the streaming batch lock should not be poisoned")
                .push(batch);
        });

        stream.push((0..20).map(group_fixture).collect());
        // Pin the throttle window in the future so the test does not depend on
        // scheduler speed. The final command response supplies pending groups.
        stream.last_emit_ms = u64::MAX;
        stream.push((20..60).map(group_fixture).collect());
        stream.finish();

        let batches = batches
            .lock()
            .expect("streaming batches should remain readable");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].groups.len(), BATCH_LIMIT);
        assert_eq!(stream.visible_group_count, VISIBLE_GROUP_LIMIT);
        assert_eq!(stream.found_group_count, 60);
        assert_eq!(stream.emitted_batch_count, 1);
    }
}
