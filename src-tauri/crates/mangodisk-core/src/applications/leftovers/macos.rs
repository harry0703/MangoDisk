use std::{
    fmt, fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use mangodisk_platform::{current_platform, Platform};
use plist::{Dictionary, Value};

use crate::{
    applications::catalog::{ApplicationInventory, ProcessSnapshot},
    filesystem::metadata::modified_ms,
};

use super::models::{
    ApplicationLeftoverCandidate, ApplicationLeftoverConfidence, ApplicationLeftoverEvidence,
    ApplicationLeftoverSource,
};

const CONTAINER_METADATA_FILE: &str = ".com.apple.containermanagerd.metadata.plist";

pub(super) struct CandidateScan {
    pub(super) candidates: Vec<ApplicationLeftoverCandidate>,
    pub(super) skipped_count: u64,
    pub(super) access_denied_count: u64,
}

struct ContainerIdentity {
    identifier: String,
    former_bundle: PathBuf,
    application_name: String,
}

struct AssociatedPath {
    source: ApplicationLeftoverSource,
    path: PathBuf,
}

#[derive(Debug)]
enum CandidateEvaluationError {
    AccessDenied,
    Rejected(String),
}

impl fmt::Display for CandidateEvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccessDenied => formatter.write_str("Apple container metadata access was denied"),
            Self::Rejected(reason) => formatter.write_str(reason),
        }
    }
}

impl From<String> for CandidateEvaluationError {
    fn from(reason: String) -> Self {
        Self::Rejected(reason)
    }
}

#[derive(Default)]
struct PathSnapshot {
    bytes: u64,
    file_count: u64,
    skipped_count: u64,
    access_denied_count: u64,
    fingerprint: Option<String>,
}

pub(super) fn scan_candidates(
    inventory: &ApplicationInventory,
    processes: &ProcessSnapshot,
) -> Result<CandidateScan, String> {
    let home = current_platform()
        .user_directories()
        .map_err(|error| error.to_string())?
        .home_directory()
        .to_path_buf();
    let root = home.join("Library/Containers");
    if !root.exists() {
        return Ok(CandidateScan {
            candidates: Vec::new(),
            skipped_count: 0,
            access_denied_count: 0,
        });
    }
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return Ok(CandidateScan {
                candidates: Vec::new(),
                skipped_count: 1,
                access_denied_count: 1,
            });
        }
        Err(error) => {
            return Err(format!(
                "failed to enumerate macOS user containers: {error}"
            ));
        }
    };
    let mut candidates = Vec::new();
    let mut skipped_count = 0_u64;
    let mut access_denied_count = 0_u64;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                skipped_count += 1;
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    access_denied_count += 1;
                }
                continue;
            }
        };
        let path = entry.path();
        match evaluate_container(&root, &path, inventory, processes) {
            Ok(Some(mut discovered)) => candidates.append(&mut discovered),
            Ok(None) => {}
            Err(CandidateEvaluationError::AccessDenied) => {
                skipped_count += 1;
                access_denied_count += 1;
            }
            Err(CandidateEvaluationError::Rejected(error)) => {
                skipped_count += 1;
                log::debug!(
                    "application_leftover_candidate_skipped path={} reason={}",
                    crate::filesystem::metadata::diagnostic_path(&path),
                    error
                );
            }
        }
    }
    candidates.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.application_name.cmp(&right.application_name))
    });
    Ok(CandidateScan {
        candidates,
        skipped_count,
        access_denied_count,
    })
}

