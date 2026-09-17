use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Instant,
};

#[cfg(target_os = "macos")]
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use crate::cleanup::CleanupCategory;
use crate::{
    cleanup::rules::{
        declarative_schema::{
            parse_catalog, DeclarativeApplicabilitySource, DeclarativeExecutionSource,
            DeclarativeMatcherSource, DeclarativeRootKind, DeclarativeRootSource,
            DeclarativeRuleSource, RootVariable, SourceCategory, SourceLifecycle, SourcePlatform,
            SourceRisk,
        },
        models::{
            ApplicabilityProbe, ExecutionSpec, MatcherSpec, PlatformConstraint, RootSpec,
            RuleLifecycle, RuleRiskLevel, RuleSpec, VerificationMetadata,
        },
    },
    filesystem::metadata::{diagnostic_path, is_link_like},
};

include!(concat!(env!("OUT_DIR"), "/embedded-cleanup-rules.rs"));

static PARSED_PLATFORM_CATALOG: OnceLock<Result<Vec<DeclarativeRuleSource>, String>> =
    OnceLock::new();

#[cfg(target_os = "macos")]
static PRIVACY_MANAGED_DYNAMIC_ROOTS_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
const PRIVACY_MANAGED_ROOT_TIMEOUT: Duration = Duration::from_secs(2);

/// Loads every validated declarative rule for the current platform.
///
/// Sources are discovered by `build.rs`; contributors only add a TOML file
/// and never register the rule in Rust. Parsed source data is cached, while
/// filesystem-dependent dynamic roots are resolved on every call so profiles
/// created during the application session become visible without a restart.
pub(crate) fn load_current_platform() -> Result<Vec<RuleSpec>, String> {
    let declarative = PARSED_PLATFORM_CATALOG
        .get_or_init(parse_current_platform_catalog)
        .clone()?;
    let specs = declarative
        .into_iter()
        .map(compile_declarative_source)
        .collect::<Result<Vec<_>, _>>()?;
    log::debug!(
        "cleanup_declarative_catalog_loaded platform={} source_count={} active_rule_count={}",
        current_source_platform().as_str(),
        EMBEDDED_DECLARATIVE_RULE_SOURCES.len(),
        specs.len()
    );
    Ok(specs)
}

