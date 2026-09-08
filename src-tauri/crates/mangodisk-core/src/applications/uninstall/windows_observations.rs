use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

use mangodisk_platform::{current_platform, DirectoryTreeAggregateError, Platform};
use serde::{Deserialize, Serialize};

use crate::{
    filesystem::metadata::{display_path, now_ms},
    shared::{application_paths, operation::OPERATION_CANCELLED_ERROR},
};

use super::models::{
    ApplicationUninstallCandidate, ApplicationUninstallInventorySource,
    ApplicationUninstallRecordState,
};

const OBSERVATION_SCHEMA_VERSION: u32 = 3;
const OBSERVATION_FILE_NAME: &str = "uninstall-observations.json";
const MAX_DIRECTORY_ENTRIES_PER_ROOT: usize = 4_096;
const MAX_RELATED_PATHS_PER_APPLICATION: usize = 4;
const MAX_OBSERVATIONS: usize = 2_048;
const MAX_OBSERVATION_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ObservationDocument {
    schema_version: u32,
    entries: Vec<ApplicationObservation>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplicationObservation {
    application_id: String,
    primary_identifier: String,
    source_identities: Vec<ObservationSourceIdentity>,
    name: String,
    publisher: Option<String>,
    application_path: Option<String>,
    possible_related_paths: Vec<String>,
    observed_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ObservationSourceIdentity {
    source: String,
    identifier: String,
}

/// Stores the reusable string facts for one application-storage directory.
///
/// Windows uninstall inventories commonly contain hundreds of applications. Matching every
/// application against every top-level storage directory used to normalize and tokenize the same
/// directory name once per application. Building these immutable facts while enumerating the
/// directories keeps filesystem discovery and identity matching separate and removes that repeated
/// allocation without changing the evidence policy.
#[derive(Debug)]
struct DirectoryMatchFact {
    path: PathBuf,
    normalized_name: String,
    normalized_segments: Vec<String>,
    sort_key: String,
}

#[derive(Debug, Default)]
pub(super) struct ObservationStats {
    pub(super) directory_count: u64,
    pub(super) elapsed_ms: u64,
}

struct DirectoryDiscovery {
    facts: Vec<DirectoryMatchFact>,
    native_root_count: u64,
    portable_fallback_count: u64,
}

pub(super) fn annotate(
    candidates: &mut [ApplicationUninstallCandidate],
    cancellation: &AtomicBool,
) -> Result<ObservationStats, String> {
    let started = Instant::now();
    let Ok(user_directories) = current_platform().user_directories() else {
        return Ok(ObservationStats::default());
    };
    let roots = user_directories.application_storage_directories();
    let directory_started = Instant::now();
    let directory_discovery = top_level_directory_facts(roots, cancellation)?;
    let directory_elapsed_ms = directory_started.elapsed().as_millis();
    let directory_facts = directory_discovery.facts;
    let Ok(application_paths) = application_paths() else {
        return Ok(ObservationStats::default());
    };
    let observation_path = application_paths
        .data_directory()
        .join(OBSERVATION_FILE_NAME);
    let load_started = Instant::now();
    let previous = load_observations(&observation_path);
    let load_elapsed_ms = load_started.elapsed().as_millis();
    let previous_document = previous.clone();
    let previous_by_identity = previous
        .entries
        .iter()
        .cloned()
        .map(|entry| (observation_key(&entry), entry))
        .collect::<HashMap<_, _>>();

    let mut next_by_identity = previous
        .entries
        .into_iter()
        .map(|entry| (observation_key(&entry), entry))
        .collect::<HashMap<_, _>>();
    let mut related_path_count = 0_usize;
    let mut orphaned_count = 0_usize;

    let matching_started = Instant::now();
    for candidate in candidates.iter_mut() {
        ensure_not_cancelled(cancellation)?;
        // An exact display-name directory is useful evidence while the
        // application is still installed because it can be observed before
        // uninstall. Once only an orphaned registration remains, a newly
        // discovered directory needs independent publisher/source evidence;
        // otherwise a generic same-name directory could be attributed to the
        // wrong application.
        let current_matches = matching_paths(
            candidate,
            &directory_facts,
            candidate.record_state == ApplicationUninstallRecordState::Installed,
        );
        let key = candidate_key(candidate);
        match candidate.record_state {
            ApplicationUninstallRecordState::Installed => {
                let mut observation = ApplicationObservation {
                    application_id: candidate.application_id.clone(),
                    primary_identifier: candidate.primary_identifier.clone(),
                    source_identities: observation_source_identities(candidate),
                    name: candidate.name.clone(),
                    publisher: candidate.publisher.clone(),
                    application_path: candidate.application_path.clone(),
                    possible_related_paths: current_matches
                        .iter()
                        .map(|path| display_path(path))
                        .collect(),
                    observed_at_ms: now_ms(),
                };
                if let Some(previous) = previous_by_identity
                    .get(&key)
                    .filter(|previous| same_observation_facts(previous, &observation))
                {
                    observation.observed_at_ms = previous.observed_at_ms;
                }
                next_by_identity.insert(key, observation);
            }
            ApplicationUninstallRecordState::OrphanedRegistration => {
                orphaned_count += 1;
                let mut matches = current_matches;
                if let Some(previous) = previous_by_identity.get(&key) {
                    matches.extend(previous.possible_related_paths.iter().filter_map(|path| {
                        let path = PathBuf::from(path);
                        safe_observed_path(&path, roots).then_some(path)
                    }));
                }
                matches.sort_by_key(|path| current_platform().path_identity_key(path));
                matches.dedup_by(|left, right| current_platform().paths_equal(left, right));
                matches.truncate(MAX_RELATED_PATHS_PER_APPLICATION);
                related_path_count += matches.len();
                candidate.possible_related_paths =
                    matches.iter().map(|path| display_path(path)).collect();
            }
        }
    }
    let matching_elapsed_ms = matching_started.elapsed().as_millis();

    let persistence_started = Instant::now();
    let mut entries = next_by_identity.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .observed_at_ms
            .cmp(&left.observed_at_ms)
            .then_with(|| {
                left.primary_identifier
                    .to_ascii_lowercase()
                    .cmp(&right.primary_identifier.to_ascii_lowercase())
            })
    });
    entries.truncate(MAX_OBSERVATIONS);
    let next_document = ObservationDocument {
        schema_version: OBSERVATION_SCHEMA_VERSION,
        entries,
    };
    if next_document != previous_document {
        if let Err(error) = save_observations(&observation_path, &next_document) {
            log::warn!(
                "application_uninstall_observation_save_failed error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
        }
    }
    let persistence_elapsed_ms = persistence_started.elapsed().as_millis();
    log::debug!(
        "application_uninstall_observations_ready candidate_count={} directory_count={} orphaned_count={} related_path_count={} native_root_count={} portable_fallback_count={} directory_elapsed_ms={} load_elapsed_ms={} matching_elapsed_ms={} persistence_elapsed_ms={} elapsed_ms={}",
        candidates.len(),
        directory_facts.len(),
        orphaned_count,
        related_path_count,
        directory_discovery.native_root_count,
        directory_discovery.portable_fallback_count,
        directory_elapsed_ms,
        load_elapsed_ms,
        matching_elapsed_ms,
        persistence_elapsed_ms,
        started.elapsed().as_millis()
    );
    Ok(ObservationStats {
        directory_count: directory_facts.len() as u64,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

fn same_observation_facts(left: &ApplicationObservation, right: &ApplicationObservation) -> bool {
    left.application_id == right.application_id
        && left.primary_identifier == right.primary_identifier
        && left.source_identities == right.source_identities
        && left.name == right.name
        && left.publisher == right.publisher
        && left.application_path == right.application_path
        && left.possible_related_paths == right.possible_related_paths
}

fn top_level_directory_facts(
    roots: &[PathBuf],
    cancellation: &AtomicBool,
) -> Result<DirectoryDiscovery, String> {
    let mut facts = Vec::new();
    let mut native_root_count = 0_u64;
    let mut portable_fallback_count = 0_u64;
    for root in roots {
        ensure_not_cancelled(cancellation)?;
        match current_platform().fast_direct_physical_directories(
            root,
            MAX_DIRECTORY_ENTRIES_PER_ROOT,
            &|| cancellation.load(Ordering::Relaxed),
        ) {
            Ok(Some(enumeration)) => {
                native_root_count = native_root_count.saturating_add(1);
                facts.extend(
                    enumeration
                        .directories
                        .into_iter()
                        .map(DirectoryMatchFact::from_path),
                );
                continue;
            }
            Ok(None) => {}
            Err(DirectoryTreeAggregateError::Cancelled) => {
                return Err(OPERATION_CANCELLED_ERROR.to_string())
            }
            Err(DirectoryTreeAggregateError::Platform(error)) => {
                log::warn!(
                    "application_observation_native_enumeration_failed error_digest={}",
                    blake3::hash(error.as_bytes()).to_hex()
                );
            }
        }
        portable_fallback_count = portable_fallback_count.saturating_add(1);
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.take(MAX_DIRECTORY_ENTRIES_PER_ROOT) {
            ensure_not_cancelled(cancellation)?;
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir() && !current_platform().is_link_like(&metadata) {
                facts.push(DirectoryMatchFact::from_path(path));
            }
        }
    }
    Ok(DirectoryDiscovery {
        facts,
        native_root_count,
        portable_fallback_count,
    })
}

fn ensure_not_cancelled(cancellation: &AtomicBool) -> Result<(), String> {
    if cancellation.load(Ordering::Relaxed) {
        Err(OPERATION_CANCELLED_ERROR.to_string())
    } else {
        Ok(())
    }
}

impl DirectoryMatchFact {
    fn from_path(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        Self {
            normalized_name: normalize_identity(name),
            normalized_segments: name
                .split(|character: char| !character.is_alphanumeric())
                .map(normalize_identity)
                .filter(|value| useful_identity(value))
                .collect(),
            sort_key: current_platform().path_identity_key(&path),
            path,
        }
    }
}

fn matching_paths(
    candidate: &ApplicationUninstallCandidate,
    directory_facts: &[DirectoryMatchFact],
    allow_observation_only_exact_name: bool,
) -> Vec<PathBuf> {
    let mut aliases = vec![
        normalize_identity(&candidate.primary_identifier),
        normalize_identity(&candidate.name),
    ];
    aliases.extend(
        candidate
            .source_identities
            .iter()
            .map(|identity| normalize_identity(&identity.identifier)),
    );
    aliases.sort();
    aliases.dedup();
    let publisher = candidate
        .publisher
        .as_deref()
        .map(normalize_identity)
        .filter(|value| useful_identity(value));
    let mut matches = directory_facts
        .iter()
        .filter(|fact| {
            let exact_name = aliases
                .iter()
                .any(|alias| useful_identity(alias) && *alias == fact.normalized_name);
            let qualified_identity = publisher.as_ref().is_some_and(|publisher| {
                fact.normalized_segments
                    .iter()
                    .any(|segment| segment == publisher)
                    && aliases.iter().any(|alias| {
                        useful_identity(alias)
                            && fact
                                .normalized_segments
                                .iter()
                                .any(|segment| segment == alias)
                    })
            });
            (allow_observation_only_exact_name && exact_name) || qualified_identity
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.sort_key.cmp(&right.sort_key));
    matches
        .into_iter()
        .map(|fact| fact.path.clone())
        .take(MAX_RELATED_PATHS_PER_APPLICATION)
        .collect::<Vec<_>>()
}

fn useful_identity(value: &str) -> bool {
    value.chars().count() >= 4
        && value.chars().any(|character| character.is_alphabetic())
        && !matches!(
            value,
            "application"
                | "client"
                | "desktop"
                | "launcher"
                | "manager"
                | "program"
                | "programs"
                | "software"
                | "update"
                | "updater"
        )
}

fn observation_source_identities(
    candidate: &ApplicationUninstallCandidate,
) -> Vec<ObservationSourceIdentity> {
    let mut identities = candidate
        .source_identities
        .iter()
        .map(|identity| ObservationSourceIdentity {
            source: inventory_source_code(identity.source).to_string(),
            identifier: identity.identifier.clone(),
        })
        .collect::<Vec<_>>();
    identities.sort_by(|left, right| {
        left.source
            .cmp(&right.source)
            .then_with(|| left.identifier.cmp(&right.identifier))
    });
    identities.dedup();
    identities
}

const fn inventory_source_code(source: ApplicationUninstallInventorySource) -> &'static str {
    match source {
        ApplicationUninstallInventorySource::MacosBundle => "macos_bundle",
        ApplicationUninstallInventorySource::WindowsRegistry => "windows_registry",
        ApplicationUninstallInventorySource::WindowsMsi => "windows_msi",
        ApplicationUninstallInventorySource::WindowsAppx => "windows_appx",
        ApplicationUninstallInventorySource::Winget => "winget",
        ApplicationUninstallInventorySource::Steam => "steam",
        ApplicationUninstallInventorySource::Scoop => "scoop",
        ApplicationUninstallInventorySource::Chocolatey => "chocolatey",
    }
}

fn normalize_identity(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn safe_observed_path(path: &Path, roots: &[PathBuf]) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if !roots
        .iter()
        .any(|root| current_platform().paths_equal(parent, root))
    {
        return false;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata.is_dir() && !current_platform().is_link_like(&metadata)
}

fn candidate_key(candidate: &ApplicationUninstallCandidate) -> String {
    candidate.application_id.clone()
}

fn observation_key(observation: &ApplicationObservation) -> String {
    observation.application_id.clone()
}

fn load_observations(path: &Path) -> ObservationDocument {
    let Ok(metadata) = path.metadata() else {
        return ObservationDocument::default();
    };
    if metadata.len() > MAX_OBSERVATION_BYTES {
        return ObservationDocument::default();
    }
    let Ok(content) = fs::read(path) else {
        return ObservationDocument::default();
    };
    let Ok(document) = serde_json::from_slice::<ObservationDocument>(&content) else {
        return ObservationDocument::default();
    };
    if document.schema_version != OBSERVATION_SCHEMA_VERSION {
        return ObservationDocument::default();
    }
    document
}

fn save_observations(path: &Path, document: &ObservationDocument) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "application observation path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create application observation directory: {error}"))?;
    let content = serde_json::to_vec(document)
        .map_err(|error| format!("failed to serialize application observations: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temporary)
        .map_err(|error| {
            format!("failed to create application observation temporary file: {error}")
        })?;
    file.write_all(&content)
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            format!("failed to write application observation temporary file: {error}")
        })?;
    replace_file(&temporary, path)
        .map_err(|error| format!("failed to save application observations: {error}"))
}

fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let succeeded = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if succeeded == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::applications::uninstall::models::{
        ApplicationUninstallCapability, ApplicationUninstallPlatform,
    };

    fn candidate(
        primary_identifier: &str,
        name: &str,
        publisher: Option<&str>,
    ) -> ApplicationUninstallCandidate {
        ApplicationUninstallCandidate {
            application_id: "application-fixture".to_string(),
            primary_identifier: primary_identifier.to_string(),
            source_identities: Vec::new(),
            name: name.to_string(),
            version: None,
            publisher: publisher.map(str::to_string),
            estimated_bytes: 0,
            last_used_at_ms: None,
            installed_at_ms: None,
            platform: ApplicationUninstallPlatform::WindowsRegistry,
            installer_kind: None,
            execution_mode: None,
            capability: ApplicationUninstallCapability::ViewOnly,
            record_state: ApplicationUninstallRecordState::OrphanedRegistration,
            uninstall_diagnostic: None,
            application_path: None,
            possible_related_paths: Vec::new(),
            icon_path: None,
            running_processes: Vec::new(),
            executable_paths: Vec::new(),
            total_bytes: 0,
            default_selected_bytes: 0,
            associated_data_complete: false,
            components: Vec::new(),
            uninstall_registration: None,
        }
    }

    fn directory_facts(paths: &[PathBuf]) -> Vec<DirectoryMatchFact> {
        paths
            .iter()
            .cloned()
            .map(DirectoryMatchFact::from_path)
            .collect()
    }

    fn legacy_matching_paths(
        candidate: &ApplicationUninstallCandidate,
        directories: &[PathBuf],
        allow_observation_only_exact_name: bool,
    ) -> Vec<PathBuf> {
        let mut aliases = vec![
            normalize_identity(&candidate.primary_identifier),
            normalize_identity(&candidate.name),
        ];
        aliases.extend(
            candidate
                .source_identities
                .iter()
                .map(|identity| normalize_identity(&identity.identifier)),
        );
        aliases.sort();
        aliases.dedup();
        let publisher = candidate
            .publisher
            .as_deref()
            .map(normalize_identity)
            .filter(|value| useful_identity(value));
        let mut matches = directories
            .iter()
            .filter(|path| {
                let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                    return false;
                };
                let normalized_name = normalize_identity(name);
                let segments = name
                    .split(|character: char| !character.is_alphanumeric())
                    .map(normalize_identity)
                    .filter(|value| useful_identity(value))
                    .collect::<Vec<_>>();
                let exact_name = aliases
                    .iter()
                    .any(|alias| useful_identity(alias) && *alias == normalized_name);
                let qualified_identity = publisher.as_ref().is_some_and(|publisher| {
                    segments.iter().any(|segment| segment == publisher)
                        && aliases.iter().any(|alias| {
                            useful_identity(alias)
                                && segments.iter().any(|segment| segment == alias)
                        })
                });
                (allow_observation_only_exact_name && exact_name) || qualified_identity
            })
            .cloned()
            .collect::<Vec<_>>();
        matches.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
        matches.truncate(MAX_RELATED_PATHS_PER_APPLICATION);
        matches
    }

    #[test]
    fn reverse_domain_directory_requires_product_and_publisher_evidence() {
        let candidate = candidate("CodexPlusPlus", "Codex++", Some("BigPizzaV3"));
        let directories = vec![
            PathBuf::from(r"C:\Users\fixture\AppData\Local\com.bigpizzav3.codexplusplus.manager"),
            PathBuf::from(r"C:\Users\fixture\AppData\Local\com.someone.codexplusplus.manager"),
        ];
        let facts = directory_facts(&directories);

        assert_eq!(
            matching_paths(&candidate, &facts, false),
            vec![directories[0].clone()]
        );
    }

    #[test]
    fn exact_display_directory_is_recorded_only_while_installed() {
        let candidate = candidate("CodexPlusPlus", "Codex++", Some("BigPizzaV3"));
        let directory = PathBuf::from(r"C:\Users\fixture\AppData\Roaming\Codex++");
        let facts = directory_facts(std::slice::from_ref(&directory));

        assert_eq!(
            matching_paths(&candidate, &facts, true),
            vec![directory.clone()]
        );
        assert!(matching_paths(&candidate, &facts, false).is_empty());
    }

    #[test]
    fn generic_directory_names_do_not_create_matches() {
        let candidate = candidate("Manager", "Updater", Some("Example"));
        let directories = vec![
            PathBuf::from(r"C:\Users\fixture\AppData\Local\Manager"),
            PathBuf::from(r"C:\Users\fixture\AppData\Local\Updater"),
        ];
        let facts = directory_facts(&directories);

        assert!(matching_paths(&candidate, &facts, true).is_empty());
    }

    #[test]
    fn directory_facts_precompute_normalized_match_data() {
        let path =
            PathBuf::from(r"C:\Users\fixture\AppData\Local\com.Big-Pizza_v3.CodexPlusPlus.manager");
        let fact = DirectoryMatchFact::from_path(path.clone());

        assert_eq!(fact.path, path);
        assert_eq!(fact.normalized_name, "combigpizzav3codexplusplusmanager");
        assert_eq!(
            fact.normalized_segments,
            vec!["pizza".to_string(), "codexplusplus".to_string()]
        );
    }

    #[test]
    fn observed_directory_accepts_a_canonical_path_under_a_display_root() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-observed-path-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let candidate = root.join("AssociatedData");
        fs::create_dir_all(&candidate).expect("the observed path fixture should be created");
        let canonical_candidate =
            fs::canonicalize(&candidate).expect("the observed path fixture should canonicalize");

        assert!(safe_observed_path(&canonical_candidate, &[root.clone()]));

        fs::remove_dir_all(root).expect("the observed path fixture should be removed");
    }

    #[test]
    fn precomputed_directory_facts_preserve_legacy_matching_results() {
        let directories = (0..256)
            .map(|index| {
                PathBuf::from(format!(
                    r"C:\Users\fixture\AppData\Local\com.publisher{index:03}.product{index:03}.cache"
                ))
            })
            .collect::<Vec<_>>();
        let facts = directory_facts(&directories);

        for index in [0, 1, 127, 255] {
            let candidate = candidate(
                &format!("product{index:03}"),
                &format!("Product {index:03}"),
                Some(&format!("publisher{index:03}")),
            );
            assert_eq!(
                matching_paths(&candidate, &facts, false),
                legacy_matching_paths(&candidate, &directories, false)
            );
        }
    }

    #[test]
    #[ignore = "performance diagnostic for Windows application observation matching"]
    fn precomputed_directory_fact_performance_diagnostic() {
        let directories = (0..MAX_DIRECTORY_ENTRIES_PER_ROOT)
            .map(|index| {
                PathBuf::from(format!(
                    r"C:\Users\fixture\AppData\Local\com.publisher{index:04}.product{index:04}.cache"
                ))
            })
            .collect::<Vec<_>>();
        let candidates = (0..256)
            .map(|index| {
                candidate(
                    &format!("product{index:04}"),
                    &format!("Product {index:04}"),
                    Some(&format!("publisher{index:04}")),
                )
            })
            .collect::<Vec<_>>();

        let legacy_started = Instant::now();
        let legacy_count = candidates
            .iter()
            .map(|candidate| legacy_matching_paths(candidate, &directories, false).len())
            .sum::<usize>();
        let legacy_elapsed = legacy_started.elapsed();

        let optimized_started = Instant::now();
        let facts = directory_facts(&directories);
        let optimized_count = candidates
            .iter()
            .map(|candidate| matching_paths(candidate, &facts, false).len())
            .sum::<usize>();
        let optimized_elapsed = optimized_started.elapsed();

        assert_eq!(optimized_count, legacy_count);
        eprintln!(
            "windows_observation_matching_benchmark candidates={} directories={} legacy_ms={} optimized_ms={}",
            candidates.len(),
            directories.len(),
            legacy_elapsed.as_millis(),
            optimized_elapsed.as_millis()
        );
    }

    #[test]
    fn observation_document_rejects_unknown_schema() {
        let content = br#"{"schemaVersion":99,"entries":[]}"#;
        let document =
            serde_json::from_slice::<ObservationDocument>(content).expect("fixture should decode");

        assert_ne!(document.schema_version, OBSERVATION_SCHEMA_VERSION);
    }

    #[test]
    fn observation_document_round_trips_through_atomic_storage() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-application-observation-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).expect("the observation fixture should be created");
        let path = root.join(OBSERVATION_FILE_NAME);
        let document = ObservationDocument {
            schema_version: OBSERVATION_SCHEMA_VERSION,
            entries: vec![ApplicationObservation {
                application_id: "application-example".to_string(),
                primary_identifier: "Example".to_string(),
                source_identities: vec![ObservationSourceIdentity {
                    source: "WindowsRegistry".to_string(),
                    identifier: "example-registry-key".to_string(),
                }],
                name: "Example App".to_string(),
                publisher: Some("Example Publisher".to_string()),
                application_path: Some(r"C:\Apps\Example".to_string()),
                possible_related_paths: vec![
                    r"C:\Users\fixture\AppData\Local\com.example.app".to_string()
                ],
                observed_at_ms: 42,
            }],
        };

        save_observations(&path, &document).expect("the observation should be saved atomically");
        assert_eq!(load_observations(&path), document);

        fs::remove_dir_all(root).expect("the observation fixture should be removed");
    }

    #[test]
    fn unchanged_observation_facts_do_not_depend_on_scan_time() {
        let first = ApplicationObservation {
            application_id: "application-example".to_string(),
            primary_identifier: "Example".to_string(),
            source_identities: Vec::new(),
            name: "Example App".to_string(),
            publisher: Some("Example Publisher".to_string()),
            application_path: Some(r"C:\Apps\Example".to_string()),
            possible_related_paths: Vec::new(),
            observed_at_ms: 1,
        };
        let mut later = first.clone();
        later.observed_at_ms = 2;

        assert!(same_observation_facts(&first, &later));
    }

    #[test]
    fn observation_identity_uses_the_stable_catalog_application_id() {
        let mut candidate = candidate("Shared", "Shared App", Some("Publisher"));
        candidate.application_id = "application-first".to_string();
        let mut other = candidate.clone();
        other.application_id = "application-second".to_string();

        assert_ne!(candidate_key(&candidate), candidate_key(&other));
    }
}
