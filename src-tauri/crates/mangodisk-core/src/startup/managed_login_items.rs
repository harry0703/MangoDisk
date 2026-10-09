use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use mangodisk_platform::{
    PlatformStartupArtifact, PlatformStartupConfiguredState, PlatformStartupControlCapability,
    PlatformStartupCoverageStatus, PlatformStartupDiagnosticCode,
    PlatformStartupIdentityConfidence, PlatformStartupOwner, PlatformStartupRuntimeState,
    PlatformStartupScope, PlatformStartupSourceKind, PlatformStartupSourceResult,
    PlatformStartupSummarySource, PlatformStartupTarget, PlatformStartupTargetKind,
    PlatformStartupTrigger, PlatformStartupTrustState,
};
use serde::{Deserialize, Serialize};

use crate::shared::{application_paths, CoreError, CoreResult};

const SOURCE_ID: &str = "macos.managed_login_items";
const NATIVE_SOURCE_ID: &str = "macos.background_tasks";
const SCHEMA_VERSION: u32 = 1;
const MAX_DOCUMENT_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ManagedLoginItem {
    path: PathBuf,
    identity_key: String,
    display_name: String,
    version: Option<String>,
    publisher: Option<String>,
    #[serde(default)]
    restore_pending: bool,
}

#[derive(Deserialize, Serialize)]
struct Document {
    schema_version: u32,
    items: Vec<ManagedLoginItem>,
}

impl ManagedLoginItem {
    fn matches_native_artifact(&self, artifact: &PlatformStartupArtifact) -> bool {
        artifact.target.path.as_ref() == Some(&self.path)
            && artifact.target.identity_key == self.identity_key
    }

    fn from_artifact(artifact: &PlatformStartupArtifact) -> CoreResult<Self> {
        let path = artifact.target.path.as_ref().ok_or_else(|| {
            CoreError::invalid_input("background login item has no application path")
        })?;
        if !can_remember(artifact) {
            return Err(CoreError::invalid_input(
                "background login item cannot be restored safely",
            ));
        }
        Ok(Self {
            path: path.clone(),
            identity_key: artifact.target.identity_key.clone(),
            display_name: artifact.display_name.clone(),
            version: artifact.owner.version.clone(),
            publisher: artifact.owner.publisher.clone(),
            restore_pending: false,
        })
    }

    fn artifact(&self) -> PlatformStartupArtifact {
        let target_exists = self.path.try_exists();
        let mut diagnostics = Vec::new();
        let control_capability = match target_exists {
            Ok(true) => PlatformStartupControlCapability::Toggleable,
            Ok(false) => {
                diagnostics.push(PlatformStartupDiagnosticCode::MissingTarget);
                PlatformStartupControlCapability::SystemManaged
            }
            Err(_) => {
                diagnostics.push(PlatformStartupDiagnosticCode::StateUnavailable);
                PlatformStartupControlCapability::SystemManaged
            }
        };
        let provider_item_id = format!(
            "managed-login-item:{}",
            blake3::hash(format!("{}\0{}", self.path.display(), self.identity_key).as_bytes())
                .to_hex()
        );
        PlatformStartupArtifact {
            provider_item_id,
            source_kind: PlatformStartupSourceKind::BackgroundTask,
            scope: PlatformStartupScope::CurrentUser,
            triggers: vec![PlatformStartupTrigger::UserLogon],
            display_name: self.display_name.clone(),
            configuration_path: None,
            target: PlatformStartupTarget {
                kind: PlatformStartupTargetKind::Application,
                identity_key: self.identity_key.clone(),
                path: Some(self.path.clone()),
                executable_name: None,
                arguments: Vec::new(),
            },
            owner: PlatformStartupOwner {
                identity_key: Some(self.identity_key.clone()),
                name: Some(self.display_name.clone()),
                publisher: self.publisher.clone(),
                summary: None,
                summary_source: PlatformStartupSummarySource::BundleMetadata,
                version: self.version.clone(),
                icon_path: Some(self.path.clone()),
                confidence: if self.identity_key.starts_with("bundle:") {
                    PlatformStartupIdentityConfidence::Exact
                } else {
                    PlatformStartupIdentityConfidence::Probable
                },
            },
            configured_state: if self.restore_pending {
                PlatformStartupConfiguredState::Enabled
            } else {
                PlatformStartupConfiguredState::Disabled
            },
            runtime_state: PlatformStartupRuntimeState::Unknown,
            control_capability,
            trust: PlatformStartupTrustState::Unknown,
            modified_at_ms: None,
            diagnostics,
        }
    }
}

