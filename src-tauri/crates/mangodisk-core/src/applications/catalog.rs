use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
    time::Instant,
};

use mangodisk_platform::InstalledApplication;
use mangodisk_platform::{
    current_platform, ControlledExecutable, Platform, PlatformCancellation, RunningProcessIdentity,
    SystemInventory,
};

use crate::filesystem::metadata::display_path;

static SYSTEM_INVENTORY: OnceLock<Mutex<Option<CachedSystemInventory>>> = OnceLock::new();

#[derive(Debug, Clone)]
struct CachedSystemInventory {
    revision: String,
    inventory: SystemInventory,
}

#[derive(Debug, Clone)]
pub(crate) struct ApplicationInventory {
    applications: Vec<InstalledApplication>,
    application_versions: HashMap<String, Vec<String>>,
    application_identifiers: HashSet<String>,
    applications_complete: bool,
    executable_names: HashSet<String>,
    executables: HashMap<String, ControlledExecutable>,
    developer_tools_complete: bool,
    filesystem_kinds: HashSet<String>,
    filesystem_complete: bool,
    capabilities: HashSet<String>,
    capabilities_complete: bool,
    os_version: String,
    pub(crate) application_count: usize,
    pub(crate) inventory_complete: bool,
}

#[derive(Debug)]
pub(crate) struct ScanContext {
    pub(crate) inventory: ApplicationInventory,
}

#[derive(Debug, Default)]
pub(crate) struct ProcessSnapshot {
    running_processes: HashSet<String>,
    running_executable_paths: HashSet<String>,
    unresolved_process_names: HashSet<String>,
    pub(crate) process_count: usize,
}

impl ProcessSnapshot {
    pub(crate) fn capture() -> Result<Self, String> {
        Self::capture_with_cancellation(&PlatformCancellation::new(|| false))
    }

    pub(crate) fn capture_with_cancellation(
        cancellation: &PlatformCancellation,
    ) -> Result<Self, String> {
        current_platform()
            .running_process_identities_with_cancellation(cancellation)
            .map_err(|error| error.to_string())
            .map(Self::from_process_identities)
    }

    pub(crate) fn matching_processes(&self, names: &[String]) -> Vec<String> {
        names
            .iter()
            .filter(|name| {
                process_aliases(name)
                    .iter()
                    .any(|alias| self.running_processes.contains(alias))
            })
            .cloned()
            .collect()
    }

    pub(crate) fn contains_any(&self, names: &[String]) -> bool {
        names.iter().any(|name| {
            process_aliases(name)
                .iter()
                .any(|alias| self.running_processes.contains(alias))
        })
    }

    pub(crate) fn matching_application_processes(
        &self,
        identity_names: &[String],
        executable_paths: &[std::path::PathBuf],
    ) -> Vec<String> {
        // Exact inventory paths take precedence over human-readable names.
        // Mixing both identity classes with OR semantics would allow a common
        // helper filename from another installation to mark this application
        // as running and later authorize closing the unrelated process.
        let mut matches = if executable_paths.is_empty() {
            self.matching_processes(identity_names)
        } else {
            executable_paths
                .iter()
                .filter_map(|path| {
                    let normalized = normalize(&display_path(path).replace('\\', "/"));
                    let display_name =
                        portable_path_file_name(path).unwrap_or_else(|| normalized.clone());
                    let exact_path_match = self.running_executable_paths.contains(&normalized);
                    let unresolved_name_match = process_aliases(&display_name)
                        .iter()
                        .any(|alias| self.unresolved_process_names.contains(alias));
                    (exact_path_match || unresolved_name_match).then_some(display_name)
                })
                .collect()
        };
        matches.sort_by_key(|value| value.to_ascii_lowercase());
        matches.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        matches
    }

    fn contains_executable_path(&self, path: &std::path::Path) -> bool {
        let normalized = normalize(&display_path(path).replace('\\', "/"));
        self.running_executable_paths.contains(&normalized)
    }