fn parse_current_platform_catalog() -> Result<Vec<DeclarativeRuleSource>, String> {
    // Re-parse embedded sources at runtime to preserve the validation boundary
    // needed by future signed rule packs.
    let parsed = parse_catalog(EMBEDDED_DECLARATIVE_RULE_SOURCES)?;
    let current_platform = current_source_platform();
    let mut declarative = parsed
        .into_iter()
        .filter(|parsed| parsed.rule.platform == current_platform)
        .map(|parsed| parsed.rule)
        .collect::<Vec<_>>();
    declarative.sort_by(|left, right| {
        category(left.category)
            .as_str()
            .cmp(category(right.category).as_str())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(declarative)
}

fn compile_declarative_source(source: DeclarativeRuleSource) -> Result<RuleSpec, String> {
    let platform = platform_constraint(source.platform)?;
    let execution = match source.execution {
        DeclarativeExecutionSource::DeleteMatchingContents { requires_app_close } => {
            ExecutionSpec::DeleteMatchingContents { requires_app_close }
        }
        DeclarativeExecutionSource::DeleteWholeRoot { requires_app_close } => {
            ExecutionSpec::DeleteWholeRoot { requires_app_close }
        }
    };
    let roots = source
        .roots
        .iter()
        .map(|root| {
            resolve_root_source(root)
                .map_err(|error| format!("Declarative rule {}: {error}", source.id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let roots = roots.into_iter().flatten().collect();
    let applicability = source
        .applicability
        .iter()
        .cloned()
        .map(|probe| applicability(probe, &source.id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RuleSpec {
        id: source.id,
        schema_version: source.schema_version,
        rule_version: source.rule_version,
        platform,
        category: category(source.category),
        risk: risk(source.risk),
        default_selected: source.default_selected,
        recommended_selected: source
            .recommended_selected
            .unwrap_or(source.default_selected),
        applicability,
        roots,
        matcher: matcher(source.matcher),
        execution,
        required_stopped_processes: source.required_stopped_processes,
        verification: VerificationMetadata {
            lifecycle: lifecycle(source.verification.lifecycle),
            evidence: source.verification.evidence,
            verified_at: source.verification.verified_at,
            verified_platform: platform_constraint(source.verification.verified_platform)?,
        },
    })
}

fn resolve_root_source(source: &DeclarativeRootSource) -> Result<Vec<RootSpec>, String> {
    let base = resolve_root_template(&source.template)?;
    match source.kind {
        DeclarativeRootKind::Static => Ok(vec![RootSpec {
            resolved_path: base,
        }]),
        DeclarativeRootKind::ChildDirectories => {
            let children = enumerate_dynamic_children(&base, source)?;
            Ok(children
                .into_iter()
                .flat_map(|child| {
                    source.suffixes.iter().map(move |suffix| RootSpec {
                        resolved_path: suffix
                            .split(['/', '\\'])
                            .fold(child.clone(), |path, component| path.join(component)),
                    })
                })
                .collect())
        }
    }
}

fn enumerate_dynamic_children(
    base: &Path,
    source: &DeclarativeRootSource,
) -> Result<Vec<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    if is_macos_privacy_managed_root(base) {
        return enumerate_privacy_managed_children(base, source);
    }

    enumerate_dynamic_children_sync(base, source)
}

fn enumerate_dynamic_children_sync(
    base: &Path,
    source: &DeclarativeRootSource,
) -> Result<Vec<PathBuf>, String> {
    let enumeration_started = Instant::now();
    log::debug!(
        "cleanup_dynamic_root_enumeration_started root={}",
        diagnostic_path(base)
    );
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "failed to enumerate dynamic root {}: {error}",
                diagnostic_path(base)
            ))
        }
    };
    let mut children = entries
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry),
            Err(error) => {
                log::debug!(
                    "cleanup_dynamic_root_entry_skipped root={} error={}",
                    diagnostic_path(base),
                    error
                );
                None
            }
        })
        .filter_map(|entry| {
            fs::symlink_metadata(entry.path())
                .ok()
                .filter(|metadata| metadata.is_dir() && !is_link_like(metadata))
                .map(|_| entry.path())
        })
        .filter(|path| {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default();
            source.include_all_children
                || source
                    .child_names
                    .iter()
                    .any(|candidate| name.eq_ignore_ascii_case(candidate))
                || source.child_prefixes.iter().any(|prefix| {
                    name.to_ascii_lowercase()
                        .starts_with(&prefix.to_ascii_lowercase())
                })
        })
        .collect::<Vec<_>>();
    children.sort();
    log::debug!(
        "cleanup_dynamic_root_enumeration_finished root={} child_count={} elapsed_ms={}",
        diagnostic_path(base),
        children.len(),
        enumeration_started.elapsed().as_millis()
    );
    Ok(children)
}

#[cfg(target_os = "macos")]
fn is_macos_privacy_managed_root(path: &Path) -> bool {
    let Ok(home) = user_home() else {
        return false;
    };
    let library = home.join("Library");
    path.starts_with(library.join("Containers"))
        || path.starts_with(library.join("Group Containers"))
}

#[cfg(target_os = "macos")]
fn enumerate_privacy_managed_children(
    base: &Path,
    source: &DeclarativeRootSource,
) -> Result<Vec<PathBuf>, String> {
    if PRIVACY_MANAGED_DYNAMIC_ROOTS_UNAVAILABLE.load(Ordering::Relaxed) {
        log::debug!(
            "cleanup_dynamic_root_enumeration_skipped root={} reason=privacy_managed_access_unavailable",
            diagnostic_path(base)
        );
        return Ok(Vec::new());
    }

    // macOS may indefinitely block an application while it opens another
    // sandboxed application's container. Resolve only these privacy-managed
    // dynamic roots on a bounded worker so one inaccessible cache cannot hold
    // the whole scan or make cancellation appear broken.
    let worker_base = base.to_path_buf();
    let worker_source = source.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("cleanup-dynamic-root".to_string())
        .spawn(move || {
            let _ = sender.send(enumerate_dynamic_children_sync(
                &worker_base,
                &worker_source,
            ));
        })
        .map_err(|error| {
            format!(
                "failed to start dynamic-root enumeration for {}: {error}",
                diagnostic_path(base)
            )
        })?;

    match receiver.recv_timeout(PRIVACY_MANAGED_ROOT_TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Container privacy is granted to the process rather than to one
            // individual application directory. After the first timeout,
            // skip the remaining optional dynamic container roots so scan
            // startup pays the bounded wait only once.
            PRIVACY_MANAGED_DYNAMIC_ROOTS_UNAVAILABLE.store(true, Ordering::Relaxed);
            log::warn!(
                "cleanup_dynamic_root_enumeration_timed_out root={} timeout_ms={} action=skip_privacy_managed_roots_until_restart",
                diagnostic_path(base),
                PRIVACY_MANAGED_ROOT_TIMEOUT.as_millis()
            );
            Ok(Vec::new())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(format!(
            "dynamic-root enumeration worker stopped before reporting {}",
            diagnostic_path(base)
        )),
    }
}