pub(super) fn can_remember(artifact: &PlatformStartupArtifact) -> bool {
    artifact.source_kind == PlatformStartupSourceKind::BackgroundTask
        && artifact.scope == PlatformStartupScope::CurrentUser
        && artifact.target.kind == PlatformStartupTargetKind::Application
        && artifact
            .target
            .path
            .as_deref()
            .is_some_and(is_application_path)
}

fn is_application_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        && !path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
}

fn document_path() -> CoreResult<PathBuf> {
    Ok(application_paths()?
        .data_directory()
        .join("startup")
        .join("managed-login-items.json"))
}

fn read(path: &Path) -> CoreResult<Vec<ManagedLoginItem>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(CoreError::persistence(format!(
                "failed to inspect managed login items: {error}"
            )))
        }
    };
    if !metadata.is_file() || metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(CoreError::persistence(
            "managed login item document is invalid",
        ));
    }
    let content = fs::read(path).map_err(|error| {
        CoreError::persistence(format!("failed to read managed login items: {error}"))
    })?;
    let document: Document = serde_json::from_slice(&content).map_err(|error| {
        CoreError::persistence(format!("failed to parse managed login items: {error}"))
    })?;
    let mut seen_paths = BTreeSet::new();
    if document.schema_version != SCHEMA_VERSION
        || document.items.iter().any(|item| {
            !is_application_path(&item.path)
                || item.identity_key.is_empty()
                || !seen_paths.insert(&item.path)
        })
    {
        return Err(CoreError::persistence(
            "managed login item document has an unsupported schema",
        ));
    }
    Ok(document.items)
}

fn write(path: &Path, items: &[ManagedLoginItem]) -> CoreResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::persistence("managed login item path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        CoreError::persistence(format!("failed to create startup data directory: {error}"))
    })?;
    let content = serde_json::to_vec(&Document {
        schema_version: SCHEMA_VERSION,
        items: items.to_vec(),
    })
    .map_err(|error| {
        CoreError::persistence(format!("failed to encode managed login items: {error}"))
    })?;
    if content.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(CoreError::persistence(
            "managed login item document exceeds its size limit",
        ));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        CoreError::persistence(format!("failed to stage managed login items: {error}"))
    })?;
    temporary
        .write_all(&content)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| {
            CoreError::persistence(format!("failed to write managed login items: {error}"))
        })?;
    temporary.persist(path).map_err(|error| {
        CoreError::persistence(format!(
            "failed to save managed login items: {}",
            error.error
        ))
    })?;
    Ok(())
}

pub(super) fn remember_disabled(artifacts: &[PlatformStartupArtifact]) -> CoreResult<()> {
    if artifacts.is_empty() {
        return Ok(());
    }
    let path = document_path()?;
    let mut items = read(&path)?;
    for artifact in artifacts {
        let item = ManagedLoginItem::from_artifact(artifact)?;
        items.retain(|existing| existing.path != item.path);
        items.push(item);
    }
    write(&path, &items)
}

pub(super) fn mark_restored(artifact: &PlatformStartupArtifact) -> CoreResult<()> {
    mark_restore_pending(artifact, true)
}

pub(super) fn mark_disabled(artifact: &PlatformStartupArtifact) -> CoreResult<()> {
    mark_restore_pending(artifact, false)
}

fn mark_restore_pending(
    artifact: &PlatformStartupArtifact,
    restore_pending: bool,
) -> CoreResult<()> {
    let path = document_path()?;
    mark_restore_pending_at(&path, artifact, restore_pending)
}

