use mangodisk_core::ApplicationUninstallScanResult;
use std::sync::RwLock;

#[derive(Default)]
pub struct ApplicationUninstallCatalogCache {
    latest: RwLock<Option<ApplicationUninstallScanResult>>,
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
            Ok(mut latest) => *latest = Some(catalog.clone()),
            Err(error) => {
                log::warn!("application_uninstall_catalog_cache_write_failed error={error}")
            }
        }
    }

    pub fn find(&self, revision: &str) -> Option<ApplicationUninstallScanResult> {
        match self.latest.read() {
            Ok(latest) => latest
                .as_ref()
                .filter(|catalog| catalog.catalog_revision.as_deref() == Some(revision))
                .cloned(),
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
    }
}