fn applicability(
    source: DeclarativeApplicabilitySource,
    rule_id: &str,
) -> Result<ApplicabilityProbe, String> {
    Ok(match source {
        DeclarativeApplicabilitySource::AnyRootExists => ApplicabilityProbe::AnyRootExists,
        DeclarativeApplicabilitySource::PathExists { template } => {
            ApplicabilityProbe::PathExists(resolve_root_template(&template).map_err(|error| {
                format!("failed to resolve an applicability path for rule {rule_id}: {error}")
            })?)
        }
        DeclarativeApplicabilitySource::ApplicationInstalled { identifiers } => {
            ApplicabilityProbe::ApplicationInstalled(identifiers)
        }
        DeclarativeApplicabilitySource::ExecutableAvailable { names } => {
            ApplicabilityProbe::ExecutableAvailable(names)
        }
        DeclarativeApplicabilitySource::ApplicationVersion {
            identifier,
            minimum,
            maximum_exclusive,
        } => ApplicabilityProbe::ApplicationVersion {
            identifier,
            minimum,
            maximum_exclusive,
        },
        DeclarativeApplicabilitySource::SystemVersion {
            minimum,
            maximum_exclusive,
        } => ApplicabilityProbe::SystemVersion {
            minimum,
            maximum_exclusive,
        },
        DeclarativeApplicabilitySource::FileSystemIn { values } => {
            ApplicabilityProbe::FileSystemIn(values)
        }
        DeclarativeApplicabilitySource::CapabilityAvailable { values } => {
            ApplicabilityProbe::CapabilityAvailable(values)
        }
        DeclarativeApplicabilitySource::ProcessRunning { values } => {
            ApplicabilityProbe::ProcessRunning(values)
        }
        DeclarativeApplicabilitySource::AnyOf { items } => ApplicabilityProbe::AnyOf(
            items
                .into_iter()
                .map(|item| applicability(item, rule_id))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        DeclarativeApplicabilitySource::AllOf { items } => ApplicabilityProbe::AllOf(
            items
                .into_iter()
                .map(|item| applicability(item, rule_id))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        DeclarativeApplicabilitySource::Not { item } => {
            ApplicabilityProbe::Not(Box::new(applicability(*item, rule_id)?))
        }
    })
}

fn resolve_root_template(template: &str) -> Result<PathBuf, String> {
    let parts = super::declarative_schema::parse_root_template(template)?;
    let base = match parts.variable {
        RootVariable::Temp => env::temp_dir(),
        RootVariable::Home => user_home()?,
        #[cfg(target_os = "macos")]
        RootVariable::UserLibrary => user_home()?.join("Library"),
        #[cfg(target_os = "macos")]
        RootVariable::ApplicationSupport => user_home()?.join("Library/Application Support"),
        #[cfg(target_os = "macos")]
        RootVariable::SystemRoot => PathBuf::from("/"),
        #[cfg(target_os = "macos")]
        RootVariable::DarwinUserCache => env::temp_dir()
            .parent()
            .map(|parent| parent.join("C"))
            .unwrap_or_else(env::temp_dir),
        #[cfg(target_os = "macos")]
        RootVariable::LocalAppData
        | RootVariable::RoamingAppData
        | RootVariable::ProgramFiles
        | RootVariable::ProgramData => {
            return Err(format!(
                "variable ${{{}}} is not available on macOS",
                parts.variable.as_str()
            ));
        }
        #[cfg(windows)]
        RootVariable::LocalAppData => required_environment_path("LOCALAPPDATA")?,
        #[cfg(windows)]
        RootVariable::RoamingAppData => required_environment_path("APPDATA")?,
        #[cfg(windows)]
        RootVariable::SystemRoot => required_environment_path("SystemRoot")?,
        #[cfg(windows)]
        RootVariable::ProgramFiles => required_environment_path("ProgramFiles")?,
        #[cfg(windows)]
        RootVariable::ProgramData => required_environment_path("PROGRAMDATA")?,
        #[cfg(windows)]
        RootVariable::UserLibrary
        | RootVariable::ApplicationSupport
        | RootVariable::DarwinUserCache => {
            return Err(format!(
                "variable ${{{}}} is not available on Windows",
                parts.variable.as_str()
            ));
        }
        #[cfg(target_os = "linux")]
        RootVariable::LocalAppData
        | RootVariable::RoamingAppData
        | RootVariable::SystemRoot
        | RootVariable::ProgramFiles
        | RootVariable::ProgramData
        | RootVariable::UserLibrary
        | RootVariable::ApplicationSupport
        | RootVariable::DarwinUserCache => {
            return Err(format!(
                "variable ${{{}}} is not available on Linux",
                parts.variable.as_str()
            ));
        }
    };
    Ok(parts
        .suffix
        .into_iter()
        .fold(base, |path, component| path.join(component)))
}

fn user_home() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    let value = env::var_os("HOME");
    #[cfg(windows)]
    let value = env::var_os("USERPROFILE");
    #[cfg(target_os = "linux")]
    let value = env::var_os("HOME");
    value
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "failed to resolve the current user's home directory".to_string())
}