    #[cfg(test)]
    pub(crate) fn from_process_names(processes: Vec<String>) -> Self {
        Self::from_process_identities(
            processes
                .into_iter()
                .map(|value| {
                    let path = std::path::PathBuf::from(&value);
                    // Tests exercise both operating-system path shapes on one
                    // host. Recognize their syntax explicitly so a macOS path
                    // does not become a name-only Windows process, or vice
                    // versa, merely because the test runner uses another OS.
                    let bytes = value.as_bytes();
                    let has_windows_drive = bytes.len() >= 3
                        && bytes[0].is_ascii_alphabetic()
                        && bytes[1] == b':'
                        && matches!(bytes[2], b'/' | b'\\');
                    let has_absolute_syntax =
                        path.is_absolute() || value.starts_with('/') || has_windows_drive;
                    let executable_path = has_absolute_syntax.then(|| path.clone());
                    let executable_name = executable_path
                        .as_deref()
                        .and_then(portable_path_file_name)
                        .unwrap_or(value);
                    RunningProcessIdentity {
                        executable_name,
                        executable_path,
                    }
                })
                .collect(),
        )
    }

    fn from_process_identities(processes: Vec<RunningProcessIdentity>) -> Self {
        let process_count = processes.len();
        let running_executable_paths = processes
            .iter()
            .filter_map(|process| process.executable_path.as_ref())
            .map(|path| normalize(&display_path(path).replace('\\', "/")))
            .collect();
        let unresolved_process_names = processes
            .iter()
            .filter(|process| process.executable_path.is_none())
            .flat_map(|process| process_aliases(&process.executable_name))
            .collect();
        let running_processes = processes
            .iter()
            .flat_map(process_identity_aliases)
            .collect();
        Self {
            running_processes,
            running_executable_paths,
            unresolved_process_names,
            process_count,
        }
    }
}

fn process_identity_aliases(process: &RunningProcessIdentity) -> Vec<String> {
    let mut aliases = process_aliases(&process.executable_name);
    if let Some(path) = &process.executable_path {
        // macOS helpers frequently use generic executable names such as
        // `browser_crashpad_handler`. Attribute every enclosing application
        // bundle as an owner alias so a rule can block the correct product
        // without also blocking identical helpers from unrelated applications.
        for component in path.components() {
            let value = component.as_os_str().to_string_lossy();
            if value.to_ascii_lowercase().ends_with(".app") {
                aliases.extend(process_aliases(&value));
            }
        }
    }
    aliases
}

impl ScanContext {
    pub(crate) fn capture() -> Self {
        Self::capture_with_revision().0
    }

    /// Custom-only cleanup rules do not depend on installed applications or
    /// platform capabilities. An empty context keeps that narrow scan from
    /// paying for an unrelated system inventory while preserving one rule
    /// applicability interface for the shared scan and execution pipelines.
    pub(crate) fn empty() -> Self {
        Self {
            inventory: ApplicationInventory::from_system(SystemInventory::default(), false),
        }
    }

    /// Captures one inventory and the revision used to validate or populate
    /// its cache. Callers that need a stable before/after comparison reuse
    /// this revision instead of issuing the same expensive operating-system
    /// query twice before any useful work begins.
    pub(crate) fn capture_with_revision() -> (Self, Option<String>) {
        Self::capture_with_revision_and_cancellation(&PlatformCancellation::new(|| false))
    }

    pub(crate) fn capture_with_revision_and_cancellation(
        cancellation: &PlatformCancellation,
    ) -> (Self, Option<String>) {
        let started = Instant::now();
        let (system, inventory_complete, revision) = cached_system_inventory(cancellation);
        let inventory = ApplicationInventory::from_system(system, inventory_complete);
        log::debug!(
            "application_inventory_ready application_count={} tool_count={} filesystem_count={} capability_count={} system_complete={} elapsed_ms={}",
            inventory.application_count,
            inventory.executable_names.len(),
            inventory.filesystem_kinds.len(),
            inventory.capabilities.len(),
            inventory.inventory_complete,
            started.elapsed().as_millis()
        );
        (Self { inventory }, revision)
    }
}