fn evaluate_container(
    root: &Path,
    path: &Path,
    inventory: &ApplicationInventory,
    processes: &ProcessSnapshot,
) -> Result<Option<Vec<ApplicationLeftoverCandidate>>, CandidateEvaluationError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            CandidateEvaluationError::AccessDenied
        } else {
            CandidateEvaluationError::Rejected(format!(
                "failed to read container metadata: {error}"
            ))
        }
    })?;
    if !metadata.is_dir() || current_platform().is_link_like(&metadata) {
        return Err("container is not a regular directory".to_string().into());
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err("container is not owned by the current user"
            .to_string()
            .into());
    }
    if path.parent() != Some(root) {
        return Err("container is outside the expected user scope"
            .to_string()
            .into());
    }

    let identity = read_container_identity(path)?;
    let directory_identifier = path
        .file_name()
        .map(|value| value.to_string_lossy())
        .ok_or_else(|| "container has no identifier".to_string())?;
    if directory_identifier.as_ref() != identity.identifier {
        return Err("container identifier does not match its directory"
            .to_string()
            .into());
    }
    if inventory.has_application_identifier(&identity.identifier) || identity.former_bundle.exists()
    {
        return Ok(None);
    }
    if !processes
        .matching_processes(&process_identity_names(&identity))
        .is_empty()
    {
        return Ok(None);
    }

    let library = root
        .parent()
        .ok_or_else(|| "container root has no Library parent".to_string())?;
    let mut associations = associated_paths(library, &identity.identifier);
    associations.insert(
        0,
        AssociatedPath {
            source: ApplicationLeftoverSource::SandboxContainer,
            path: path.to_path_buf(),
        },
    );
    let mut candidates = Vec::new();
    for association in associations {
        match candidate_from_path(&identity, association) {
            Ok(Some(candidate)) => candidates.push(candidate),
            Ok(None) => {}
            Err(CandidateEvaluationError::AccessDenied) => {
                return Err(CandidateEvaluationError::AccessDenied);
            }
            Err(CandidateEvaluationError::Rejected(error)) => {
                log::debug!(
                    "application_leftover_association_skipped identifier_digest={} reason={}",
                    &blake3::hash(identity.identifier.as_bytes()).to_hex()[..16],
                    error
                );
            }
        }
    }
    if candidates
        .iter()
        .all(|candidate| candidate.source != ApplicationLeftoverSource::SandboxContainer)
    {
        return Err("verified container candidate is unavailable"
            .to_string()
            .into());
    }
    Ok(Some(candidates))
}

fn associated_paths(library: &Path, identifier: &str) -> Vec<AssociatedPath> {
    [
        (
            ApplicationLeftoverSource::ApplicationSupport,
            library.join("Application Support").join(identifier),
        ),
        (
            ApplicationLeftoverSource::Preferences,
            library
                .join("Preferences")
                .join(format!("{identifier}.plist")),
        ),
        (
            ApplicationLeftoverSource::Logs,
            library.join("Logs").join(identifier),
        ),
        (
            ApplicationLeftoverSource::SavedState,
            library
                .join("Saved Application State")
                .join(format!("{identifier}.savedState")),
        ),
        (
            ApplicationLeftoverSource::WebData,
            library.join("WebKit").join(identifier),
        ),
        (
            ApplicationLeftoverSource::WebData,
            library.join("HTTPStorages").join(identifier),
        ),
        (
            ApplicationLeftoverSource::ApplicationScripts,
            library.join("Application Scripts").join(identifier),
        ),
    ]
    .into_iter()
    .map(|(source, path)| AssociatedPath { source, path })
    .collect()
}

fn candidate_from_path(
    identity: &ContainerIdentity,
    association: AssociatedPath,
) -> Result<Option<ApplicationLeftoverCandidate>, CandidateEvaluationError> {
    let metadata = match fs::symlink_metadata(&association.path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(CandidateEvaluationError::AccessDenied);
        }
        Err(error) => {
            return Err(CandidateEvaluationError::Rejected(format!(
                "failed to read associated path metadata: {error}"
            )));
        }
    };
    if current_platform().is_link_like(&metadata) {
        return Err("associated path is a link".to_string().into());
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err("associated path is not owned by the current user"
            .to_string()
            .into());
    }
    if !metadata.is_dir() && !metadata.is_file() {
        return Err("associated path is not a regular file or directory"
            .to_string()
            .into());
    }
    current_platform()
        .validate_path_no_links(&association.path)
        .map_err(|error| CandidateEvaluationError::from(error.to_string()))?;
    let snapshot = snapshot_path(&association.path, association.source);
    if snapshot.access_denied_count > 0 {
        return Err(CandidateEvaluationError::AccessDenied);
    }
    let Some(fingerprint) = complete_fingerprint(&snapshot) else {
        return Err("associated path snapshot is incomplete".to_string().into());
    };
    if snapshot.file_count == 0 {
        return Ok(None);
    }
    let mut evidence = vec![
        ApplicationLeftoverEvidence::ContainerMetadataVerified,
        ApplicationLeftoverEvidence::FormerBundleMissing,
        ApplicationLeftoverEvidence::InstalledOwnerAbsent,
    ];
    if association.source != ApplicationLeftoverSource::SandboxContainer {
        evidence.push(ApplicationLeftoverEvidence::ExactIdentifierAssociation);
    }
    evidence.push(ApplicationLeftoverEvidence::FilesystemSnapshotComplete);
    Ok(Some(ApplicationLeftoverCandidate {
        candidate_id: candidate_id(&identity.identifier, &association.path),
        application_identifier: identity.identifier.clone(),
        application_name: identity.application_name.clone(),
        source: association.source,
        path: association.path.to_string_lossy().into_owned(),
        bytes: snapshot.bytes,
        file_count: snapshot.file_count,
        modified_at_ms: modified_ms(&metadata),
        confidence: ApplicationLeftoverConfidence::High,
        default_selected: false,
        evidence,
        snapshot_fingerprint: fingerprint,
    }))
}

