use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Instant,
};

use mangodisk_platform::FileReadStage;

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

#[derive(Debug)]
enum RootResolutionError {
    ReadFailed {
        path: PathBuf,
        error: std::io::Error,
    },
    Failed(String),
}

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
    let mut roots = Vec::new();
    let mut discovery_read_failures = mangodisk_platform::FileReadFailures::default();
    for root in &source.roots {
        match resolve_root_source(root, &mut discovery_read_failures) {
            Ok(resolved) => roots.extend(resolved),
            Err(RootResolutionError::ReadFailed { path, error }) => {
                discovery_read_failures.record(&path, &error, FileReadStage::OpenDirectory);
                log::warn!(
                    "cleanup_dynamic_root_skipped rule_id={} root={} error={} outcome=partial_scan",
                    source.id,
                    diagnostic_path(&path),
                    mangodisk_platform::diagnostics::text(&error)
                );
            }
            Err(RootResolutionError::Failed(diagnostic)) => {
                return Err(format!("Declarative rule {}: {diagnostic}", source.id));
            }
        }
    }
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
        discovery_read_failures,
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

fn resolve_root_source(
    source: &DeclarativeRootSource,
    failures: &mut mangodisk_platform::FileReadFailures,
) -> Result<Vec<RootSpec>, RootResolutionError> {
    let base = resolve_root_template(&source.template).map_err(RootResolutionError::Failed)?;
    match source.kind {
        DeclarativeRootKind::Static => Ok(vec![RootSpec {
            resolved_path: base,
        }]),
        DeclarativeRootKind::ChildDirectories => {
            let children = enumerate_dynamic_children(&base, source, failures)?;
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
    failures: &mut mangodisk_platform::FileReadFailures,
) -> Result<Vec<PathBuf>, RootResolutionError> {
    let enumeration_started = Instant::now();
    log::debug!(
        "cleanup_dynamic_root_enumeration_started root={}",
        diagnostic_path(base)
    );
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(RootResolutionError::ReadFailed {
                path: base.to_path_buf(),
                error,
            })
        }
    };
    let mut children = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.record(base, &error, FileReadStage::ReadDirectory);
                continue;
            }
        };
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                failures.record(&path, &error, FileReadStage::ReadMetadata);
                continue;
            }
        };
        if !metadata.is_dir() || is_link_like(&metadata) {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if source.include_all_children
            || source
                .child_names
                .iter()
                .any(|candidate| name.eq_ignore_ascii_case(candidate))
            || source.child_prefixes.iter().any(|prefix| {
                name.to_ascii_lowercase()
                    .starts_with(&prefix.to_ascii_lowercase())
            })
        {
            children.push(path);
        }
    }
    children.sort();
    log::debug!(
        "cleanup_dynamic_root_enumeration_finished root={} child_count={} elapsed_ms={}",
        diagnostic_path(base),
        children.len(),
        enumeration_started.elapsed().as_millis()
    );
    Ok(children)
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

#[cfg(all(test, any(target_os = "macos", windows)))]
#[path = "notion_and_claude_code_tests.rs"]
mod notion_and_claude_code_tests;