fn cached_system_inventory(
    cancellation: &PlatformCancellation,
) -> (SystemInventory, bool, Option<String>) {
    // The progress event is emitted before inventory capture so adapters can
    // cancel immediately. Honor that request before even the cheap revision
    // probe; on large application roots the probe itself is observable work.
    if cancellation.is_cancelled() {
        return (SystemInventory::default(), false, None);
    }
    let cache = SYSTEM_INVENTORY.get_or_init(|| Mutex::new(None));
    let revision = current_platform().system_inventory_revision_with_cancellation(cancellation);
    if let (Ok(revision), Ok(guard)) = (&revision, cache.lock()) {
        if let Some(cached) = guard.as_ref().filter(|cached| cached.revision == *revision) {
            return (cached.inventory.clone(), true, Some(revision.clone()));
        }
    }

    match current_platform().system_inventory_with_cancellation(cancellation) {
        Ok(inventory) => {
            let revision = revision.ok();
            if let Some(revision) = &revision {
                if let Ok(mut guard) = cache.lock() {
                    *guard = Some(CachedSystemInventory {
                        revision: revision.clone(),
                        inventory: inventory.clone(),
                    });
                }
            }
            (inventory, true, revision)
        }
        Err(error) => {
            if cancellation.is_cancelled() {
                return (SystemInventory::default(), false, None);
            }
            log::warn!(
                "application_inventory_capture_failed error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
            // A stale inventory is more useful than an empty one after a
            // revision-probe failure, but only positive matches remain safe.
            if let Ok(guard) = cache.lock() {
                if let Some(cached) = guard.as_ref() {
                    return (cached.inventory.clone(), false, None);
                }
            }
            (SystemInventory::default(), false, None)
        }
    }
}

impl ApplicationInventory {
    fn from_system(system: SystemInventory, inventory_complete: bool) -> Self {
        let application_count = system.installed_applications.len();
        let applications = system.installed_applications;
        let mut application_identifiers = HashSet::new();
        let mut application_versions = HashMap::<String, Vec<String>>::new();
        let mut executable_names = HashSet::new();
        let mut executables = HashMap::new();
        for application in &applications {
            let version = application.version.as_deref();
            for identifier in application
                .identifiers
                .iter()
                .chain(std::iter::once(&application.name))
            {
                let identifier = normalize(identifier);
                application_identifiers.insert(identifier.clone());
                if let Some(version) = version {
                    let versions = application_versions.entry(identifier).or_default();
                    if !versions.iter().any(|existing| existing == version) {
                        versions.push(version.to_string());
                    }
                }
            }
        }
        for tool in system.developer_tools {
            let name = normalize(&tool.name);
            executable_names.insert(name.clone());
            executables.insert(name, tool.executable);
        }
        Self {
            applications,
            application_versions,
            application_identifiers,
            applications_complete: system.installed_applications_complete,
            executable_names,
            executables,
            developer_tools_complete: system.developer_tools_complete,
            filesystem_kinds: system
                .filesystem_kinds
                .into_iter()
                .map(|value| normalize(&value))
                .collect(),
            filesystem_complete: system.filesystem_complete,
            capabilities: system
                .capabilities
                .into_iter()
                .map(|value| normalize(&value))
                .collect(),
            capabilities_complete: system.capabilities_complete,
            os_version: system.os_version,
            application_count,
            inventory_complete,
        }
    }

    /// Specialized operations must execute the absolute path captured by the
    /// inventory. Aliases are compile-time constants and never rule-provided.
    pub(crate) fn executable(&self, aliases: &[&str]) -> Option<ControlledExecutable> {
        aliases
            .iter()
            .find_map(|alias| self.executables.get(&normalize(alias)).cloned())
    }

    pub(crate) fn executable_inventory_complete(&self) -> bool {
        self.inventory_complete && self.developer_tools_complete
    }

    pub(crate) fn application_inventory_complete(&self) -> bool {
        self.inventory_complete && self.applications_complete
    }

    pub(crate) fn installed_applications(&self) -> &[InstalledApplication] {
        &self.applications
    }

    /// Resolves an icon source only from exact application or executable
    /// identities captured by the platform inventory. Substring matching is
    /// deliberately excluded because common helper names could otherwise show
    /// another installed application's artwork in a destructive workflow.
    pub(crate) fn application_icon_path_for_process(
        &self,
        process_name: &str,
    ) -> Option<&std::path::PathBuf> {
        let aliases = process_aliases(process_name);
        unique_best_icon(self.applications.iter().filter_map(|application| {
            let icon_path = application.icon_path.as_ref()?;
            let score = application_process_match_score(application, &aliases);
            (score > 0).then_some((score, icon_path))
        }))
    }

    /// Prefers the installed application whose exact executable path is
    /// present in the process snapshot. This disambiguates applications that
    /// intentionally share a display name or executable filename without
    /// weakening process-control authorization to a name-only match.
    pub(crate) fn application_icon_path_for_running_process(
        &self,
        process_name: &str,
        processes: &ProcessSnapshot,
    ) -> Option<&std::path::PathBuf> {
        let aliases = process_aliases(process_name);
        unique_best_icon(self.applications.iter().filter_map(|application| {
            let icon_path = application.icon_path.as_ref()?;
            application
                .executable_paths
                .iter()
                .any(|path| {
                    processes.contains_executable_path(path)
                        && path.file_name().is_some_and(|name| {
                            aliases_overlap(&aliases, &process_aliases(&name.to_string_lossy()))
                        })
                })
                .then_some((1, icon_path))
        }))
    }

    /// Uses stable application identifiers declared by the owning cleanup
    /// rule when a helper executable cannot be associated by path or name.
    /// Equal-scoring applications with different artwork deliberately remain
    /// unresolved instead of displaying a misleading icon.
    pub(crate) fn application_icon_path_for_identifiers(
        &self,
        identifiers: &[String],
    ) -> Option<&std::path::PathBuf> {
        let identifiers = identifiers
            .iter()
            .map(|value| normalize(value))
            .collect::<HashSet<_>>();
        unique_best_icon(self.applications.iter().filter_map(|application| {
            let icon_path = application.icon_path.as_ref()?;
            let score = application_identifier_match_score(application, &identifiers);
            (score > 0).then_some((score, icon_path))
        }))
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn has_application_identifier(&self, identifier: &str) -> bool {
        self.application_identifiers
            .contains(&normalize(identifier))
    }

    pub(crate) fn applications_complete(&self) -> bool {
        self.inventory_complete && self.applications_complete
    }

    pub(crate) fn has_application(&self, identifiers: &[String]) -> bool {
        identifiers
            .iter()
            .map(|value| normalize(value))
            .any(|value| self.application_identifiers.contains(&value))
    }

    pub(crate) fn application_versions(&self, identifier: &str) -> Option<&[String]> {
        self.application_versions
            .get(&normalize(identifier))
            .map(Vec::as_slice)
    }

    pub(crate) fn developer_tools_complete(&self) -> bool {
        self.developer_tools_complete
    }

    pub(crate) fn has_executable(&self, names: &[String]) -> bool {
        names
            .iter()
            .map(|value| normalize(value))
            .any(|value| self.executable_names.contains(&value))
    }

    pub(crate) fn os_version(&self) -> &str {
        &self.os_version
    }

    pub(crate) fn filesystem_complete(&self) -> bool {
        self.filesystem_complete
    }

    pub(crate) fn has_filesystem_kind(&self, values: &[String]) -> bool {
        values
            .iter()
            .map(|value| normalize(value))
            .any(|value| self.filesystem_kinds.contains(&value))
    }

    pub(crate) fn capabilities_complete(&self) -> bool {
        self.capabilities_complete
    }

    pub(crate) fn has_capability(&self, values: &[String]) -> bool {
        values
            .iter()
            .map(|value| normalize(value))
            .any(|value| self.capabilities.contains(&value))
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn fixture(
        applications: Vec<InstalledApplication>,
        applications_complete: bool,
    ) -> Self {
        Self::from_system(
            SystemInventory {
                installed_applications: applications,
                installed_applications_complete: applications_complete,
                developer_tools_complete: true,
                filesystem_complete: true,
                capabilities_complete: true,
                ..SystemInventory::default()
            },
            true,
        )
    }
}

fn process_aliases(value: &str) -> Vec<String> {
    let normalized = value.replace('\\', "/");
    let name = normalized.rsplit('/').next().unwrap_or(&normalized);
    let mut aliases = vec![normalize(name)];
    if let Some(without_app) = aliases[0].strip_suffix(".app") {
        aliases.push(without_app.to_string());
    }
    if let Some(without_exe) = aliases[0].strip_suffix(".exe") {
        aliases.push(without_exe.to_string());
    }
    aliases
}

fn portable_path_file_name(path: &std::path::Path) -> Option<String> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized
        .rsplit('/')
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

fn application_process_match_score(application: &InstalledApplication, aliases: &[String]) -> u8 {
    if application.executable_paths.iter().any(|path| {
        path.file_name()
            .is_some_and(|name| aliases_overlap(aliases, &process_aliases(&name.to_string_lossy())))
    }) {
        return 3;
    }
    if aliases_overlap(aliases, &process_aliases(&application.name)) {
        return 2;
    }
    if application.bundle_path.as_ref().is_some_and(|path| {
        path.file_name()
            .is_some_and(|name| aliases_overlap(aliases, &process_aliases(&name.to_string_lossy())))
    }) {
        return 2;
    }
    0
}

fn application_identifier_match_score(
    application: &InstalledApplication,
    identifiers: &HashSet<String>,
) -> u8 {
    if identifiers.contains(&normalize(&application.primary_identifier)) {
        return 3;
    }
    if application
        .identifiers
        .iter()
        .any(|identifier| identifiers.contains(&normalize(identifier)))
    {
        return 2;
    }
    if identifiers.contains(&normalize(&application.name)) {
        1
    } else {
        0
    }
}

fn unique_best_icon<'a>(
    matches: impl Iterator<Item = (u8, &'a std::path::PathBuf)>,
) -> Option<&'a std::path::PathBuf> {
    let mut best_score = 0;
    let mut best_icon = None;
    let mut ambiguous = false;
    for (score, icon_path) in matches {
        if score > best_score {
            best_score = score;
            best_icon = Some(icon_path);
            ambiguous = false;
        } else if score == best_score && best_icon.is_some_and(|best| best != icon_path) {
            // Shared helpers and broad rule aliases can match more than one
            // installed product. Keep the fallback deterministic and honest.
            ambiguous = true;
        }
    }
    (!ambiguous).then_some(best_icon).flatten()
}

fn aliases_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|alias| right.contains(alias))
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

#[cfg(test)]
mod icon_tests {
    use std::path::PathBuf;

    use mangodisk_platform::{InstalledApplication, RunningProcessIdentity, SystemInventory};

    use super::{ApplicationInventory, ProcessSnapshot};

    fn application(name: &str, executable: &str, icon: &str) -> InstalledApplication {
        InstalledApplication {
            uninstall_diagnostic: None,
            catalog_identifier: format!("fixture:{name}"),
            primary_identifier: format!("fixture.{name}"),
            identifiers: vec![name.to_string()],
            source_identities: Vec::new(),
            name: name.to_string(),
            version: None,
            publisher: None,
            estimated_bytes: 0,
            last_used_at_ms: None,
            installed_at_ms: None,
            icon_path: Some(PathBuf::from(icon)),
            bundle_path: Some(PathBuf::from(format!("/Applications/{name}.app"))),
            executable_paths: vec![PathBuf::from(executable)],
            uninstall_registration: None,
        }
    }

    fn inventory(applications: Vec<InstalledApplication>) -> ApplicationInventory {
        ApplicationInventory::from_system(
            SystemInventory {
                installed_applications: applications,
                installed_applications_complete: true,
                ..SystemInventory::default()
            },
            true,
        )
    }

    #[test]
    fn icon_lookup_matches_primary_and_helper_executable_names() {
        let inventory = inventory(vec![application(
            "Tencent Lemon",
            "/Applications/Tencent Lemon.app/Contents/MacOS/LemonMonitor",
            "/Applications/Tencent Lemon.app",
        )]);

        assert_eq!(
            inventory.application_icon_path_for_process("Tencent Lemon"),
            Some(&PathBuf::from("/Applications/Tencent Lemon.app"))
        );
        assert_eq!(
            inventory.application_icon_path_for_process("LemonMonitor"),
            Some(&PathBuf::from("/Applications/Tencent Lemon.app"))
        );
    }

    #[test]
    fn icon_lookup_does_not_guess_from_substrings() {
        let inventory = inventory(vec![application(
            "Example Helper Studio",
            "/Applications/Example Helper Studio.app/Contents/MacOS/Example Helper Studio",
            "/Applications/Example Helper Studio.app",
        )]);

        assert!(inventory
            .application_icon_path_for_process("Helper")
            .is_none());
    }

    #[test]
    fn application_process_lookup_does_not_fall_back_to_names_when_paths_exist() {
        let snapshot = ProcessSnapshot::from_process_names(vec![
            "/Applications/Other.app/Contents/MacOS/SharedHelper".to_string(),
        ]);

        assert!(snapshot
            .matching_application_processes(
                &["SharedHelper".to_string()],
                &[PathBuf::from(
                    "/Applications/Expected.app/Contents/MacOS/SharedHelper",
                )],
            )
            .is_empty());
    }

    #[test]
    fn application_process_lookup_blocks_when_windows_path_is_unavailable() {
        let snapshot = ProcessSnapshot::from_process_identities(vec![RunningProcessIdentity {
            executable_name: "Example.exe".to_string(),
            executable_path: None,
        }]);

        assert_eq!(
            snapshot.matching_application_processes(
                &["Example".to_string()],
                &[PathBuf::from(r"C:\Program Files\Example\Example.exe")],
            ),
            vec!["Example.exe".to_string()]
        );
    }

    #[test]
    fn application_process_lookup_rejects_known_same_name_path_mismatch() {
        let snapshot = ProcessSnapshot::from_process_identities(vec![RunningProcessIdentity {
            executable_name: "Example.exe".to_string(),
            executable_path: Some(PathBuf::from(r"C:\Other\Example.exe")),
        }]);

        assert!(snapshot
            .matching_application_processes(
                &["Example".to_string()],
                &[PathBuf::from(r"C:\Program Files\Example\Example.exe")],
            )
            .is_empty());
    }

    #[test]
    fn process_lookup_attributes_generic_macos_helpers_to_their_bundle_owner() {
        let snapshot = ProcessSnapshot::from_process_identities(vec![RunningProcessIdentity {
            executable_name: "browser_crashpad_handler".to_string(),
            executable_path: Some(PathBuf::from(
                "/Applications/ChatGPT.app/Contents/Frameworks/Codex Framework.framework/Helpers/browser_crashpad_handler",
            )),
        }]);

        assert_eq!(
            snapshot.matching_processes(&["ChatGPT".to_string()]),
            vec!["ChatGPT".to_string()]
        );
        assert!(snapshot
            .matching_processes(&["Chrome".to_string()])
            .is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn application_process_lookup_matches_canonical_and_display_paths() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-process-path-{}-{}",
            std::process::id(),
            crate::filesystem::metadata::now_ms()
        ));
        std::fs::create_dir_all(&root).expect("the process path fixture should be created");
        let executable = root.join("Example.exe");
        std::fs::write(&executable, b"fixture")
            .expect("the process executable fixture should be written");
        let canonical = std::fs::canonicalize(&executable)
            .expect("the process executable fixture should canonicalize");
        let snapshot = ProcessSnapshot::from_process_identities(vec![RunningProcessIdentity {
            executable_name: "Example.exe".to_string(),
            executable_path: Some(canonical),
        }]);

        assert_eq!(
            snapshot.matching_application_processes(
                &["Example".to_string()],
                std::slice::from_ref(&executable),
            ),
            vec!["Example.exe".to_string()]
        );

        std::fs::remove_dir_all(root).expect("the process path fixture should be removed");
    }

    #[test]
    fn icon_lookup_returns_none_for_equal_scoring_different_applications() {
        let inventory = inventory(vec![
            application(
                "First",
                "/Applications/First.app/Contents/MacOS/SharedHelper",
                "/Applications/First.app",
            ),
            application(
                "Second",
                "/Applications/Second.app/Contents/MacOS/SharedHelper",
                "/Applications/Second.app",
            ),
        ]);

        assert!(inventory
            .application_icon_path_for_process("SharedHelper")
            .is_none());
    }

    #[test]
    fn icon_lookup_prefers_the_exact_running_executable_path() {
        let inventory = inventory(vec![
            application(
                "ChatGPT",
                "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT",
                "/Applications/ChatGPT.app",
            ),
            application(
                "ChatGPT",
                "/Applications/ChatGPT Classic.app/Contents/MacOS/ChatGPT",
                "/Applications/ChatGPT Classic.app",
            ),
        ]);
        let snapshot = ProcessSnapshot::from_process_names(vec![
            "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT".to_string(),
        ]);

        assert_eq!(
            inventory.application_icon_path_for_running_process("ChatGPT", &snapshot),
            Some(&PathBuf::from("/Applications/ChatGPT.app"))
        );
    }

    #[test]
    fn icon_lookup_uses_rule_application_identifiers_for_external_helpers() {
        let mut zenaion = application(
            "ZenAion",
            "/Applications/ZenAion.app/Contents/MacOS/ZenAI",
            "/Applications/ZenAion.app",
        );
        zenaion.primary_identifier = "bot.zenai".to_string();
        zenaion.identifiers.push("bot.zenai".to_string());
        let inventory = inventory(vec![zenaion]);

        assert!(inventory
            .application_icon_path_for_process("zenai-host")
            .is_none());
        assert_eq!(
            inventory.application_icon_path_for_identifiers(&["bot.zenai".to_string()]),
            Some(&PathBuf::from("/Applications/ZenAion.app"))
        );
    }

    #[test]
    fn icon_lookup_uses_the_display_name_for_a_registry_ico() {
        let display_name = "\u{641c}\u{72d7}\u{9ad8}\u{901f}\u{6d4f}\u{89c8}\u{5668}";
        let mut sogou = application(display_name, "", "C:/Program Files/Sogou/app_sogou.ico");
        sogou.bundle_path = None;
        sogou.executable_paths.clear();
        let inventory = inventory(vec![sogou]);

        assert!(inventory
            .application_icon_path_for_process("SogouExplorer.exe")
            .is_none());
        assert_eq!(
            inventory.application_icon_path_for_identifiers(&[
                "Sogou Explorer".to_string(),
                display_name.to_string(),
                "SogouExplorer".to_string(),
            ]),
            Some(&PathBuf::from("C:/Program Files/Sogou/app_sogou.ico"))
        );
    }
}