fn read_container_identity(path: &Path) -> Result<ContainerIdentity, CandidateEvaluationError> {
    let metadata_path = path.join(CONTAINER_METADATA_FILE);
    let metadata_file = fs::File::open(&metadata_path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            CandidateEvaluationError::AccessDenied
        } else {
            CandidateEvaluationError::Rejected(format!(
                "failed to open Apple container metadata: {error}"
            ))
        }
    })?;
    let dictionary = Value::from_reader(metadata_file)
        .map_err(|error| {
            CandidateEvaluationError::Rejected(format!(
                "failed to read Apple container metadata: {error}"
            ))
        })?
        .into_dictionary()
        .ok_or_else(|| "Apple container metadata is not a dictionary".to_string())?;
    let identifier = dictionary_string(&dictionary, "MCMMetadataIdentifier")
        .ok_or_else(|| "Apple container metadata has no identifier".to_string())?;
    let validation = nested_dictionary(
        &dictionary,
        &[
            "MCMMetadataInfo",
            "SandboxProfileDataValidationInfo",
            "Parameters",
        ],
    )
    .ok_or_else(|| "Apple container ownership metadata is incomplete".to_string())?;
    let bundle_identifier = dictionary_string(validation, "application_bundle_id")
        .ok_or_else(|| "Apple container metadata has no bundle identifier".to_string())?;
    if bundle_identifier != identifier {
        return Err("Apple container bundle identifier does not match"
            .to_string()
            .into());
    }
    let former_bundle = PathBuf::from(
        dictionary_string(validation, "application_bundle")
            .ok_or_else(|| "Apple container metadata has no former bundle path".to_string())?,
    );
    if !former_bundle.is_absolute()
        || !former_bundle
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    {
        return Err("Apple container metadata has an invalid bundle path"
            .to_string()
            .into());
    }
    let application_name = former_bundle
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            identifier
                .rsplit('.')
                .next()
                .unwrap_or(&identifier)
                .to_string()
        });
    Ok(ContainerIdentity {
        identifier,
        former_bundle,
        application_name,
    })
}

fn nested_dictionary<'a>(root: &'a Dictionary, keys: &[&str]) -> Option<&'a Dictionary> {
    keys.iter()
        .try_fold(root, |dictionary, key| dictionary.get(key)?.as_dictionary())
}

fn dictionary_string(dictionary: &Dictionary, key: &str) -> Option<String> {
    dictionary
        .get(key)
        .and_then(Value::as_string)
        .map(str::to_string)
}

fn snapshot_path(path: &Path, source: ApplicationLeftoverSource) -> PathSnapshot {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-application-leftover-snapshot-v3");
    hasher.update(source.stable_code().as_bytes());
    let mut snapshot = PathSnapshot::default();
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            snapshot.skipped_count += 1;
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                snapshot.access_denied_count += 1;
            }
            return snapshot;
        }
    };
    hash_metadata_identity(&metadata, &mut hasher);
    if metadata.is_dir() {
        snapshot_directory(path, path, &mut snapshot, &mut hasher);
    } else if metadata.is_file() {
        hasher.update(&[1]);
        hasher.update(&metadata.len().to_le_bytes());
        hasher.update(&metadata.mtime().to_le_bytes());
        hasher.update(&metadata.mtime_nsec().to_le_bytes());
        snapshot.bytes = metadata.len();
        snapshot.file_count = 1;
    } else {
        snapshot.skipped_count += 1;
    }
    if snapshot.skipped_count == 0 {
        snapshot.fingerprint = Some(hasher.finalize().to_hex().to_string());
    }
    snapshot
}