fn mark_restore_pending_at(
    path: &Path,
    artifact: &PlatformStartupArtifact,
    restore_pending: bool,
) -> CoreResult<()> {
    let Some(target_path) = artifact.target.path.as_ref() else {
        return Ok(());
    };
    let mut items = read(path)?;
    let mut changed = false;
    for item in &mut items {
        if item.path == *target_path
            && item.identity_key == artifact.target.identity_key
            && item.restore_pending != restore_pending
        {
            item.restore_pending = restore_pending;
            changed = true;
        }
    }
    if changed {
        write(path, &items)?;
        if restore_pending {
            log::info!(
                "startup_managed_login_item_waiting_for_btm target_path={}",
                mangodisk_platform::diagnostics::text(&target_path.to_string_lossy())
            );
        } else {
            log::info!(
                "startup_managed_login_item_disabled target_path={}",
                mangodisk_platform::diagnostics::text(&target_path.to_string_lossy())
            );
        }
    }
    Ok(())
}

pub(super) fn merge_results(
    results: &mut Vec<PlatformStartupSourceResult>,
    retire_native_ready: bool,
) -> CoreResult<()> {
    let path = document_path()?;
    let enabled_paths = if path.exists() {
        mangodisk_platform::macos_enabled_login_item_paths()
            .inspect_err(|error| {
                log::warn!(
                    "startup_managed_login_item_state_unavailable source_id={} reason={:?} detail={}",
                    SOURCE_ID, error.code(), mangodisk_platform::diagnostics::text(error.diagnostic())
                );
            })
            .ok()
    } else {
        None
    };
    merge_results_at(&path, results, retire_native_ready, enabled_paths.as_ref())
}

fn merge_results_at(
    path: &Path,
    results: &mut Vec<PlatformStartupSourceResult>,
    retire_native_ready: bool,
    enabled_paths: Option<&BTreeSet<PathBuf>>,
) -> CoreResult<()> {
    let mut items = read(path)?;
    let previous_items = items.clone();
    reconcile_results(results, &mut items, retire_native_ready, enabled_paths);
    if items != previous_items {
        write(path, &items)?;
        for item in &items {
            if previous_items.iter().any(|previous| {
                previous.path == item.path && previous.restore_pending != item.restore_pending
            }) {
                log::info!(
                    "startup_managed_login_item_state_reconciled target_path={} enabled={}",
                    mangodisk_platform::diagnostics::text(&item.path.to_string_lossy()),
                    item.restore_pending
                );
            }
        }
        for previous in &previous_items {
            if !items.iter().any(|item| item.path == previous.path) {
                log::info!(
                    "startup_managed_login_item_native_ready target_path={}",
                    mangodisk_platform::diagnostics::text(&previous.path.to_string_lossy())
                );
            }
        }
    }
    Ok(())
}

