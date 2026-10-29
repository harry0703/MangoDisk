use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) enum Omission {
    Excluded,
    LinkOrMount,
    Other,
}

impl AnalysisTraversal<'_> {
    pub(super) fn skip(&self, aggregate: &mut DirectoryAggregate, reason: Omission) {
        aggregate.skipped_count += 1;
        match reason {
            Omission::Excluded => aggregate.omissions.excluded += 1,
            Omission::LinkOrMount => aggregate.omissions.link_or_mount += 1,
            Omission::Other => aggregate.omissions.other += 1,
        }
    }

    pub(super) fn skip_io(
        &self,
        aggregate: &mut DirectoryAggregate,
        path: &Path,
        error: &std::io::Error,
    ) {
        aggregate.skipped_count += 1;
        let denied = error.kind() == std::io::ErrorKind::PermissionDenied;
        if denied {
            aggregate.omissions.permission_denied += 1;
        } else {
            aggregate.omissions.read_errors += 1;
        }
        if self.purpose == ScanPurpose::Analysis
            && self
                .skip_log_budget
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                    (count < 16).then_some(count + 1)
                })
                .is_ok()
        {
            log::warn!(
                "analysis_item_omitted root={} path={} reason={} native_code={:?} error={}",
                diagnostic_path(self.scan_root),
                diagnostic_path(path),
                if denied {
                    "permissionDenied"
                } else {
                    "readError"
                },
                error.raw_os_error(),
                mangodisk_platform::diagnostics::text(error)
            );
        }
    }
}