fn snapshot_directory(
    root: &Path,
    directory: &Path,
    snapshot: &mut PathSnapshot,
    hasher: &mut blake3::Hasher,
) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            snapshot.skipped_count += 1;
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                snapshot.access_denied_count += 1;
            }
            return;
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(error) => {
                snapshot.skipped_count += 1;
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    snapshot.access_denied_count += 1;
                }
            }
        }
    }
    paths.sort();
    for path in paths {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                snapshot.skipped_count += 1;
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    snapshot.access_denied_count += 1;
                }
                continue;
            }
        };
        let Ok(relative) = path.strip_prefix(root) else {
            snapshot.skipped_count += 1;
            continue;
        };
        let relative_bytes = relative.as_os_str().as_encoded_bytes();
        hasher.update(&(relative_bytes.len() as u64).to_le_bytes());
        hasher.update(relative_bytes);
        hash_metadata_identity(&metadata, hasher);
        if metadata.file_type().is_symlink() {
            let target = match fs::read_link(&path) {
                Ok(target) => target,
                Err(error) => {
                    snapshot.skipped_count += 1;
                    if error.kind() == std::io::ErrorKind::PermissionDenied {
                        snapshot.access_denied_count += 1;
                    }
                    continue;
                }
            };
            // Apple sandbox redirects are normally relative links. WebKit also
            // creates absolute self-links inside a container. Hash either form
            // without following it, but reject an absolute destination outside
            // this container so an unexpected link cannot broaden the cleanup
            // scope.
            if target.is_absolute()
                && (!target.starts_with(root)
                    || target
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir)))
            {
                snapshot.skipped_count += 1;
                continue;
            }
            let target_bytes = target.as_os_str().as_encoded_bytes();
            hasher.update(&[3]);
            hasher.update(&(target_bytes.len() as u64).to_le_bytes());
            hasher.update(target_bytes);
            snapshot.file_count += 1;
        } else if metadata.is_dir() {
            hasher.update(&[2]);
            snapshot_directory(root, &path, snapshot, hasher);
        } else if metadata.is_file() {
            hasher.update(&[1]);
            hasher.update(&metadata.len().to_le_bytes());
            hasher.update(&metadata.mtime().to_le_bytes());
            hasher.update(&metadata.mtime_nsec().to_le_bytes());
            snapshot.bytes = snapshot.bytes.saturating_add(metadata.len());
            snapshot.file_count += 1;
        } else {
            snapshot.skipped_count += 1;
        }
    }
}

fn hash_metadata_identity(metadata: &fs::Metadata, hasher: &mut blake3::Hasher) {
    hasher.update(&metadata.dev().to_le_bytes());
    hasher.update(&metadata.ino().to_le_bytes());
    hasher.update(&metadata.mode().to_le_bytes());
    hasher.update(&metadata.uid().to_le_bytes());
    hasher.update(&metadata.gid().to_le_bytes());
    hasher.update(&metadata.ctime().to_le_bytes());
    hasher.update(&metadata.ctime_nsec().to_le_bytes());
}

fn complete_fingerprint(snapshot: &PathSnapshot) -> Option<String> {
    (snapshot.skipped_count == 0)
        .then(|| snapshot.fingerprint.clone())
        .flatten()
}

fn candidate_id(identifier: &str, path: &Path) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-application-leftover-v2");
    hasher.update(identifier.as_bytes());
    hasher.update(path.as_os_str().as_encoded_bytes());
    format!("leftover-{}", &hasher.finalize().to_hex()[..24])
}

pub(super) fn candidate_process_names(candidate: &ApplicationLeftoverCandidate) -> Vec<String> {
    process_identity_names_from_parts(
        &candidate.application_name,
        &candidate.application_identifier,
    )
}

fn process_identity_names(identity: &ContainerIdentity) -> Vec<String> {
    process_identity_names_from_parts(&identity.application_name, &identity.identifier)
}

