use mangodisk_core::ApplicationUninstallScanResult;
use std::sync::RwLock;

#[derive(Default)]
pub struct ApplicationUninstallCatalogCache {
    latest: RwLock<Option<CachedCatalog>>,
}

struct CachedCatalog {
    catalog: ApplicationUninstallScanResult,
    execution_fresh: bool,
}

impl ApplicationUninstallCatalogCache {
    pub fn clear(&self) {
        if let Ok(mut latest) = self.latest.write() {
            *latest = None;
        } else {
            log::warn!("application_uninstall_catalog_cache_clear_failed");
        }
    }

    pub fn replace(&self, catalog: &ApplicationUninstallScanResult) {
        match self.latest.write() {
            Ok(mut latest) => {
                *latest = Some(CachedCatalog {
                    catalog: catalog.clone(),
                    execution_fresh: true,
                })
            }
            Err(error) => {
                log::warn!("application_uninstall_catalog_cache_write_failed error={error}")
            }
        }
    }

    /// Closing processes updates running state, not installed inventory. Keep any
    /// invalidation from record removal until an actual inventory scan replaces it.
    pub fn update_after_close(&self, catalog: &ApplicationUninstallScanResult) {
        match self.latest.write() {
            Ok(mut latest) => {
                if let Some(cached) = latest
                    .as_mut()
                    .filter(|cached| cached.catalog.catalog_revision == catalog.catalog_revision)
                {
                    cached.catalog = catalog.clone();
                }
            }
            Err(_) => log::warn!("application_uninstall_catalog_close_update_failed"),
        }
    }

    /// Retain trusted display evidence for other rows while invalidating execution reuse.
    /// A registry mutation changes inventory revision; the next explicit preparation must
    /// obtain a fresh catalog, but deleting one record must not force an immediate scan.
    pub fn remove_record(&self, application_id: &str) {
        match self.latest.write() {
            Ok(mut latest) => {
                if let Some(cached) = latest.as_mut() {
                    let removed_count = cached.catalog.remove_record(application_id);
                    cached.execution_fresh = false;
                    log::info!("application_uninstall_catalog_record_removed removed_count={} remaining_count={} execution_cache_invalidated=true", removed_count, cached.catalog.candidates.len());
                }
            }
            Err(_) => log::warn!("application_uninstall_catalog_record_removal_failed"),
        }
    }

    pub fn find(&self, revision: &str) -> Option<ApplicationUninstallScanResult> {
        self.find_catalog(revision, true)
    }

    /// Read trusted identities for diagnostics or closing previously reviewed processes.
    /// These snapshots do not authorize uninstall preparation after inventory changes.
    pub fn find_snapshot(&self, revision: &str) -> Option<ApplicationUninstallScanResult> {
        self.find_catalog(revision, false)
    }

    fn find_catalog(
        &self,
        revision: &str,
        require_execution_fresh: bool,
    ) -> Option<ApplicationUninstallScanResult> {
        match self.latest.read() {
            Ok(latest) => latest
                .as_ref()
                .filter(|cached| !require_execution_fresh || cached.execution_fresh)
                .filter(|cached| cached.catalog.catalog_revision.as_deref() == Some(revision))
                .map(|cached| cached.catalog.clone()),
            Err(error) => {
                log::warn!("application_uninstall_catalog_cache_read_failed error={error}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(revision: &str) -> ApplicationUninstallScanResult {
        ApplicationUninstallScanResult {
            schema_version: 7,
            scanned_at_ms: 1,
            supported: true,
            execution_supported: true,
            catalog_actionable: true,
            inventory_complete: true,
            catalog_revision: Some(revision.to_string()),
            candidates: Vec::new(),
            ready_count: 0,
            blocked_count: 0,
            hidden_count: 0,
            related_directory_count: 0,
            related_path_scan_elapsed_ms: 0,
            elapsed_ms: 1,
        }
    }

    #[test]
    fn returns_only_the_matching_catalog_revision() {
        let cache = ApplicationUninstallCatalogCache::default();
        cache.replace(&catalog("revision-1"));

        assert!(cache.find("revision-1").is_some());
        assert!(cache.find("revision-2").is_none());
        cache.update_after_close(&catalog("revision-2"));
        assert!(cache.find("revision-1").is_some());
        cache.update_after_close(&catalog("revision-1"));
        assert!(cache.find("revision-1").is_some());
    }
    #[test]
    fn record_removal_retains_diagnostics_but_requires_fresh_preparation() {
        let cache = ApplicationUninstallCatalogCache::default();
        cache.replace(&catalog("revision-1"));
        cache.remove_record("removed");
        assert!(cache.find("revision-1").is_none());
        assert!(cache.find_snapshot("revision-1").is_some());
        assert!(cache.find_snapshot("unrelated").is_none());
        cache.update_after_close(&catalog("revision-1"));
        assert!(cache.find("revision-1").is_none());
        assert!(cache.find_snapshot("revision-1").is_some());
        cache.replace(&catalog("revision-2"));
        assert!(cache.find("revision-2").is_some());
        cache.clear();
        assert!(cache.find_snapshot("revision-2").is_none());
    }
}