#[cfg(windows)]
fn required_environment_path(name: &str) -> Result<PathBuf, String> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| format!("failed to resolve controlled system path variable {name}"))
}

fn matcher(source: DeclarativeMatcherSource) -> MatcherSpec {
    match source {
        DeclarativeMatcherSource::All => MatcherSpec::All,
        DeclarativeMatcherSource::NameEquals { values } => MatcherSpec::NameEquals(values),
        DeclarativeMatcherSource::NameGlob { values } => MatcherSpec::NameGlob(values),
        DeclarativeMatcherSource::ExtensionIn { values } => MatcherSpec::ExtensionIn(values),
        DeclarativeMatcherSource::PathSegmentIn { values } => MatcherSpec::PathSegmentIn(values),
        DeclarativeMatcherSource::OlderThan { days } => MatcherSpec::OlderThanDays(days),
        DeclarativeMatcherSource::LargerThan { bytes } => MatcherSpec::LargerThanBytes(bytes),
        DeclarativeMatcherSource::SmallerThan { bytes } => MatcherSpec::SmallerThanBytes(bytes),
        DeclarativeMatcherSource::MaxDepth { depth } => MatcherSpec::MaxDepth(depth),
        DeclarativeMatcherSource::AllOf { items } => {
            MatcherSpec::AllOf(items.into_iter().map(matcher).collect())
        }
        DeclarativeMatcherSource::AnyOf { items } => {
            MatcherSpec::AnyOf(items.into_iter().map(matcher).collect())
        }
        DeclarativeMatcherSource::Not { item } => MatcherSpec::Not(Box::new(matcher(*item))),
    }
}

const fn current_source_platform() -> SourcePlatform {
    #[cfg(target_os = "macos")]
    {
        SourcePlatform::Macos
    }
    #[cfg(windows)]
    {
        SourcePlatform::Windows
    }
    #[cfg(target_os = "linux")]
    {
        SourcePlatform::Linux
    }
}

fn platform_constraint(platform: SourcePlatform) -> Result<PlatformConstraint, String> {
    match platform {
        #[cfg(target_os = "macos")]
        SourcePlatform::Macos => Ok(PlatformConstraint::Macos),
        #[cfg(windows)]
        SourcePlatform::Windows => Ok(PlatformConstraint::Windows),
        #[cfg(target_os = "linux")]
        SourcePlatform::Linux => Ok(PlatformConstraint::Linux),
        _ => Err(format!(
            "declarative rule platform {} does not match the current build target",
            platform.as_str()
        )),
    }
}

const fn category(value: SourceCategory) -> CleanupCategory {
    match value {
        SourceCategory::System => CleanupCategory::System,
        SourceCategory::Browser => CleanupCategory::Browser,
        SourceCategory::Application => CleanupCategory::Application,
        SourceCategory::Development => CleanupCategory::Development,
        SourceCategory::Ai => CleanupCategory::Ai,
        SourceCategory::Container => CleanupCategory::Container,
    }
}