fn reconcile_results(
    results: &mut Vec<PlatformStartupSourceResult>,
    items: &mut Vec<ManagedLoginItem>,
    retire_native_ready: bool,
    enabled_paths: Option<&BTreeSet<PathBuf>>,
) {
    // Positive shared-list evidence is usable even in a partial scan. Absence is conclusive
    // only with complete coverage, or a matching BTM artifact whose toggleable capability
    // proves that the platform read its current shared-list state rather than old disposition.
    let shared_list = results
        .iter()
        .find(|source| source.source_id == "macos.login_items");
    for item in items.iter_mut() {
        // This verified snapshot also covers the interval where disabling removed the BTM
        // record and unrelated missing bookmarks made the descriptive source scan partial.
        if let Some(paths) = enabled_paths {
            item.restore_pending = paths.contains(&item.path);
            continue;
        }
        let shared_list_enabled = shared_list
            .into_iter()
            .flat_map(|source| &source.items)
            .any(|artifact| {
                (artifact.target.path.as_ref() == Some(&item.path)
                    || artifact.owner.icon_path.as_ref() == Some(&item.path))
                    && artifact.target.identity_key == item.identity_key
                    && artifact.configured_state == PlatformStartupConfiguredState::Enabled
            });
        if shared_list_enabled {
            item.restore_pending = true;
        } else if shared_list
            .is_some_and(|source| source.status == PlatformStartupCoverageStatus::Complete)
            || results
                .iter()
                .filter(|source| source.source_id == NATIVE_SOURCE_ID)
                .flat_map(|source| &source.items)
                .any(|artifact| {
                    item.matches_native_artifact(artifact)
                        && artifact.configured_state == PlatformStartupConfiguredState::Disabled
                        && artifact.control_capability
                            == PlatformStartupControlCapability::Toggleable
                })
        {
            item.restore_pending = false;
        }
    }
    items.retain(|item| {
        if !retire_native_ready {
            return true;
        }
        // Native orphan removal already validates the missing target and exact system record.
        // Hand ownership back before displaying that action so successful cleanup cannot leave
        // a synthetic leftover behind. Preflight still retains any selected managed identity.
        let native_ready = results
            .iter()
            .filter(|source| source.source_id == NATIVE_SOURCE_ID)
            .flat_map(|source| &source.items)
            .any(|artifact| {
                item.matches_native_artifact(artifact)
                    && (super::policy::supports_removal(artifact)
                        || (item.restore_pending
                            && artifact.configured_state
                                == PlatformStartupConfiguredState::Enabled
                            && artifact.control_capability
                                == PlatformStartupControlCapability::Toggleable))
            });
        !native_ready
    });
    if items.is_empty() {
        return;
    }
    for source in results
        .iter_mut()
        .filter(|source| source.source_id == NATIVE_SOURCE_ID)
    {
        source.items.retain(|artifact| {
            !items
                .iter()
                .any(|item| item.matches_native_artifact(artifact))
        });
    }
    results.push(PlatformStartupSourceResult {
        source_id: SOURCE_ID.to_owned(),
        required: false,
        status: PlatformStartupCoverageStatus::Complete,
        reason: None,
        items: items.iter().map(ManagedLoginItem::artifact).collect(),
        elapsed_ms: 0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: PathBuf) -> ManagedLoginItem {
        ManagedLoginItem {
            path,
            identity_key: "bundle:com.example.App".to_owned(),
            display_name: "Example".to_owned(),
            version: Some("1.0".to_owned()),
            publisher: None,
            restore_pending: false,
        }
    }

    fn native(
        item: &ManagedLoginItem,
        state: PlatformStartupConfiguredState,
    ) -> PlatformStartupSourceResult {
        let mut artifact = item.artifact();
        artifact.provider_item_id = "background-task:native-record".to_owned();
        artifact.configured_state = state;
        artifact.control_capability = PlatformStartupControlCapability::Toggleable;
        PlatformStartupSourceResult {
            source_id: NATIVE_SOURCE_ID.to_owned(),
            required: false,
            status: PlatformStartupCoverageStatus::Complete,
            reason: None,
            items: vec![artifact],
            elapsed_ms: 0,
        }
    }

    fn shared_list(items: Vec<PlatformStartupArtifact>) -> PlatformStartupSourceResult {
        PlatformStartupSourceResult {
            source_id: "macos.login_items".to_owned(),
            required: true,
            status: PlatformStartupCoverageStatus::Complete,
            reason: None,
            items,
            elapsed_ms: 0,
        }
    }

    #[test]
    fn external_disable_updates_and_persists_managed_state() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("Example.app");
        fs::create_dir(&app).unwrap();
        let mut item = item(app);
        item.restore_pending = true;
        let document = directory.path().join("managed-login-items.json");
        write(&document, &[item]).unwrap();
        let mut sources = vec![shared_list(Vec::new())];

        merge_results_at(&document, &mut sources, true, None).unwrap();

        assert!(!read(&document).unwrap()[0].restore_pending);
        assert_eq!(
            sources[1].items[0].configured_state,
            PlatformStartupConfiguredState::Disabled
        );
        assert_eq!(
            sources[1].items[0].control_capability,
            PlatformStartupControlCapability::Toggleable
        );
    }

    #[test]
    fn native_disabled_state_reconciles_when_unrelated_login_items_are_unresolved() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("Example.app");
        fs::create_dir(&app).unwrap();
        let mut item = item(app);
        item.restore_pending = true;
        let mut login_items = shared_list(Vec::new());
        login_items.status = PlatformStartupCoverageStatus::Partial;
        login_items.reason = Some(mangodisk_platform::PlatformStartupCoverageReason::InvalidData);
        let mut sources = vec![
            login_items,
            native(&item, PlatformStartupConfiguredState::Disabled),
        ];
        let mut items = vec![item];

        reconcile_results(&mut sources, &mut items, false, None);

        assert!(!items[0].restore_pending);
        assert_eq!(
            sources[2].items[0].configured_state,
            PlatformStartupConfiguredState::Disabled
        );
    }

    #[test]
    fn verified_membership_reconciles_external_disable_without_btm_record() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("Example.app");
        fs::create_dir(&app).unwrap();
        let mut item = item(app);
        item.restore_pending = true;
        let document = directory.path().join("managed-login-items.json");
        write(&document, &[item]).unwrap();
        let mut login_items = shared_list(Vec::new());
        login_items.status = PlatformStartupCoverageStatus::Partial;
        login_items.reason = Some(mangodisk_platform::PlatformStartupCoverageReason::InvalidData);
        let mut sources = vec![login_items];

        merge_results_at(&document, &mut sources, true, Some(&BTreeSet::new())).unwrap();

        assert!(!read(&document).unwrap()[0].restore_pending);
        assert_eq!(
            sources[1].items[0].configured_state,
            PlatformStartupConfiguredState::Disabled
        );
        assert_eq!(
            sources[1].items[0].control_capability,
            PlatformStartupControlCapability::Toggleable
        );
    }

    #[test]
    fn verified_membership_overrides_older_scan_in_both_directions() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("Example.app");
        fs::create_dir(&app).unwrap();
        let item = item(app);
        let mut items = vec![item.clone()];
        let mut sources = vec![shared_list(Vec::new())];
        let enabled = BTreeSet::from([item.path.clone()]);

        reconcile_results(&mut sources, &mut items, false, Some(&enabled));

        assert!(items[0].restore_pending);
        let mut sources = vec![native(&item, PlatformStartupConfiguredState::Enabled)];
        reconcile_results(&mut sources, &mut items, true, Some(&BTreeSet::new()));
        assert!(!items[0].restore_pending);
        assert!(sources[0].items.is_empty());
        assert_eq!(
            sources[1].items[0].configured_state,
            PlatformStartupConfiguredState::Disabled
        );
    }

    #[test]
    fn incomplete_shared_list_does_not_infer_external_disable() {
        let mut item = item(PathBuf::from("/Applications/Example.app"));
        item.restore_pending = true;
        for status in [
            PlatformStartupCoverageStatus::Partial,
            PlatformStartupCoverageStatus::Unavailable,
            PlatformStartupCoverageStatus::Cancelled,
        ] {
            let mut login_items = shared_list(Vec::new());
            login_items.status = status;
            let mut btm = native(&item, PlatformStartupConfiguredState::Disabled);
            btm.items[0].control_capability = PlatformStartupControlCapability::SystemManaged;
            let mut sources = vec![login_items, btm];
            let mut items = vec![item.clone()];

            reconcile_results(&mut sources, &mut items, true, None);

            assert!(items[0].restore_pending, "incomplete coverage: {status:?}");
        }
    }

    #[test]
    fn native_removable_orphan_retires_managed_record_and_keeps_cleanup_action() {
        let directory = tempfile::tempdir().unwrap();
        let item = item(directory.path().join("Removed.app"));
        let document = directory.path().join("managed-login-items.json");
        write(&document, std::slice::from_ref(&item)).unwrap();
        let mut orphan = native(&item, PlatformStartupConfiguredState::Disabled);
        orphan.items[0].control_capability = PlatformStartupControlCapability::RemoveOnly;
        let mut sources = vec![orphan.clone()];

        merge_results_at(&document, &mut sources, true, None).unwrap();

        assert!(read(&document).unwrap().is_empty());
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].items, orphan.items);
        assert!(super::super::policy::supports_removal(&sources[0].items[0]));
        let mut after_removal = vec![shared_list(Vec::new())];
        merge_results_at(&document, &mut after_removal, true, None).unwrap();
        assert_eq!(after_removal.len(), 1);
        assert!(after_removal[0].items.is_empty());
    }

    #[test]
    fn orphan_handoff_requires_matching_identity_and_native_removal_capability() {
        let directory = tempfile::tempdir().unwrap();
        let item = item(directory.path().join("Removed.app"));
        let mut orphan = native(&item, PlatformStartupConfiguredState::Disabled);
        orphan.items[0].control_capability = PlatformStartupControlCapability::RemoveOnly;
        orphan.items[0].target.identity_key = "bundle:com.example.Replaced".to_owned();
        let mut items = vec![item.clone()];
        let mut sources = vec![orphan];

        reconcile_results(&mut sources, &mut items, true, None);

        assert_eq!(items, vec![item.clone()]);
        assert_eq!(sources[0].items.len(), 1);
        let mut read_only = native(&item, PlatformStartupConfiguredState::Disabled);
        read_only.items[0].control_capability = PlatformStartupControlCapability::SystemManaged;
        let mut sources = vec![read_only];
        reconcile_results(&mut sources, &mut items, true, None);
        assert_eq!(items, vec![item]);
        assert_eq!(
            sources[1].items[0].control_capability,
            PlatformStartupControlCapability::SystemManaged
        );
    }

    #[test]
    fn disabled_native_record_is_replaced_by_restorable_managed_item() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("Example.app");
        fs::create_dir(&app).unwrap();
        let item = item(app);
        let mut items = vec![item.clone()];
        let mut sources = vec![native(&item, PlatformStartupConfiguredState::Disabled)];

        reconcile_results(&mut sources, &mut items, true, None);

        assert_eq!(items, vec![item]);
        assert!(sources[0].items.is_empty());
        assert_eq!(sources[1].source_id, SOURCE_ID);
        assert_eq!(
            sources[1].items[0].configured_state,
            PlatformStartupConfiguredState::Disabled
        );
        assert_eq!(
            sources[1].items[0].control_capability,
            PlatformStartupControlCapability::Toggleable
        );
    }

    #[test]
    fn shared_list_restoration_stays_visible_until_btm_is_controllable() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("Example.app");
        fs::create_dir(&app).unwrap();
        let item = item(app);
        let mut items = vec![item.clone()];
        let mut login_item = native(&item, PlatformStartupConfiguredState::Enabled);
        login_item.source_id = "macos.login_items".to_owned();
        login_item.items[0].target.path = Some(item.path.join("Contents/MacOS/Example"));
        let mut sources = vec![login_item];

        reconcile_results(&mut sources, &mut items, true, None);

        assert!(items[0].restore_pending);
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].items.len(), 1);
        assert_eq!(
            sources[1].items[0].configured_state,
            PlatformStartupConfiguredState::Enabled
        );
        assert_eq!(
            sources[1].items[0].control_capability,
            PlatformStartupControlCapability::Toggleable
        );
        assert!(!sources[1].items[0]
            .diagnostics
            .contains(&PlatformStartupDiagnosticCode::MissingTarget));

        let mut btm = native(&item, PlatformStartupConfiguredState::Enabled);
        btm.items[0].control_capability = PlatformStartupControlCapability::SystemManaged;
        let mut sources = vec![btm];
        reconcile_results(&mut sources, &mut items, true, None);
        assert_eq!(items.len(), 1);
        assert!(sources[0].items.is_empty());

        let mut sources = vec![native(&item, PlatformStartupConfiguredState::Enabled)];
        reconcile_results(&mut sources, &mut items, true, None);
        assert!(items.is_empty());
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].items.len(), 1);
    }

    #[test]
    fn stale_enabled_btm_disposition_does_not_erase_recovery_record() {
        let item = item(PathBuf::from("/Applications/Example.app"));
        let mut items = vec![item.clone()];
        let mut sources = vec![native(&item, PlatformStartupConfiguredState::Enabled)];

        reconcile_results(&mut sources, &mut items, true, None);

        assert_eq!(items, vec![item]);
        assert!(sources[0].items.is_empty());
        assert_eq!(sources[1].source_id, SOURCE_ID);
    }

    #[test]
    fn verified_restore_stays_visible_until_native_record_appears() {
        let mut item = item(PathBuf::from("/Applications/Example.app"));
        item.restore_pending = true;
        let mut items = vec![item.clone()];
        let mut sources = vec![PlatformStartupSourceResult::unavailable(
            NATIVE_SOURCE_ID,
            false,
            mangodisk_platform::PlatformStartupCoverageReason::AccessDenied,
        )];

        reconcile_results(&mut sources, &mut items, true, None);

        assert_eq!(items, vec![item.clone()]);
        assert_eq!(
            sources[1].items[0].configured_state,
            PlatformStartupConfiguredState::Enabled
        );

        let mut sources = vec![native(&item, PlatformStartupConfiguredState::Enabled)];
        reconcile_results(&mut sources, &mut items, true, None);

        assert!(items.is_empty());
        assert_eq!(sources[0].items.len(), 1);
        assert_eq!(sources.len(), 1);
    }

    #[test]
    fn preflight_keeps_selected_managed_identity_after_native_record_appears() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("Example.app");
        fs::create_dir(&app).unwrap();
        let mut item = item(app);
        item.restore_pending = true;
        let document = directory.path().join("managed-login-items.json");
        write(&document, std::slice::from_ref(&item)).unwrap();
        let mut sources = vec![native(&item, PlatformStartupConfiguredState::Enabled)];

        merge_results_at(&document, &mut sources, false, None).unwrap();

        assert_eq!(read(&document).unwrap(), vec![item]);
        assert!(sources[0].items.is_empty());
        assert_eq!(
            sources[1].items[0].configured_state,
            PlatformStartupConfiguredState::Enabled
        );
        assert_eq!(
            sources[1].items[0].control_capability,
            PlatformStartupControlCapability::Toggleable
        );
    }

    #[test]
    fn managed_record_survives_native_record_disappearing() {
        let item = item(PathBuf::from("/Applications/Example.app"));
        let mut items = vec![item];
        let mut sources = vec![PlatformStartupSourceResult::unavailable(
            NATIVE_SOURCE_ID,
            false,
            mangodisk_platform::PlatformStartupCoverageReason::AccessDenied,
        )];

        reconcile_results(&mut sources, &mut items, true, None);

        assert_eq!(sources[1].items.len(), 1);
        assert!(sources[1].items[0]
            .diagnostics
            .contains(&PlatformStartupDiagnosticCode::MissingTarget));
    }

    #[test]
    fn document_round_trip_preserves_restoration_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("managed-login-items.json");
        let items = vec![item(PathBuf::from("/Applications/Example.app"))];

        write(&path, &items).unwrap();

        assert_eq!(read(&path).unwrap(), items);
    }

    #[test]
    fn external_restore_persists_enabled_state_before_btm_appears() {
        let directory = tempfile::tempdir().unwrap();
        let document = directory.path().join("managed-login-items.json");
        let app = directory.path().join("Example.app");
        fs::create_dir(&app).unwrap();
        let item = item(app);
        write(&document, std::slice::from_ref(&item)).unwrap();
        let mut login_item = native(&item, PlatformStartupConfiguredState::Enabled);
        login_item.source_id = "macos.login_items".to_owned();
        login_item.status = PlatformStartupCoverageStatus::Partial;
        let mut sources = vec![login_item];

        merge_results_at(&document, &mut sources, true, None).unwrap();

        assert!(read(&document).unwrap()[0].restore_pending);
        assert_eq!(
            sources[1].items[0].configured_state,
            PlatformStartupConfiguredState::Enabled
        );
    }

    #[test]
    fn restored_item_can_be_disabled_again_before_btm_appears() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("Example.app");
        fs::create_dir(&app).unwrap();
        let mut item = item(app);
        item.restore_pending = true;
        let document = directory.path().join("managed-login-items.json");
        write(&document, std::slice::from_ref(&item)).unwrap();

        mark_restore_pending_at(&document, &item.artifact(), false).unwrap();
        let mut sources = vec![native(&item, PlatformStartupConfiguredState::Enabled)];
        let mut items = read(&document).unwrap();
        reconcile_results(&mut sources, &mut items, true, None);

        assert!(!items[0].restore_pending);
        assert_eq!(
            sources[1].items[0].configured_state,
            PlatformStartupConfiguredState::Disabled
        );
        assert_eq!(
            sources[1].items[0].control_capability,
            PlatformStartupControlCapability::Toggleable
        );
    }

    #[test]
    fn duplicate_managed_paths_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("managed-login-items.json");
        let item = item(PathBuf::from("/Applications/Example.app"));
        write(&path, &[item.clone(), item]).unwrap();

        assert!(read(&path).is_err());
    }

    #[test]
    fn only_application_bundles_are_saved_for_restoration() {
        let item = item(PathBuf::from("/Applications/Example.app"));
        let mut artifact = item.artifact();
        assert!(can_remember(&artifact));

        artifact.target.path = Some(PathBuf::from("/usr/local/bin/example"));
        assert!(!can_remember(&artifact));
    }
}