fn process_identity_names_from_parts(application_name: &str, identifier: &str) -> Vec<String> {
    let identifier_leaf = identifier.rsplit('.').next().unwrap_or(identifier);
    let mut names = vec![
        application_name.to_string(),
        identifier.to_string(),
        identifier_leaf.to_string(),
    ];
    names.sort_by_key(|value| value.to_ascii_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    names
}

pub(super) fn revalidate_candidate(candidate: &ApplicationLeftoverCandidate) -> Result<(), String> {
    let path = PathBuf::from(&candidate.path);
    let library = current_platform()
        .user_directories()
        .map_err(|error| error.to_string())?
        .home_directory()
        .join("Library");
    let container_root = library.join("Containers");
    let container_path = container_root.join(&candidate.application_identifier);
    let path_is_expected = if candidate.source == ApplicationLeftoverSource::SandboxContainer {
        path == container_path
    } else {
        associated_paths(&library, &candidate.application_identifier)
            .into_iter()
            .any(|association| association.source == candidate.source && association.path == path)
    };
    if !path_is_expected {
        return Err("candidate is outside the expected application data location".to_string());
    }
    current_platform()
        .validate_path_no_links(&path)
        .map_err(|error| error.to_string())?;
    current_platform()
        .validate_cleanup_root(&path)
        .map_err(|error| error.to_string())?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("failed to revalidate candidate metadata: {error}"))?;
    if (!metadata.is_dir() && !metadata.is_file()) || current_platform().is_link_like(&metadata) {
        return Err("candidate type changed after scanning".to_string());
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err("candidate ownership changed after scanning".to_string());
    }
    let identity = read_container_identity(&container_path).map_err(|error| error.to_string())?;
    if identity.identifier != candidate.application_identifier || identity.former_bundle.exists() {
        return Err("application ownership changed after scanning".to_string());
    }
    let snapshot = snapshot_path(&path, candidate.source);
    let fingerprint = complete_fingerprint(&snapshot)
        .ok_or_else(|| "candidate snapshot became incomplete".to_string())?;
    if snapshot.bytes != candidate.bytes
        || snapshot.file_count != candidate.file_count
        || fingerprint != candidate.snapshot_fingerprint
    {
        return Err("candidate content changed after scanning".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_platform::InstalledApplication;
    use std::{
        os::unix::fs::PermissionsExt,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn fixture_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mangodisk-leftover-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    fn write_container(root: &Path, identifier: &str, former_bundle: &Path) -> PathBuf {
        let container = root.join(identifier);
        fs::create_dir_all(container.join("Data/Library"))
            .expect("the container fixture should be created");
        fs::write(container.join("Data/Library/cache.bin"), b"fixture")
            .expect("the container payload should be written");
        let mut parameters = Dictionary::new();
        parameters.insert(
            "application_bundle_id".to_string(),
            Value::String(identifier.to_string()),
        );
        parameters.insert(
            "application_bundle".to_string(),
            Value::String(former_bundle.to_string_lossy().into_owned()),
        );
        let mut validation = Dictionary::new();
        validation.insert("Parameters".to_string(), Value::Dictionary(parameters));
        let mut info = Dictionary::new();
        info.insert(
            "SandboxProfileDataValidationInfo".to_string(),
            Value::Dictionary(validation),
        );
        let mut metadata = Dictionary::new();
        metadata.insert(
            "MCMMetadataIdentifier".to_string(),
            Value::String(identifier.to_string()),
        );
        metadata.insert("MCMMetadataInfo".to_string(), Value::Dictionary(info));
        Value::Dictionary(metadata)
            .to_file_xml(container.join(CONTAINER_METADATA_FILE))
            .expect("the Apple metadata fixture should be written");
        container
    }

    #[test]
    fn verified_orphan_container_is_reported() {
        let root = fixture_path("orphan");
        fs::create_dir_all(&root).expect("the fixture root should be created");
        let container = write_container(&root, "com.example.removed", &root.join("Removed.app"));
        let inventory = ApplicationInventory::fixture(Vec::new(), true);
        let processes = ProcessSnapshot::default();

        let candidates = evaluate_container(&root, &container, &inventory, &processes)
            .expect("the container should be evaluated")
            .expect("the orphan should be reported");
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.source == ApplicationLeftoverSource::SandboxContainer)
            .expect("the sandbox container should be reported");

        assert_eq!(candidate.application_identifier, "com.example.removed");
        assert!(!candidate.default_selected);
        assert!(candidate.bytes > 0);
        fs::remove_dir_all(root).expect("the fixture should be removed");
    }

    #[test]
    fn installed_component_identity_blocks_candidate() {
        let root = fixture_path("owned");
        fs::create_dir_all(&root).expect("the fixture root should be created");
        let container = write_container(&root, "com.example.helper", &root.join("Removed.app"));
        let inventory = ApplicationInventory::fixture(
            vec![InstalledApplication {
                uninstall_diagnostic: None,
                catalog_identifier: format!(
                    "macos-bundle:{}",
                    root.join("Example.app").to_string_lossy()
                ),
                source_identities: Vec::new(),
                primary_identifier: "com.example.main".to_string(),
                identifiers: vec![
                    "com.example.main".to_string(),
                    "com.example.helper".to_string(),
                ],
                name: "Example".to_string(),
                version: None,
                publisher: None,
                estimated_bytes: 0,
                last_used_at_ms: None,
                installed_at_ms: None,
                icon_path: Some(root.join("Example.app")),
                bundle_path: Some(root.join("Example.app")),
                executable_paths: Vec::new(),
                uninstall_registration: None,
            }],
            true,
        );
        let processes = ProcessSnapshot::default();

        assert!(
            evaluate_container(&root, &container, &inventory, &processes)
                .expect("the container should be evaluated")
                .is_none()
        );
        fs::remove_dir_all(root).expect("the fixture should be removed");
    }

    #[test]
    fn identifier_mismatch_is_rejected() {
        let root = fixture_path("mismatch");
        fs::create_dir_all(&root).expect("the fixture root should be created");
        let container = write_container(&root, "com.example.actual", &root.join("Removed.app"));
        fs::rename(&container, root.join("com.example.different"))
            .expect("the fixture should be renamed");
        let inventory = ApplicationInventory::fixture(Vec::new(), true);
        let processes = ProcessSnapshot::default();

        let error = evaluate_container(
            &root,
            &root.join("com.example.different"),
            &inventory,
            &processes,
        )
        .expect_err("mismatched identity must fail closed");
        assert!(error.to_string().contains("identifier"));
        fs::remove_dir_all(root).expect("the fixture should be removed");
    }

    #[test]
    fn denied_container_metadata_is_reported_as_limited_access() {
        let root = fixture_path("permission-denied");
        fs::create_dir_all(&root).expect("the fixture root should be created");
        let container = write_container(&root, "com.example.denied", &root.join("Removed.app"));
        let metadata_path = container.join(CONTAINER_METADATA_FILE);
        let original_permissions = fs::metadata(&metadata_path)
            .expect("the metadata permissions should be readable")
            .permissions();
        fs::set_permissions(&metadata_path, fs::Permissions::from_mode(0o000))
            .expect("the metadata fixture should become unreadable");

        let result = evaluate_container(
            &root,
            &container,
            &ApplicationInventory::fixture(Vec::new(), true),
            &ProcessSnapshot::default(),
        );

        fs::set_permissions(&metadata_path, original_permissions)
            .expect("the metadata fixture permissions should be restored");
        assert!(matches!(
            result,
            Err(CandidateEvaluationError::AccessDenied)
        ));
        fs::remove_dir_all(root).expect("the fixture should be removed");
    }

    #[test]
    fn denied_exact_association_invalidates_partial_container_result() {
        let base = fixture_path("denied-companion");
        let root = base.join("Library/Containers");
        fs::create_dir_all(&root).expect("the fixture root should be created");
        let container = write_container(&root, "com.example.denied", &base.join("Removed.app"));
        let support = base
            .join("Library/Application Support")
            .join("com.example.denied");
        fs::create_dir_all(&support).expect("the companion fixture should be created");
        fs::write(support.join("settings.db"), b"fixture")
            .expect("the companion payload should be written");
        let original_permissions = fs::metadata(&support)
            .expect("the companion permissions should be readable")
            .permissions();
        fs::set_permissions(&support, fs::Permissions::from_mode(0o000))
            .expect("the companion fixture should become unreadable");

        let result = evaluate_container(
            &root,
            &container,
            &ApplicationInventory::fixture(Vec::new(), true),
            &ProcessSnapshot::default(),
        );

        fs::set_permissions(&support, original_permissions)
            .expect("the companion permissions should be restored");
        assert!(matches!(
            result,
            Err(CandidateEvaluationError::AccessDenied)
        ));
        fs::remove_dir_all(base).expect("the fixture should be removed");
    }

    #[test]
    fn relative_sandbox_redirect_is_hashed_without_following_target() {
        let root = fixture_path("relative-link");
        fs::create_dir_all(&root).expect("the fixture root should be created");
        let container = write_container(&root, "com.example.relative", &root.join("Removed.app"));
        std::os::unix::fs::symlink("../../../../Downloads", container.join("Data/Downloads"))
            .expect("the relative sandbox redirect should be created");

        let snapshot = snapshot_path(&container, ApplicationLeftoverSource::SandboxContainer);

        assert_eq!(snapshot.skipped_count, 0);
        assert!(snapshot.fingerprint.is_some());
        assert_eq!(snapshot.file_count, 3);
        fs::remove_dir_all(root).expect("the fixture should be removed");
    }

    #[test]
    fn absolute_symlink_makes_snapshot_incomplete() {
        let root = fixture_path("absolute-link");
        fs::create_dir_all(&root).expect("the fixture root should be created");
        let container = write_container(&root, "com.example.absolute", &root.join("Removed.app"));
        std::os::unix::fs::symlink(std::env::temp_dir(), container.join("Data/External"))
            .expect("the absolute link should be created");

        let snapshot = snapshot_path(&container, ApplicationLeftoverSource::SandboxContainer);

        assert_eq!(snapshot.skipped_count, 1);
        assert!(snapshot.fingerprint.is_none());
        fs::remove_dir_all(root).expect("the fixture should be removed");
    }

    #[test]
    fn absolute_link_inside_container_is_hashed_without_following_target() {
        let root = fixture_path("internal-absolute-link");
        fs::create_dir_all(&root).expect("the fixture root should be created");
        let container = write_container(&root, "com.example.internal", &root.join("Removed.app"));
        let target = container.join("Data/Library");
        std::os::unix::fs::symlink(&target, container.join("Data/Internal"))
            .expect("the internal absolute link should be created");

        let snapshot = snapshot_path(&container, ApplicationLeftoverSource::SandboxContainer);

        assert_eq!(snapshot.skipped_count, 0);
        assert!(snapshot.fingerprint.is_some());
        assert_eq!(snapshot.file_count, 3);
        fs::remove_dir_all(root).expect("the fixture should be removed");
    }

    #[test]
    fn candidate_identity_stays_stable_when_the_snapshot_changes() {
        let root = fixture_path("stable-candidate");
        let container = write_container(&root, "com.example.stable", &root.join("Removed.app"));
        let before = candidate_id("com.example.stable", &container);
        fs::write(container.join("Data/Library/cache.bin"), b"changed fixture")
            .expect("the container fixture should change");
        let after = candidate_id("com.example.stable", &container);

        assert_eq!(before, after);
        fs::remove_dir_all(root).expect("the fixture should be removed");
    }

    #[test]
    fn process_matching_uses_application_name_and_identifier_leaf() {
        let names = process_identity_names_from_parts("Picture Viewer", "com.example.miniqpicview");

        assert!(names.iter().any(|name| name == "Picture Viewer"));
        assert!(names.iter().any(|name| name == "com.example.miniqpicview"));
        assert!(names.iter().any(|name| name == "miniqpicview"));
    }

    #[test]
    fn exact_identifier_companion_is_grouped_with_verified_container() {
        let base = fixture_path("companion");
        let root = base.join("Library/Containers");
        fs::create_dir_all(&root).expect("the fixture root should be created");
        let container = write_container(&root, "com.example.removed", &base.join("Removed.app"));
        let support = base
            .join("Library/Application Support")
            .join("com.example.removed");
        fs::create_dir_all(&support).expect("the companion fixture should be created");
        fs::write(support.join("settings.db"), b"fixture")
            .expect("the companion payload should be written");

        let candidates = evaluate_container(
            &root,
            &container,
            &ApplicationInventory::fixture(Vec::new(), true),
            &ProcessSnapshot::default(),
        )
        .expect("the container should be evaluated")
        .expect("the orphan should be reported");

        assert!(candidates.iter().any(|candidate| {
            candidate.source == ApplicationLeftoverSource::ApplicationSupport
                && candidate.path == support.to_string_lossy()
                && candidate
                    .evidence
                    .contains(&ApplicationLeftoverEvidence::ExactIdentifierAssociation)
        }));
        fs::remove_dir_all(base).expect("the fixture should be removed");
    }
}
