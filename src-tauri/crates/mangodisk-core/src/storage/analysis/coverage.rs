use mangodisk_platform::ScanOmissionCounts;
use serde::Serialize;

pub const ANALYSIS_SCAN_POLICY_VERSION: u32 = 2;

pub(crate) fn entry_size_status(
    aggregate: crate::storage::index::cache::DirectoryAggregate,
) -> super::AnalysisEntrySizeStatus {
    let counts = aggregate
        .omissions
        .with_unclassified(aggregate.skipped_count);
    if counts.total() == 0 {
        super::AnalysisEntrySizeStatus::Complete
    } else if aggregate.bytes == 0
        && counts.permission_denied + counts.read_errors + counts.other > 0
    {
        super::AnalysisEntrySizeStatus::Unavailable
    } else {
        super::AnalysisEntrySizeStatus::Partial
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisCoverage {
    pub schema_version: u32,
    pub omissions: ScanOmissionCounts,
    pub elevated: Option<bool>,
}

pub(crate) fn process_elevation() -> Option<bool> {
    #[cfg(windows)]
    {
        mangodisk_platform::windows_process_is_elevated().ok()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

impl AnalysisCoverage {
    pub(crate) fn new(omissions: ScanOmissionCounts, skipped: u64) -> Self {
        Self {
            schema_version: 1,
            omissions: omissions.with_unclassified(skipped),
            elevated: process_elevation(),
        }
    }
}

pub(crate) fn configuration_fingerprint(exclusions: [u8; 32], elevation: Option<bool>) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"analysis-scan-policy");
    hash.update(&ANALYSIS_SCAN_POLICY_VERSION.to_le_bytes());
    hash.update(&exclusions);
    // A cache measured by an unelevated reader must not mask newly readable scopes.
    hash.update(&[match elevation {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }]);
    *hash.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_policy_and_privilege_cannot_reuse_legacy_or_unelevated_totals() {
        let ordinary = configuration_fingerprint([0; 32], Some(false));
        assert_ne!(ordinary, [0; 32]);
        assert_ne!(ordinary, configuration_fingerprint([0; 32], Some(true)));
        assert_ne!(ordinary, configuration_fingerprint([0; 32], None));
        assert_ne!(ordinary, configuration_fingerprint([1; 32], Some(false)));
        assert_eq!(ordinary, configuration_fingerprint([0; 32], Some(false)));
    }

    #[test]
    fn native_unclassified_skips_are_never_reported_as_permission_failures() {
        let coverage = AnalysisCoverage::new(
            ScanOmissionCounts {
                excluded: 2,
                ..Default::default()
            },
            5,
        );
        assert_eq!(coverage.omissions.excluded, 2);
        assert_eq!(coverage.omissions.other, 3);
        assert_eq!(coverage.omissions.permission_denied, 0);
        assert_eq!(coverage.omissions.total(), 5);
    }
}