#[cfg(all(test, any(target_os = "macos", windows)))]
#[path = "brave_origin_tests.rs"]
mod brave_origin_tests;

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
    fn brave_origin_macos_cache_has_its_own_profile_roots() {
        let parsed = parse_catalog(EMBEDDED_DECLARATIVE_RULE_SOURCES).unwrap();
        let rule = parsed
            .iter()
            .find(|parsed| {
                parsed.rule.id == "browser.brave-origin-cache"
                    && parsed.rule.platform == SourcePlatform::Macos
            })
            .expect("standalone Brave Origin must have a macOS cache rule");
        assert!(rule.rule.roots.iter().all(|root| {
            root.template.contains("/BraveSoftware/Brave-Origin")
                && !root.template.contains("Brave-Browser")
        }));
        assert!(rule
            .rule
            .required_stopped_processes
            .contains(&"Brave Origin".to_string()));
    }

    #[test]
    fn brave_origin_windows_cache_has_its_own_profile_roots() {
        let parsed = parse_catalog(EMBEDDED_DECLARATIVE_RULE_SOURCES)
            .expect("embedded rules must pass runtime validation");
        let rule = parsed
            .iter()
            .find(|parsed| {
                parsed.rule.id == "browser.brave-origin-cache"
                    && parsed.rule.platform == SourcePlatform::Windows
            })
            .expect("standalone Brave Origin must have a Windows cache rule");
        assert!(rule.rule.roots.iter().all(|root| root
            .template
            .starts_with("${local_app_data}/BraveSoftware/Brave-Origin/User Data")));
        assert!(rule
            .rule
            .required_stopped_processes
            .contains(&"brave.exe".to_string()));
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

    #[test]
    fn notion_and_claude_code_rules_keep_narrow_cross_platform_boundaries() {
        let parsed = parse_catalog(EMBEDDED_DECLARATIVE_RULE_SOURCES)
            .expect("embedded rules must pass runtime validation");
        let notion = parsed
            .iter()
            .filter(|entry| entry.rule.id == "app.notion-service-worker-cache")
            .collect::<Vec<_>>();
        let claude = parsed
            .iter()
            .filter(|entry| entry.rule.id == "ai.claude-code-cache")
            .collect::<Vec<_>>();

        assert_eq!(notion.len(), 2, "Notion must cover macOS and Windows");
        assert_eq!(claude.len(), 2, "Claude Code must cover macOS and Windows");

        for entry in notion {
            let rule = &entry.rule;
            let (template, process) = match rule.platform {
                SourcePlatform::Macos => ("${application_support}/Notion/Partitions", "Notion"),
                SourcePlatform::Windows => ("${roaming_app_data}/Notion/Partitions", "Notion.exe"),
                SourcePlatform::Linux => panic!("Notion has no verified Linux cleanup rule"),
            };
            assert_eq!(rule.risk, SourceRisk::Recoverable);
            assert!(!rule.default_selected);
            assert_eq!(rule.recommended_selected, Some(true));
            assert!(rule.execution.requires_app_close());
            assert_eq!(rule.required_stopped_processes, [process]);
            assert_eq!(rule.roots.len(), 1);
            assert_eq!(rule.roots[0].template, template);
            assert_eq!(rule.roots[0].kind, DeclarativeRootKind::ChildDirectories);
            assert!(rule.roots[0].include_all_children);
            assert_eq!(rule.roots[0].suffixes, ["Service Worker/CacheStorage"]);
            assert!(rule.roots[0].verified_rebuildable);
            assert_eq!(rule.matcher, DeclarativeMatcherSource::All);
        }

        for entry in claude {
            let rule = &entry.rule;
            assert_eq!(rule.risk, SourceRisk::Safe);
            assert!(!rule.execution.requires_app_close());
            assert!(rule.required_stopped_processes.is_empty());
            assert_eq!(rule.roots.len(), 1);
            assert_eq!(rule.roots[0].kind, DeclarativeRootKind::Static);
            assert_eq!(rule.roots[0].template, "${home}/.claude/cache");
            assert_eq!(
                rule.matcher,
                DeclarativeMatcherSource::AllOf {
                    items: vec![
                        DeclarativeMatcherSource::NameEquals {
                            values: vec!["changelog.md".to_string()],
                        },
                        DeclarativeMatcherSource::MaxDepth { depth: 1 },
                    ],
                }
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

        let roots = resolve_root_source(&source, &mut Default::default())
            .expect("dynamic roots must resolve");
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
            resolve_root_source(&source, &mut Default::default())
                .expect("a missing optional application root must be inapplicable")
                .len(),
            0
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unreadable_dynamic_root_keeps_other_roots_and_records_incomplete_scan() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = tempfile::tempdir().expect("temporary fixture must be created");
        let available_root = fixture.path().join("Caches");
        fs::create_dir(&available_root).expect("static root must be created");
        let root = fixture.path().join("profiles");
        fs::create_dir(&root).expect("dynamic root must be created");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o000))
            .expect("dynamic root must become unreadable");
        let mut source = parse_current_platform_catalog()
            .expect("embedded catalog must parse")
            .into_iter()
            .find(|rule| rule.id == "browser.brave-cache")
            .expect("Brave rule must exist on macOS");
        let mut static_root = source.roots[0].clone();
        static_root.template = format!(
            "${{temp}}/{}",
            available_root
                .strip_prefix(env::temp_dir())
                .unwrap()
                .display()
        );
        let mut dynamic_root = source.roots[1].clone();
        dynamic_root.template = format!(
            "${{temp}}/{}",
            root.strip_prefix(env::temp_dir()).unwrap().display()
        );
        source.roots = vec![static_root, dynamic_root];

        let result = compile_declarative_source(source);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("dynamic root permissions must be restored");
        let rule = result.expect("one unreadable optional root must not abort the catalog");
        assert_eq!(rule.roots.len(), 1);
        assert_eq!(rule.roots[0].resolved_path, available_root);
        assert_eq!(rule.discovery_read_failures.count, 1);
        assert_eq!(
            rule.discovery_read_failures.privacy_restricted_count, 0,
            "ordinary directory permissions must not imply macOS privacy restrictions"
        );
        assert_eq!(
            std::io::Error::from_raw_os_error(1).kind(),
            ErrorKind::PermissionDenied
        );
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
        let roots = resolve_root_source(&fixture_source, &mut Default::default())
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