const fn risk(value: SourceRisk) -> RuleRiskLevel {
    match value {
        SourceRisk::Safe => RuleRiskLevel::Safe,
        SourceRisk::Recoverable => RuleRiskLevel::Recoverable,
        SourceRisk::HighImpact => RuleRiskLevel::HighImpact,
    }
}

const fn lifecycle(value: SourceLifecycle) -> RuleLifecycle {
    match value {
        SourceLifecycle::Candidate => RuleLifecycle::Candidate,
        SourceLifecycle::Verified => RuleLifecycle::Verified,
        SourceLifecycle::Stable => RuleLifecycle::Stable,
        SourceLifecycle::Deprecated => RuleLifecycle::Deprecated,
        SourceLifecycle::Disabled => RuleLifecycle::Disabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn declarative_sources_auto_register_without_rust_placeholders() {
        let specs = load_current_platform().expect("embedded rules must pass runtime validation");
        assert!(!specs.is_empty());
        assert!(specs
            .iter()
            .all(|rule| !rule.verification.evidence.is_empty()));
    }

    #[test]
    fn chromium_offline_cache_rules_preserve_service_worker_scripts() {
        let parsed = parse_catalog(EMBEDDED_DECLARATIVE_RULE_SOURCES)
            .expect("embedded rules must pass runtime validation");
        let rules = parsed
            .iter()
            .filter(|parsed| {
                matches!(
                    parsed.rule.id.as_str(),
                    "browser.chrome-offline-cache" | "browser.edge-offline-cache"
                )
            })
            .collect::<Vec<_>>();

        // Chrome and Edge store registration metadata separately from the
        // referenced scripts. Deleting ScriptCache alone leaves a valid-looking
        // registration whose background worker cannot start, so every platform
        // variant must keep that directory outside the cleanup boundary.
        assert_eq!(rules.len(), 4);
        for parsed in rules {
            let suffixes = parsed
                .rule
                .roots
                .iter()
                .flat_map(|root| root.suffixes.iter().map(String::as_str))
                .collect::<Vec<_>>();
            assert!(
                suffixes.contains(&"Service Worker/CacheStorage"),
                "{} must continue selecting rebuildable offline data",
                parsed.source_name
            );
            assert!(
                !suffixes.contains(&"Service Worker/ScriptCache"),
                "{} must preserve registered service worker scripts",
                parsed.source_name
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn browser_automation_rule_preserves_selenium_user_configuration() {
        let specs = load_current_platform().expect("embedded rules must pass runtime validation");
        let rule = specs
            .iter()
            .find(|rule| rule.id == "dev.browser-automation-cache")
            .expect("the browser automation rule must be registered");

        assert!(rule
            .roots
            .iter()
            .any(|root| root.resolved_path.ends_with("Library/Caches/Cypress")));
        assert!(rule
            .roots
            .iter()
            .any(|root| root.resolved_path.ends_with(".cache/selenium")));
        assert_eq!(
            rule.matcher,
            MatcherSpec::Not(Box::new(MatcherSpec::NameEquals(vec![
                "se-config.toml".to_string()
            ])))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn chrome_rule_includes_browser_level_shader_caches() {
        let specs = load_current_platform().expect("embedded rules must pass runtime validation");
        let rule = specs
            .iter()
            .find(|rule| rule.id == "browser.chrome-cache")
            .expect("the Chrome cache rule must be registered");
        let application_support = env::var_os("HOME")
            .map(PathBuf::from)
            .expect("HOME must be available")
            .join("Library/Application Support/Google/Chrome");

        for cache_name in [
            "ShaderCache",
            "GrShaderCache",
            "GraphiteDawnCache",
            "GPUPersistentCache",
        ] {
            assert!(rule
                .roots
                .iter()
                .any(|root| root.resolved_path == application_support.join(cache_name)));
        }
        assert!(rule
            .roots
            .iter()
            .all(|root| !root.resolved_path.ends_with("Local State")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn gradle_rule_includes_rebuildable_wrapper_and_temp_data() {
        let specs = load_current_platform().expect("embedded rules must pass runtime validation");
        let rule = specs
            .iter()
            .find(|rule| rule.id == "dev.gradle-cache")
            .expect("the Gradle cache rule must be registered");
        let gradle_home = env::var_os("HOME")
            .map(PathBuf::from)
            .expect("HOME must be available")
            .join(".gradle");

        for relative in ["wrapper/dists", ".tmp"] {
            assert!(rule
                .roots
                .iter()
                .any(|root| root.resolved_path == gradle_home.join(relative)));
        }
        assert!(rule
            .roots
            .iter()
            .all(|root| root.resolved_path != gradle_home.join("gradle.properties")));
    }

    #[test]
    fn dynamic_roots_select_direct_children_and_append_fixed_suffixes() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be valid")
            .as_nanos();
        let fixture_name = format!("mangodisk-dynamic-root-{unique}");
        let fixture = env::temp_dir().join(&fixture_name);
        let default_profile = fixture.join("Default");
        let numbered_profile = fixture.join("Profile 2");
        let ignored_profile = fixture.join("Other");
        for directory in [&default_profile, &numbered_profile, &ignored_profile] {
            fs::create_dir_all(directory).expect("dynamic-root fixture must be created");
        }

        let source = DeclarativeRootSource {
            template: format!("${{temp}}/{fixture_name}"),
            kind: DeclarativeRootKind::ChildDirectories,
            child_names: vec!["Default".to_string()],
            child_prefixes: vec!["Profile ".to_string()],
            include_all_children: false,
            suffixes: vec!["Cache/Data".to_string()],
            verified_rebuildable: false,
        };

        let roots = resolve_root_source(&source).expect("dynamic roots must resolve");
        assert_eq!(
            roots
                .into_iter()
                .map(|root| root.resolved_path)
                .collect::<Vec<_>>(),
            vec![
                default_profile.join("Cache/Data"),
                numbered_profile.join("Cache/Data"),
            ]
        );

        fs::remove_dir_all(fixture).expect("dynamic-root fixture must be removed");
    }

    #[test]
    fn missing_dynamic_root_is_not_an_error() {
        let source = DeclarativeRootSource {
            template: format!(
                "${{temp}}/mangodisk-missing-dynamic-root-{}",
                std::process::id()
            ),
            kind: DeclarativeRootKind::ChildDirectories,
            child_names: vec!["Default".to_string()],
            child_prefixes: Vec::new(),
            include_all_children: false,
            suffixes: vec!["Cache".to_string()],
            verified_rebuildable: false,
        };

        assert_eq!(
            resolve_root_source(&source)
                .expect("a missing optional application root must be inapplicable")
                .len(),
            0
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn privacy_managed_dynamic_roots_are_recognized_without_matching_normal_library_data() {
        let home = user_home().expect("HOME must be available");

        assert!(is_macos_privacy_managed_root(
            &home.join("Library/Containers/com.example.app/Data/Library/Caches")
        ));
        assert!(is_macos_privacy_managed_root(
            &home.join("Library/Group Containers/group.com.example.app/Caches")
        ));
        assert!(!is_macos_privacy_managed_root(
            &home.join("Library/Application Support/Example/Partitions")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn teams_msix_dynamic_roots_only_expand_profile_directories() {
        let catalog = parse_current_platform_catalog().expect("embedded rules must be valid");
        let teams_rule = catalog
            .iter()
            .find(|rule| rule.id == "app.teams-msix-cache")
            .expect("the Teams MSIX cache rule must be registered");
        let dynamic_source = teams_rule
            .roots
            .iter()
            .find(|root| root.kind == DeclarativeRootKind::ChildDirectories)
            .expect("the Teams rule must define profile cache roots");

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be valid")
            .as_nanos();
        let fixture_name = format!("mangodisk-teams-msix-root-{unique}");
        let fixture = env::temp_dir().join(&fixture_name);
        let default_profile = fixture.join("Default");
        let numbered_profile = fixture.join("Profile 2");
        let shader_cache = fixture.join("ShaderCache");
        for directory in [&default_profile, &numbered_profile, &shader_cache] {
            fs::create_dir_all(directory.join("Cache"))
                .expect("Teams WebView2 cache fixture must be created");
        }

        let mut fixture_source = dynamic_source.clone();
        fixture_source.template = format!("${{temp}}/{fixture_name}");
        let roots = resolve_root_source(&fixture_source)
            .expect("Teams profile cache roots must resolve")
            .into_iter()
            .map(|root| root.resolved_path)
            .collect::<Vec<_>>();

        assert!(roots.contains(&default_profile.join("Cache")));
        assert!(roots.contains(&numbered_profile.join("Cache")));
        assert!(roots.iter().all(|root| !root.starts_with(&shader_cache)));

        fs::remove_dir_all(fixture).expect("Teams WebView2 cache fixture must be removed");
    }
}
