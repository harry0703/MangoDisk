use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use super::protected_paths::{
    is_protected_home_relative_path, is_protected_library_root, is_protected_repository_component,
};

const RULE_SCHEMA_VERSION: u32 = 3;
const MAX_MATCHER_DEPTH: usize = 8;
const MAX_MATCHER_VALUES: usize = 64;
const MAX_ROOTS_PER_RULE: usize = 32;
const MAX_TEXT_VALUE_BYTES: usize = 256;
const MAX_RULE_ID_BYTES: usize = 128;
const MAX_ROOT_TEMPLATE_BYTES: usize = 1_024;
const MAX_ROOT_COMPONENTS: usize = 32;
const MAX_VERIFICATION_REFERENCES: usize = 8;
const MAX_REFERENCE_BYTES: usize = 1_024;

#[derive(Debug, Clone)]
pub(crate) struct ParsedDeclarativeRule {
    pub source_name: String,
    pub rule: DeclarativeRuleSource,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeclarativeRuleSource {
    pub id: String,
    pub schema_version: u32,
    pub rule_version: u32,
    pub platform: SourcePlatform,
    pub category: SourceCategory,
    pub risk: SourceRisk,
    pub default_selected: bool,
    #[serde(default)]
    pub recommended_selected: Option<bool>,
    #[serde(default)]
    pub required_stopped_processes: Vec<String>,
    pub applicability: Vec<DeclarativeApplicabilitySource>,
    pub roots: Vec<DeclarativeRootSource>,
    pub matcher: DeclarativeMatcherSource,
    pub execution: DeclarativeExecutionSource,
    pub verification: DeclarativeVerificationSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SourcePlatform {
    Macos,
    Windows,
    Linux,
}

impl SourcePlatform {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Linux => "linux",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SourceCategory {
    System,
    Browser,
    Application,
    Development,
    Ai,
    Container,
}

impl SourceCategory {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Browser => "browser",
            Self::Application => "application",
            Self::Development => "development",
            Self::Ai => "ai",
            Self::Container => "container",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SourceRisk {
    Safe,
    Recoverable,
    HighImpact,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeclarativeRootSource {
    pub template: String,
    #[serde(default)]
    pub kind: DeclarativeRootKind,
    #[serde(default)]
    pub child_names: Vec<String>,
    #[serde(default)]
    pub child_prefixes: Vec<String>,
    #[serde(default)]
    pub include_all_children: bool,
    #[serde(default)]
    pub suffixes: Vec<String>,
    #[serde(default)]
    pub verified_rebuildable: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DeclarativeRootKind {
    #[default]
    Static,
    ChildDirectories,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum DeclarativeApplicabilitySource {
    AnyRootExists,
    PathExists {
        template: String,
    },
    ApplicationInstalled {
        identifiers: Vec<String>,
    },
    ExecutableAvailable {
        names: Vec<String>,
    },
    ApplicationVersion {
        identifier: String,
        minimum: Option<String>,
        maximum_exclusive: Option<String>,
    },
    SystemVersion {
        minimum: Option<String>,
        maximum_exclusive: Option<String>,
    },
    FileSystemIn {
        values: Vec<String>,
    },
    CapabilityAvailable {
        values: Vec<String>,
    },
    ProcessRunning {
        values: Vec<String>,
    },
    AnyOf {
        items: Vec<DeclarativeApplicabilitySource>,
    },
    AllOf {
        items: Vec<DeclarativeApplicabilitySource>,
    },
    Not {
        item: Box<DeclarativeApplicabilitySource>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum DeclarativeMatcherSource {
    All,
    NameEquals {
        values: Vec<String>,
    },
    NameGlob {
        values: Vec<String>,
    },
    ExtensionIn {
        values: Vec<String>,
    },
    PathSegmentIn {
        values: Vec<String>,
    },
    OlderThan {
        days: u64,
    },
    LargerThan {
        bytes: u64,
    },
    SmallerThan {
        bytes: u64,
    },
    MaxDepth {
        depth: usize,
    },
    AllOf {
        items: Vec<DeclarativeMatcherSource>,
    },
    AnyOf {
        items: Vec<DeclarativeMatcherSource>,
    },
    Not {
        item: Box<DeclarativeMatcherSource>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum DeclarativeExecutionSource {
    DeleteMatchingContents { requires_app_close: bool },
    DeleteWholeRoot { requires_app_close: bool },
}

impl DeclarativeExecutionSource {
    pub(crate) const fn requires_app_close(self) -> bool {
        match self {
            Self::DeleteMatchingContents { requires_app_close }
            | Self::DeleteWholeRoot { requires_app_close } => requires_app_close,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeclarativeVerificationSource {
    pub lifecycle: SourceLifecycle,
    pub evidence: String,
    pub verified_at: String,
    pub verified_platform: SourcePlatform,
    #[serde(default)]
    pub references: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SourceLifecycle {
    Candidate,
    Verified,
    Stable,
    Deprecated,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootVariable {
    Home,
    Temp,
    LocalAppData,
    RoamingAppData,
    SystemRoot,
    ProgramFiles,
    ProgramData,
    UserLibrary,
    ApplicationSupport,
    DarwinUserCache,
}

impl RootVariable {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Temp => "temp",
            Self::LocalAppData => "local_app_data",
            Self::RoamingAppData => "roaming_app_data",
            Self::SystemRoot => "system_root",
            Self::ProgramFiles => "program_files",
            Self::ProgramData => "program_data",
            Self::UserLibrary => "user_library",
            Self::ApplicationSupport => "application_support",
            Self::DarwinUserCache => "darwin_user_cache",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootTemplateParts {
    pub variable: RootVariable,
    pub suffix: Vec<String>,
}

/// Build time and runtime share the same parser and static validation so an
/// installed application cannot interpret rules differently from the build.
/// Runtime repeats validation to guard against corrupted generated resources
/// and future signed rule packs.
pub(crate) fn parse_catalog(
    sources: &[(&str, &str)],
) -> Result<Vec<ParsedDeclarativeRule>, String> {
    let mut parsed = Vec::with_capacity(sources.len());
    for (source_name, content) in sources {
        let rule = toml::from_str::<DeclarativeRuleSource>(content)
            .map_err(|error| format!("declarative rule {source_name} has invalid TOML: {error}"))?;
        validate_rule(source_name, &rule)?;
        parsed.push(ParsedDeclarativeRule {
            source_name: (*source_name).to_string(),
            rule,
        });
    }
    validate_catalog(&parsed)?;
    Ok(parsed)
}

fn validate_rule(source_name: &str, rule: &DeclarativeRuleSource) -> Result<(), String> {
    if rule.schema_version != RULE_SCHEMA_VERSION {
        return Err(format!(
            "declarative rule {source_name} must use schema version {RULE_SCHEMA_VERSION}"
        ));
    }
    if rule.rule_version == 0 {
        return Err(format!(
            "declarative rule {source_name} must have a positive version"
        ));
    }
    if !valid_rule_id(&rule.id) {
        return Err(format!("declarative rule {source_name} has an invalid ID"));
    }
    let expected_file_name = format!("{}.toml", rule.id);
    let source_parts = source_name.split('/').collect::<Vec<_>>();
    if source_parts.as_slice()
        != [
            rule.platform.as_str(),
            rule.category.as_str(),
            expected_file_name.as_str(),
        ]
    {
        return Err(format!(
            "declarative rules must use platform/category/ID.toml paths: {source_name} -> {}/{}/{}",
            rule.platform.as_str(),
            rule.category.as_str(),
            expected_file_name
        ));
    }
    if rule.default_selected && !matches!(rule.risk, SourceRisk::Safe) {
        return Err(format!(
            "declarative rule {} has an unsafe default risk",
            rule.id
        ));
    }
    let recommended_selected = rule.recommended_selected.unwrap_or(rule.default_selected);
    if rule.default_selected && !recommended_selected {
        return Err(format!(
            "declarative rule {} cannot disable the interactive recommendation for an automatic rule",
            rule.id
        ));
    }
    if recommended_selected
        && matches!(rule.risk, SourceRisk::Recoverable)
        && rule.roots.iter().any(|root| !root.verified_rebuildable)
    {
        return Err(format!(
            "declarative rule {} must verify every rebuildable root before recommending recoverable cleanup",
            rule.id
        ));
    }
    // Candidate rules are development-only and Disabled rules have a known
    // issue. Production resources reject both at the source instead of relying
    // on default_selected=false, because callers could still expose or select
    // them.
    if matches!(
        rule.verification.lifecycle,
        SourceLifecycle::Candidate | SourceLifecycle::Disabled
    ) {
        return Err(format!(
            "declarative rule {} has a lifecycle that is not allowed in production",
            rule.id
        ));
    }
    if rule.verification.verified_platform != rule.platform {
        return Err(format!(
            "declarative rule {} has a mismatched verification platform",
            rule.id
        ));
    }
    validate_text("verification evidence", &rule.verification.evidence, false)?;
    validate_verification_references(&rule.id, &rule.verification.references)?;
    if !valid_date(&rule.verification.verified_at) {
        return Err(format!(
            "declarative rule {} has an invalid verification date",
            rule.id
        ));
    }
    if rule.roots.is_empty() || rule.roots.len() > MAX_ROOTS_PER_RULE {
        return Err(format!(
            "declarative rule {} must have 1-{MAX_ROOTS_PER_RULE} roots",
            rule.id
        ));
    }
    if rule.applicability.is_empty() || rule.applicability.len() > MAX_MATCHER_VALUES {
        return Err(format!(
            "declarative rule {} must have 1-{MAX_MATCHER_VALUES} applicability probes",
            rule.id
        ));
    }
    for probe in &rule.applicability {
        validate_applicability(&rule.id, rule.platform, probe, 0)?;
    }
    let mut root_templates = HashSet::new();
    for root in &rule.roots {
        let parts = parse_root_template(&root.template)?;
        if !root_variable_allowed_for_platform(parts.variable, rule.platform) {
            return Err(format!(
                "declarative rule {} uses path variable ${{{}}}, which is not available on {}",
                rule.id,
                parts.variable.as_str(),
                rule.platform.as_str()
            ));
        }
        if !root_templates.insert(root.template.to_ascii_lowercase()) {
            return Err(format!(
                "declarative rule {} contains a duplicate root",
                rule.id
            ));
        }
        validate_root_expansion(&rule.id, root, &parts)?;
        validate_protected_root_policy(&rule.id, &parts)?;
        validate_personal_root_policy(rule, &parts)?;
        if matcher_can_match_entire_root(&rule.matcher) {
            let final_roots = final_root_parts(root, &parts)?;
            // Some package stores and build outputs do not use a conventional
            // cache directory name. They remain declarative, but the rule must
            // explicitly document that boundary and may never be selected by
            // default. This keeps unusual rebuildable roots extensible without
            // weakening the automatic-cleanup policy.
            let exceptional_root_allowed = root.verified_rebuildable
                && !rule.default_selected
                && final_roots.iter().all(verified_rebuildable_root_allowed);
            if !exceptional_root_allowed
                && final_roots
                    .iter()
                    .any(|parts| !all_matcher_root_allowed(parts))
            {
                return Err(format!(
                    "declarative rule {} uses a broad matcher outside an explicit cache root: {}",
                    rule.id, root.template
                ));
            }
        }
    }
    validate_matcher(&rule.id, &rule.matcher, 0)?;
    validate_execution_policy(rule)?;
    validate_process_policy(rule)?;
    Ok(())
}

fn validate_execution_policy(rule: &DeclarativeRuleSource) -> Result<(), String> {
    if !matches!(
        rule.execution,
        DeclarativeExecutionSource::DeleteWholeRoot { .. }
    ) {
        return Ok(());
    }

    // Whole-root staging deliberately trades per-file identity checks for one
    // protected root identity. Keep the authoring contract narrow: the user
    // must opt in to the rule, every byte must match, and every static root
    // must be independently documented as disposable and rebuildable.
    if rule.default_selected
        || !matches!(rule.matcher, DeclarativeMatcherSource::All)
        || rule
            .roots
            .iter()
            .any(|root| root.kind != DeclarativeRootKind::Static || !root.verified_rebuildable)
    {
        return Err(format!(
            "declarative rule {} may delete whole roots only for non-default, rebuildable static roots with an all matcher",
            rule.id
        ));
    }
    for root in &rule.roots {
        let parts = parse_root_template(&root.template)?;
        if !verified_rebuildable_root_allowed(&parts) {
            return Err(format!(
                "declarative rule {} may not delete the whole root: {}",
                rule.id, root.template
            ));
        }
    }
    Ok(())
}

fn verified_rebuildable_root_allowed(parts: &RootTemplateParts) -> bool {
    !parts.suffix.is_empty()
        && !contains_protected_content_segment(&parts.suffix)
        && match parts.variable {
            RootVariable::Home => !is_personal_home_directory(&parts.suffix[0]),
            RootVariable::UserLibrary
            | RootVariable::LocalAppData
            | RootVariable::RoamingAppData
            | RootVariable::ApplicationSupport => true,
            RootVariable::Temp
            | RootVariable::SystemRoot
            | RootVariable::ProgramFiles
            | RootVariable::ProgramData
            | RootVariable::DarwinUserCache => false,
        }
}

fn validate_root_expansion(
    rule_id: &str,
    root: &DeclarativeRootSource,
    _parts: &RootTemplateParts,
) -> Result<(), String> {
    match root.kind {
        DeclarativeRootKind::Static => {
            if root.include_all_children
                || !root.child_names.is_empty()
                || !root.child_prefixes.is_empty()
                || !root.suffixes.is_empty()
            {
                return Err(format!(
                    "Declarative rule {rule_id} gives expansion fields to a static root"
                ));
            }
        }
        DeclarativeRootKind::ChildDirectories => {
            if root.include_all_children
                && (!root.child_names.is_empty() || !root.child_prefixes.is_empty())
            {
                return Err(format!(
                    "Declarative rule {rule_id} combines all-child expansion with redundant filters"
                ));
            }
            if root.suffixes.is_empty()
                || (!root.include_all_children
                    && root.child_names.is_empty()
                    && root.child_prefixes.is_empty())
            {
                return Err(format!(
                    "Declarative rule {rule_id} has an incomplete child-directory root"
                ));
            }
            if root.suffixes.len() > MAX_MATCHER_VALUES
                || root.child_names.len() > MAX_MATCHER_VALUES
                || root.child_prefixes.len() > MAX_MATCHER_VALUES
            {
                return Err(format!(
                    "Declarative rule {rule_id} exceeds root expansion limits"
                ));
            }
            validate_path_names(rule_id, "child name", &root.child_names, false, false)?;
            validate_path_names(rule_id, "child prefix", &root.child_prefixes, false, true)?;
            validate_path_names(rule_id, "root suffix", &root.suffixes, true, false)?;
        }
    }
    Ok(())
}

fn validate_path_names(
    rule_id: &str,
    field: &str,
    values: &[String],
    allow_relative_path: bool,
    allow_trailing_space: bool,
) -> Result<(), String> {
    let mut unique = HashSet::new();
    for value in values {
        validate_text(field, value, false)?;
        if value.trim_start() != value
            || (!allow_trailing_space && value.trim_end() != value)
            || value.starts_with('/')
            || value.starts_with('\\')
            || value.ends_with('/')
            || value.ends_with('\\')
            || value
                .split(['/', '\\'])
                .any(|part| part.is_empty() || matches!(part, "." | "..") || part.contains(':'))
            || (!allow_relative_path && (value.contains('/') || value.contains('\\')))
        {
            return Err(format!(
                "Declarative rule {rule_id} contains an invalid {field}: {value}"
            ));
        }
        if !unique.insert(value.to_ascii_lowercase()) {
            return Err(format!(
                "Declarative rule {rule_id} contains a duplicate {field}: {value}"
            ));
        }
    }
    Ok(())
}

fn final_root_parts(
    root: &DeclarativeRootSource,
    base: &RootTemplateParts,
) -> Result<Vec<RootTemplateParts>, String> {
    match root.kind {
        DeclarativeRootKind::Static => Ok(vec![base.clone()]),
        DeclarativeRootKind::ChildDirectories => root
            .suffixes
            .iter()
            .map(|suffix| {
                let mut parts = base.clone();
                // Preserve the structural depth contributed by the selected
                // direct child without trusting its runtime name as a cache
                // boundary. Safety still depends on the fixed suffix authored
                // in the rule.
                parts.suffix.push("<selected-child>".to_string());
                parts
                    .suffix
                    .extend(suffix.split(['/', '\\']).map(str::to_string));
                Ok(parts)
            })
            .collect(),
    }
}

fn validate_personal_root_policy(
    rule: &DeclarativeRuleSource,
    parts: &RootTemplateParts,
) -> Result<(), String> {
    if parts.variable != RootVariable::Home || parts.suffix.as_slice() != ["Downloads"] {
        return Ok(());
    }

    // Downloads contains personal data. Only stale incomplete downloads may
    // enter it under a strict matcher. Build validation prevents rule drift,
    // while the executor repeats the check against canonical paths and the
    // compiled matcher to guard future rule packs and filesystem changes.
    let allowed_extensions = ["crdownload", "download", "partial", "part"];
    let DeclarativeMatcherSource::AllOf { items } = &rule.matcher else {
        return Err(format!(
            "declarative rule {} cannot use Downloads as a general cleanup root",
            rule.id
        ));
    };
    let has_age_gate = items
        .iter()
        .any(|item| matches!(item, DeclarativeMatcherSource::OlderThan { days } if *days >= 7));
    let has_strict_extension_gate = items.iter().any(|item| {
        let DeclarativeMatcherSource::ExtensionIn { values } = item else {
            return false;
        };
        !values.is_empty()
            && values.iter().all(|value| {
                allowed_extensions
                    .iter()
                    .any(|allowed| value.trim_start_matches('.').eq_ignore_ascii_case(allowed))
            })
    });
    let has_depth_gate = items
        .iter()
        .any(|item| matches!(item, DeclarativeMatcherSource::MaxDepth { depth } if *depth <= 3));
    if rule.id != "system.stale-partial-downloads"
        || !matches!(rule.risk, SourceRisk::Recoverable)
        || rule.default_selected
        || !has_age_gate
        || !has_strict_extension_gate
        || !has_depth_gate
    {
        return Err(format!(
            "declarative rule {} does not meet the Downloads safety policy",
            rule.id
        ));
    }
    Ok(())
}

fn validate_protected_root_policy(rule_id: &str, parts: &RootTemplateParts) -> Result<(), String> {
    if parts
        .suffix
        .iter()
        .any(|part| is_protected_repository_component(part))
    {
        return Err(format!(
            "declarative rule {rule_id} cannot own repository metadata"
        ));
    }

    match parts.variable {
        RootVariable::Home => {
            let Some(first) = parts.suffix.first() else {
                return Err(format!(
                    "declarative rule {rule_id} cannot own the user home directory"
                ));
            };
            if is_protected_home_relative_path(&parts.suffix)
                || (is_personal_home_directory(first) && !first.eq_ignore_ascii_case("Downloads"))
            {
                return Err(format!(
                    "declarative rule {rule_id} cannot own protected user content"
                ));
            }
        }
        RootVariable::UserLibrary => {
            if parts
                .suffix
                .first()
                .is_some_and(|first| is_protected_library_root(first))
            {
                return Err(format!(
                    "declarative rule {rule_id} cannot own protected Library content"
                ));
            }
        }
        RootVariable::Temp
        | RootVariable::LocalAppData
        | RootVariable::RoamingAppData
        | RootVariable::ApplicationSupport
        | RootVariable::SystemRoot
        | RootVariable::ProgramFiles
        | RootVariable::ProgramData
        | RootVariable::DarwinUserCache => {}
    }
    Ok(())
}

fn validate_applicability(
    rule_id: &str,
    platform: SourcePlatform,
    probe: &DeclarativeApplicabilitySource,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_MATCHER_DEPTH {
        return Err(format!(
            "declarative rule {rule_id} has applicability probes nested too deeply"
        ));
    }
    match probe {
        DeclarativeApplicabilitySource::AnyRootExists => Ok(()),
        DeclarativeApplicabilitySource::PathExists { template } => {
            let parts = parse_root_template(template)?;
            if root_variable_allowed_for_platform(parts.variable, platform) {
                Ok(())
            } else {
                Err(format!(
                    "declarative rule {rule_id} uses an applicability path unavailable on this platform"
                ))
            }
        }
        DeclarativeApplicabilitySource::ApplicationInstalled { identifiers } => {
            validate_probe_values(rule_id, "application identifier", identifiers)
        }
        DeclarativeApplicabilitySource::ExecutableAvailable { names } => {
            validate_probe_values(rule_id, "executable name", names)?;
            if names
                .iter()
                .any(|value| value.contains('/') || value.contains('\\'))
            {
                return Err(format!(
                    "declarative rule {rule_id} tool probes must use executable names"
                ));
            }
            Ok(())
        }
        DeclarativeApplicabilitySource::ApplicationVersion {
            identifier,
            minimum,
            maximum_exclusive,
        } => {
            validate_probe_values(
                rule_id,
                "application identifier",
                std::slice::from_ref(identifier),
            )?;
            validate_version_range(rule_id, minimum.as_deref(), maximum_exclusive.as_deref())
        }
        DeclarativeApplicabilitySource::SystemVersion {
            minimum,
            maximum_exclusive,
        } => validate_version_range(rule_id, minimum.as_deref(), maximum_exclusive.as_deref()),
        DeclarativeApplicabilitySource::ProcessRunning { values } => {
            validate_probe_values(rule_id, "process name", values)?;
            if values
                .iter()
                .any(|value| value.contains('/') || value.contains('\\'))
            {
                return Err(format!(
                    "declarative rule {rule_id} process probes must use process names"
                ));
            }
            Ok(())
        }
        DeclarativeApplicabilitySource::FileSystemIn { values }
        | DeclarativeApplicabilitySource::CapabilityAvailable { values } => {
            validate_probe_values(rule_id, "probe value", values)
        }
        DeclarativeApplicabilitySource::AnyOf { items }
        | DeclarativeApplicabilitySource::AllOf { items }
            if !items.is_empty() && items.len() <= MAX_MATCHER_VALUES =>
        {
            for item in items {
                validate_applicability(rule_id, platform, item, depth + 1)?;
            }
            Ok(())
        }
        DeclarativeApplicabilitySource::Not { item } => {
            validate_applicability(rule_id, platform, item, depth + 1)
        }
        _ => Err(format!(
            "declarative rule {rule_id} has invalid applicability probe parameters"
        )),
    }
}

fn validate_probe_values(rule_id: &str, name: &str, values: &[String]) -> Result<(), String> {
    if values.is_empty() || values.len() > MAX_MATCHER_VALUES {
        return Err(format!(
            "declarative rule {rule_id} has an invalid number of {name} values"
        ));
    }
    let mut unique = HashSet::new();
    for value in values {
        validate_text(name, value, false)?;
        if value.trim() != value || value.chars().any(char::is_control) {
            return Err(format!("declarative rule {rule_id} has an invalid {name}"));
        }
        if !unique.insert(value.to_ascii_lowercase()) {
            return Err(format!(
                "declarative rule {rule_id} contains a duplicate {name}"
            ));
        }
    }
    Ok(())
}

fn validate_version_range(
    rule_id: &str,
    minimum: Option<&str>,
    maximum_exclusive: Option<&str>,
) -> Result<(), String> {
    if minimum.is_none() && maximum_exclusive.is_none() {
        return Err(format!(
            "declarative rule {rule_id} version probe has no range"
        ));
    }
    for version in [minimum, maximum_exclusive].into_iter().flatten() {
        validate_text("version", version, false)?;
        if !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
            || !version.bytes().any(|byte| byte.is_ascii_digit())
        {
            return Err(format!("declarative rule {rule_id} has an invalid version"));
        }
    }
    Ok(())
}

fn validate_catalog(rules: &[ParsedDeclarativeRule]) -> Result<(), String> {
    let mut ids = HashMap::new();
    for parsed in rules {
        let rule = &parsed.rule;
        if let Some(existing) = ids.insert((rule.platform, rule.id.as_str()), &parsed.source_name) {
            return Err(format!(
                "duplicate declarative rule ID on the same platform: {} ({} / {})",
                rule.id, existing, parsed.source_name
            ));
        }
    }
    Ok(())
}

pub(crate) fn parse_root_template(template: &str) -> Result<RootTemplateParts, String> {
    if template.is_empty()
        || template.len() > MAX_ROOT_TEMPLATE_BYTES
        || template.trim() != template
        || template.contains('\\')
        || template.contains('\0')
    {
        return Err(format!(
            "root templates must use `/` separators: {template}"
        ));
    }
    let variable_end = template
        .strip_prefix("${")
        .and_then(|value| value.find('}').map(|index| index + 3))
        .ok_or_else(|| {
            format!("root templates must start with a controlled variable: {template}")
        })?;
    let variable_name = &template[2..variable_end - 1];
    let variable = match variable_name {
        "home" => RootVariable::Home,
        "temp" => RootVariable::Temp,
        "local_app_data" => RootVariable::LocalAppData,
        "roaming_app_data" => RootVariable::RoamingAppData,
        "system_root" => RootVariable::SystemRoot,
        "program_files" => RootVariable::ProgramFiles,
        "program_data" => RootVariable::ProgramData,
        "user_library" => RootVariable::UserLibrary,
        "application_support" => RootVariable::ApplicationSupport,
        "darwin_user_cache" => RootVariable::DarwinUserCache,
        _ => {
            return Err(format!(
                "root template uses an unauthorized variable: {variable_name}"
            ))
        }
    };
    let remainder = &template[variable_end..];
    if remainder.contains("${")
        || remainder.contains("//")
        || (!remainder.is_empty() && !remainder.starts_with('/'))
    {
        return Err(format!(
            "root templates may contain only one leading variable: {template}"
        ));
    }
    let suffix = if remainder.is_empty() {
        Vec::new()
    } else {
        remainder
            .strip_prefix('/')
            .expect("a non-empty template suffix must start with a slash")
            .split('/')
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    if suffix.len() > MAX_ROOT_COMPONENTS
        || remainder.ends_with('/')
        || suffix.iter().any(|part| {
            part.is_empty()
                || part.trim() != part
                || matches!(part.as_str(), "." | "..")
                || part.contains(':')
                || part.contains('\0')
        })
    {
        return Err(format!(
            "root template contains an invalid path segment: {template}"
        ));
    }
    Ok(RootTemplateParts { variable, suffix })
}

const fn root_variable_allowed_for_platform(
    variable: RootVariable,
    platform: SourcePlatform,
) -> bool {
    match platform {
        SourcePlatform::Macos => matches!(
            variable,
            RootVariable::Home
                | RootVariable::Temp
                | RootVariable::SystemRoot
                | RootVariable::UserLibrary
                | RootVariable::ApplicationSupport
                | RootVariable::DarwinUserCache
        ),
        SourcePlatform::Windows => matches!(
            variable,
            RootVariable::Home
                | RootVariable::Temp
                | RootVariable::LocalAppData
                | RootVariable::RoamingAppData
                | RootVariable::SystemRoot
                | RootVariable::ProgramFiles
                | RootVariable::ProgramData
        ),
        SourcePlatform::Linux => matches!(variable, RootVariable::Home | RootVariable::Temp),
    }
}

/// Composite matchers can collapse into a whole-root match through
/// `AnyOf(All, ...)`, `Not(...)`, or `NameGlob("*")`. Apply the same allowlist
/// as explicit `All` so composition cannot bypass deletion boundaries.
fn matcher_can_match_entire_root(matcher: &DeclarativeMatcherSource) -> bool {
    match matcher {
        DeclarativeMatcherSource::All | DeclarativeMatcherSource::Not { .. } => true,
        DeclarativeMatcherSource::NameGlob { values } => values.iter().any(|value| {
            !value.is_empty()
                && value
                    .chars()
                    .all(|character| matches!(character, '*' | '?'))
        }),
        DeclarativeMatcherSource::AnyOf { items } => {
            items.iter().any(matcher_can_match_entire_root)
        }
        DeclarativeMatcherSource::AllOf { items } => {
            items.iter().all(matcher_can_match_entire_root)
        }
        _ => false,
    }
}

fn all_matcher_root_allowed(parts: &RootTemplateParts) -> bool {
    if parts.variable == RootVariable::LocalAppData
        && parts.suffix.as_slice().first().is_some_and(|value| {
            value.eq_ignore_ascii_case("CrashDumps") || value.to_ascii_lowercase().contains("cache")
        })
    {
        return true;
    }
    if parts.variable == RootVariable::ProgramData
        && parts.suffix.len() >= 4
        && parts.suffix[0].eq_ignore_ascii_case("Microsoft")
        && parts.suffix[1].eq_ignore_ascii_case("Windows")
        && parts.suffix[2].eq_ignore_ascii_case("WER")
        && matches!(
            parts.suffix[3].to_ascii_lowercase().as_str(),
            "reportarchive" | "reportqueue" | "temp"
        )
    {
        return true;
    }
    if parts.suffix.is_empty() || contains_protected_content_segment(&parts.suffix) {
        return false;
    }
    match parts.variable {
        // Shared temporary roots always require an age, name, or exclusion gate.
        RootVariable::Temp => false,
        RootVariable::DarwinUserCache => true,
        RootVariable::UserLibrary
        | RootVariable::LocalAppData
        | RootVariable::RoamingAppData
        | RootVariable::ApplicationSupport => contains_rebuildable_boundary(&parts.suffix),
        RootVariable::Home => {
            !is_personal_home_directory(&parts.suffix[0])
                && contains_rebuildable_boundary(&parts.suffix)
        }
        RootVariable::SystemRoot | RootVariable::ProgramFiles | RootVariable::ProgramData => false,
    }
}

fn contains_rebuildable_boundary(parts: &[String]) -> bool {
    parts.iter().any(|part| {
        let normalized = part.to_ascii_lowercase();
        let conventional_name = normalized.trim_start_matches(['.', '_']);
        normalized.contains("cache")
            || matches!(
                conventional_name,
                "crashdumps"
                    | "log"
                    | "logs"
                    | "reportarchive"
                    | "reportqueue"
                    | "reports"
                    | "temp"
                    | "tmp"
            )
    })
}

fn is_personal_home_directory(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "desktop"
            | "documents"
            | "downloads"
            | "movies"
            | "music"
            | "pictures"
            | "public"
            | "videos"
    )
}

fn contains_protected_content_segment(parts: &[String]) -> bool {
    parts.iter().any(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "checkpoints"
                | "accounts"
                | "addressbook"
                | "calendars"
                | "cloudstorage"
                | "cookies"
                | "credentials"
                | "datasets"
                | "keychains"
                | "keys"
                | "mail"
                | "messages"
                | "mobile documents"
                | "models"
                | "notes"
                | "photos"
                | "projects"
                | "reminders"
                | "safari"
                | "sessions"
                | "wallet"
                | "wallets"
        )
    })
}

fn validate_matcher(
    rule_id: &str,
    matcher: &DeclarativeMatcherSource,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_MATCHER_DEPTH {
        return Err(format!(
            "declarative rule {rule_id} has matchers nested too deeply"
        ));
    }
    match matcher {
        DeclarativeMatcherSource::All => Ok(()),
        DeclarativeMatcherSource::NameEquals { values }
        | DeclarativeMatcherSource::ExtensionIn { values }
        | DeclarativeMatcherSource::PathSegmentIn { values } => {
            validate_matcher_values(rule_id, values, false)
        }
        DeclarativeMatcherSource::NameGlob { values } => {
            validate_matcher_values(rule_id, values, true)
        }
        DeclarativeMatcherSource::OlderThan { days } if *days > 0 => Ok(()),
        DeclarativeMatcherSource::LargerThan { bytes }
        | DeclarativeMatcherSource::SmallerThan { bytes }
            if *bytes > 0 =>
        {
            Ok(())
        }
        DeclarativeMatcherSource::MaxDepth { depth } if *depth > 0 => Ok(()),
        DeclarativeMatcherSource::AllOf { items } | DeclarativeMatcherSource::AnyOf { items }
            if !items.is_empty() && items.len() <= MAX_MATCHER_VALUES =>
        {
            for item in items {
                validate_matcher(rule_id, item, depth + 1)?;
            }
            Ok(())
        }
        DeclarativeMatcherSource::Not { item } => validate_matcher(rule_id, item, depth + 1),
        _ => Err(format!(
            "declarative rule {rule_id} has invalid matcher parameters"
        )),
    }
}

fn validate_matcher_values(
    rule_id: &str,
    values: &[String],
    is_name_glob: bool,
) -> Result<(), String> {
    if values.is_empty() || values.len() > MAX_MATCHER_VALUES {
        return Err(format!(
            "declarative rule {rule_id} has an invalid number of matcher values"
        ));
    }
    let mut unique = HashSet::new();
    for value in values {
        validate_text("matcher value", value, false)?;
        if value.trim() != value
            || value.chars().any(char::is_control)
            || value.contains('/')
            || value.contains('\\')
        {
            return Err(format!(
                "declarative rule {rule_id} name matcher cannot contain a path"
            ));
        }
        if is_name_glob && value.contains("**") {
            return Err(format!(
                "declarative rule {rule_id} name glob does not support `**`"
            ));
        }
        if !unique.insert(value.to_ascii_lowercase()) {
            return Err(format!(
                "declarative rule {rule_id} contains a duplicate matcher value"
            ));
        }
    }
    Ok(())
}

fn validate_process_policy(rule: &DeclarativeRuleSource) -> Result<(), String> {
    let requires_close = rule.execution.requires_app_close();
    if requires_close == rule.required_stopped_processes.is_empty() {
        return Err(format!(
            "declarative rule {} has inconsistent app-close behavior and process names",
            rule.id
        ));
    }
    let mut unique = HashSet::new();
    for process in &rule.required_stopped_processes {
        validate_text("process name", process, false)?;
        if process.trim() != process
            || process.chars().any(char::is_control)
            || process.contains('/')
            || process.contains('\\')
            || process.contains(',')
            || process.contains('"')
        {
            return Err(format!(
                "declarative rule {} process names must be individual executable names",
                rule.id
            ));
        }
        if !unique.insert(process.to_ascii_lowercase()) {
            return Err(format!(
                "declarative rule {} contains a duplicate process name",
                rule.id
            ));
        }
    }
    Ok(())
}

fn validate_text(name: &str, value: &str, allow_empty: bool) -> Result<(), String> {
    let value = value.trim();
    if (!allow_empty && value.is_empty()) || value.len() > MAX_TEXT_VALUE_BYTES {
        return Err(format!(
            "{name} is empty or exceeds {MAX_TEXT_VALUE_BYTES} bytes"
        ));
    }
    Ok(())
}

fn validate_verification_references(rule_id: &str, references: &[String]) -> Result<(), String> {
    if references.len() > MAX_VERIFICATION_REFERENCES {
        return Err(format!(
            "declarative rule {rule_id} exceeds the verification reference limit"
        ));
    }
    let mut unique = HashSet::new();
    for reference in references {
        if reference.len() > MAX_REFERENCE_BYTES
            || !reference.starts_with("https://")
            || reference.chars().any(char::is_whitespace)
            || reference.chars().any(char::is_control)
            || !unique.insert(reference.to_ascii_lowercase())
        {
            return Err(format!(
                "declarative rule {rule_id} contains an invalid verification reference"
            ));
        }
    }
    Ok(())
}

fn valid_rule_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_RULE_ID_BYTES
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !(bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit()))
    {
        return false;
    }
    let Ok(year) = value[0..4].parse::<u32>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u32>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u32>() else {
        return false;
    };
    if year < 2020 || !(1..=12).contains(&month) {
        return false;
    }
    let leap_year =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=max_day).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_source(
        id: &str,
        platform: &str,
        root: &str,
        lifecycle: &str,
        matcher: &str,
    ) -> String {
        format!(
            r#"
id = "{id}"
schema_version = 3
rule_version = 1
platform = "{platform}"
category = "system"
risk = "safe"
default_selected = false
required_stopped_processes = []

[[applicability]]
kind = "anyRootExists"

[[roots]]
template = "{root}"

[matcher]
{matcher}

[execution]
kind = "deleteMatchingContents"
requires_app_close = false

[verification]
lifecycle = "{lifecycle}"
evidence = "fixture evidence"
verified_at = "2026-07-17"
verified_platform = "{platform}"
"#
        )
    }

    #[test]
    fn root_templates_reject_parent_segments_and_arbitrary_environment_variables() {
        assert!(parse_root_template("${temp}").is_ok());
        assert!(parse_root_template("${home}/../Library").is_err());
        assert!(parse_root_template("${USERPROFILE}/Cache").is_err());
        assert!(parse_root_template("${home}/${temp}").is_err());
        assert!(parse_root_template("${home}/Library//Caches").is_err());
        assert!(parse_root_template(" ${home}/Library/Caches").is_err());
    }

    #[test]
    fn dynamic_roots_require_direct_child_filters_and_fixed_safe_suffixes() {
        let static_rule = fixture_source(
            "fixture.rule",
            "macos",
            "${user_library}/Application Support/Fixture",
            "verified",
            r#"kind = "all""#,
        );
        let dynamic_root = r#"[[roots]]
template = "${user_library}/Application Support/Fixture"
kind = "childDirectories"
child_names = ["Default"]
child_prefixes = ["Profile "]
suffixes = ["Cache"]"#;
        let valid = static_rule.replace(
            r#"[[roots]]
template = "${user_library}/Application Support/Fixture""#,
            dynamic_root,
        );
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &valid)]).is_ok());

        let missing_suffix = valid.replace("suffixes = [\"Cache\"]", "suffixes = []");
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &missing_suffix)]).is_err());

        let parent_suffix = valid.replace("suffixes = [\"Cache\"]", "suffixes = [\"../Cache\"]");
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &parent_suffix)]).is_err());

        let unrestricted_and_named = valid.replace(
            "child_names = [\"Default\"]",
            "child_names = [\"Default\"]\ninclude_all_children = true",
        );
        assert!(
            parse_catalog(&[("macos/system/fixture.rule.toml", &unrestricted_and_named,)]).is_err()
        );
    }

    #[test]
    fn composite_matchers_limit_nesting_and_empty_sets() {
        let empty = DeclarativeMatcherSource::AnyOf { items: Vec::new() };
        assert!(validate_matcher("fixture.rule", &empty, 0).is_err());

        let mut nested = DeclarativeMatcherSource::All;
        for _ in 0..=MAX_MATCHER_DEPTH {
            nested = DeclarativeMatcherSource::Not {
                item: Box::new(nested),
            };
        }
        assert!(validate_matcher("fixture.rule", &nested, 0).is_err());
    }

    #[test]
    fn broad_composite_matchers_cannot_bypass_root_allowlists() {
        let matcher = DeclarativeMatcherSource::AnyOf {
            items: vec![
                DeclarativeMatcherSource::NameEquals {
                    values: vec!["safe.cache".to_string()],
                },
                DeclarativeMatcherSource::All,
            ],
        };
        assert!(matcher_can_match_entire_root(&matcher));

        let restrictive = DeclarativeMatcherSource::AllOf {
            items: vec![
                DeclarativeMatcherSource::All,
                DeclarativeMatcherSource::ExtensionIn {
                    values: vec!["tmp".to_string()],
                },
            ],
        };
        assert!(!matcher_can_match_entire_root(&restrictive));
    }

    #[test]
    fn whole_root_deletion_requires_an_explicit_rebuildable_boundary() {
        let base = fixture_source(
            "system.fixture-cache",
            "macos",
            "${user_library}/Caches/Fixture",
            "verified",
            r#"kind = "all""#,
        );
        let valid = base
            .replace(
                "template = \"${user_library}/Caches/Fixture\"",
                "template = \"${user_library}/Caches/Fixture\"\nverified_rebuildable = true",
            )
            .replace(
                "kind = \"deleteMatchingContents\"",
                "kind = \"deleteWholeRoot\"",
            );
        assert!(parse_catalog(&[("macos/system/system.fixture-cache.toml", &valid,)]).is_ok());

        let automatic = valid.replace("default_selected = false", "default_selected = true");
        assert!(parse_catalog(&[("macos/system/system.fixture-cache.toml", &automatic,)]).is_err());

        let filtered = valid.replace(
            "[matcher]\nkind = \"all\"",
            "[matcher]\nkind = \"extensionIn\"\nvalues = [\"tmp\"]",
        );
        assert!(parse_catalog(&[("macos/system/system.fixture-cache.toml", &filtered,)]).is_err());

        let unverified = valid.replace("verified_rebuildable = true\n", "");
        assert!(
            parse_catalog(&[("macos/system/system.fixture-cache.toml", &unverified,)]).is_err()
        );
    }

    #[test]
    fn downloads_allows_only_stale_incomplete_download_rules() {
        let valid = fixture_source(
            "system.stale-partial-downloads",
            "macos",
            "${home}/Downloads",
            "verified",
            r#"kind = "allOf"
items = [
  { kind = "olderThan", days = 7 },
  { kind = "extensionIn", values = ["crdownload", "download", "partial", "part"] },
  { kind = "maxDepth", depth = 3 },
]"#,
        )
        .replace(r#"risk = "safe""#, r#"risk = "recoverable""#);
        assert!(
            parse_catalog(&[("macos/system/system.stale-partial-downloads.toml", &valid,)]).is_ok()
        );

        let recent = valid.replace("days = 7", "days = 6");
        assert!(
            parse_catalog(&[("macos/system/system.stale-partial-downloads.toml", &recent,)])
                .is_err()
        );

        let broad_extension = valid.replace(
            r#"["crdownload", "download", "partial", "part"]"#,
            r#"["crdownload", "zip"]"#,
        );
        assert!(parse_catalog(&[(
            "macos/system/system.stale-partial-downloads.toml",
            &broad_extension,
        )])
        .is_err());

        let wrong_rule = valid.replace("system.stale-partial-downloads", "system.download-cleanup");
        assert!(
            parse_catalog(&[("macos/system/system.download-cleanup.toml", &wrong_rule,)]).is_err()
        );
    }

    #[test]
    fn verification_dates_must_be_valid_calendar_dates() {
        assert!(valid_date("2026-07-17"));
        assert!(valid_date("2024-02-29"));
        assert!(!valid_date("2026-02-29"));
        assert!(!valid_date("2026-13-01"));
    }

    #[test]
    fn verification_references_are_optional_but_must_be_https_urls() {
        assert!(validate_verification_references(
            "fixture.rule",
            &["https://github.com/example/project/blob/main/rules/cache.toml".to_string()]
        )
        .is_ok());
        assert!(validate_verification_references(
            "fixture.rule",
            &["http://example.com/rule".to_string()]
        )
        .is_err());
        assert!(validate_verification_references(
            "fixture.rule",
            &[
                "https://example.com/rule".to_string(),
                "https://example.com/RULE".to_string(),
            ]
        )
        .is_err());
    }

    #[test]
    fn complete_toml_rejects_unknown_fields_and_wrong_catalog_paths() {
        let valid = fixture_source(
            "fixture.rule",
            "macos",
            "${user_library}/Caches/Fixture",
            "verified",
            r#"kind = "all""#,
        );
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &valid)]).is_ok());

        let unknown = valid.replace(
            "verified_platform = \"macos\"",
            "verified_platform = \"macos\"\nunknown_field = true",
        );
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &unknown)]).is_err());

        assert!(parse_catalog(&[("windows/system/fixture.rule.toml", &valid)]).is_err());
        assert!(parse_catalog(&[("macos/browser/fixture.rule.toml", &valid)]).is_err());

        let missing_applicability =
            valid.replace("[[applicability]]\nkind = \"anyRootExists\"\n\n", "");
        assert!(
            parse_catalog(&[("macos/system/fixture.rule.toml", &missing_applicability)]).is_err()
        );
    }

    #[test]
    fn applicability_probes_validate_platform_paths_and_version_ranges() {
        let invalid_path = DeclarativeApplicabilitySource::PathExists {
            template: "${local_app_data}/Cache".to_string(),
        };
        assert!(
            validate_applicability("fixture.rule", SourcePlatform::Macos, &invalid_path, 0)
                .is_err()
        );

        let missing_range = DeclarativeApplicabilitySource::SystemVersion {
            minimum: None,
            maximum_exclusive: None,
        };
        assert!(
            validate_applicability("fixture.rule", SourcePlatform::Macos, &missing_range, 0)
                .is_err()
        );
    }

    #[test]
    fn production_catalog_rejects_candidate_rules_and_cross_platform_variables() {
        let candidate = fixture_source(
            "fixture.rule",
            "macos",
            "${user_library}/Caches/Fixture",
            "candidate",
            r#"kind = "all""#,
        );
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &candidate)]).is_err());

        let wrong_variable = fixture_source(
            "fixture.rule",
            "macos",
            "${local_app_data}/Cache/Fixture",
            "verified",
            r#"kind = "all""#,
        );
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &wrong_variable)]).is_err());
    }

    #[test]
    fn missing_evidence_and_app_close_policy_fail_validation() {
        let valid = fixture_source(
            "fixture.rule",
            "macos",
            "${user_library}/Caches/Fixture",
            "stable",
            r#"kind = "all""#,
        );
        let missing_evidence = valid.replace("evidence = \"fixture evidence\"", "evidence = \"\"");
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &missing_evidence)]).is_err());

        let action_conflict =
            valid.replace("requires_app_close = false", "requires_app_close = true");
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &action_conflict)]).is_err());
    }

    #[test]
    fn broad_matchers_fail_while_nested_roots_defer_to_the_scan_plan() {
        let broad = fixture_source(
            "fixture.rule",
            "macos",
            "${home}/Documents",
            "verified",
            r#"kind = "all""#,
        );
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &broad)]).is_err());

        let library_content = fixture_source(
            "fixture.rule",
            "macos",
            "${user_library}/Mail/V10",
            "verified",
            r#"kind = "all""#,
        );
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &library_content)]).is_err());

        let app_data_content = fixture_source(
            "fixture.rule",
            "windows",
            "${local_app_data}/Example/State",
            "verified",
            r#"kind = "all""#,
        );
        assert!(parse_catalog(&[("windows/system/fixture.rule.toml", &app_data_content)]).is_err());

        let explicit_rebuildable = app_data_content
            .replace(
                r#"template = "${local_app_data}/Example/State""#,
                r#"template = "${local_app_data}/Example/State"
verified_rebuildable = true"#,
            )
            .replace(r#"risk = "safe""#, r#"risk = "recoverable""#);
        assert!(
            parse_catalog(&[("windows/system/fixture.rule.toml", &explicit_rebuildable)]).is_ok()
        );

        let recommended_rebuildable = explicit_rebuildable.replace(
            "default_selected = false",
            "default_selected = false\nrecommended_selected = true",
        );
        assert!(
            parse_catalog(&[("windows/system/fixture.rule.toml", &recommended_rebuildable,)])
                .is_ok()
        );

        let unverified_recommendation = app_data_content
            .replace(r#"risk = "safe""#, r#"risk = "recoverable""#)
            .replace(
                "default_selected = false",
                "default_selected = false\nrecommended_selected = true",
            );
        assert!(parse_catalog(&[(
            "windows/system/fixture.rule.toml",
            &unverified_recommendation,
        )])
        .is_err());

        let default_selected_rebuildable =
            explicit_rebuildable.replace("default_selected = false", "default_selected = true");
        assert!(parse_catalog(&[(
            "windows/system/fixture.rule.toml",
            &default_selected_rebuildable,
        )])
        .is_err());

        let shared_temp = fixture_source(
            "fixture.rule",
            "macos",
            "${temp}",
            "verified",
            r#"kind = "all""#,
        );
        assert!(parse_catalog(&[("macos/system/fixture.rule.toml", &shared_temp)]).is_err());

        let parent = fixture_source(
            "fixture.parent",
            "macos",
            "${user_library}/Caches/Fixture",
            "verified",
            r#"kind = "nameEquals"
values = ["one.tmp"]"#,
        );
        let child = fixture_source(
            "fixture.child",
            "macos",
            "${user_library}/Caches/Fixture/Nested",
            "verified",
            r#"kind = "nameEquals"
values = ["two.tmp"]"#,
        );
        assert!(parse_catalog(&[
            ("macos/system/fixture.parent.toml", &parent),
            ("macos/system/fixture.child.toml", &child),
        ])
        .is_ok());

        let alias = fixture_source(
            "fixture.alias",
            "macos",
            "${home}/Library/Caches/Fixture/Nested",
            "verified",
            r#"kind = "nameEquals"
values = ["three.tmp"]"#,
        );
        assert!(parse_catalog(&[
            ("macos/system/fixture.parent.toml", &parent),
            ("macos/system/fixture.alias.toml", &alias),
        ])
        .is_ok());
    }

    #[test]
    fn protected_roots_reject_narrow_matchers_and_rebuildable_overrides() {
        for (template, platform) in [
            ("${home}/.ssh/cache", "macos"),
            ("${home}/.aws/cache", "windows"),
            ("${home}/.config/tool/cache", "macos"),
            ("${home}/.local/share/tool/cache", "macos"),
            ("${home}/OneDrive/cache", "windows"),
            ("${home}/OneDrive - Example Organization/cache", "windows"),
            ("${home}/projects/generated", "macos"),
            ("${home}/repo/.git/objects", "macos"),
            ("${home}/Library/CloudStorage/Provider/cache", "macos"),
            ("${user_library}/Mobile Documents/CloudDocs/cache", "macos"),
        ] {
            let source = fixture_source(
                "fixture.rule",
                platform,
                template,
                "verified",
                r#"kind = "extensionIn"
values = ["tmp"]"#,
            )
            .replace(
                &format!(r#"template = "{template}""#),
                &format!(
                    r#"template = "{template}"
verified_rebuildable = true"#
                ),
            )
            .replace(r#"risk = "safe""#, r#"risk = "recoverable""#);
            let source_name = format!("{platform}/system/fixture.rule.toml");
            assert!(
                parse_catalog(&[(&source_name, &source)]).is_err(),
                "{template} must remain protected regardless of matcher width"
            );
        }
    }

    #[test]
    fn protected_roots_allow_only_verified_nuget_http_cache_roots() {
        for template in [
            "${home}/.local/share/NuGet/http-cache",
            "${home}/.local/share/NuGet/v3-cache",
        ] {
            let source = fixture_source(
                "fixture.nuget-cache",
                "macos",
                template,
                "verified",
                r#"kind = "all""#,
            )
            .replace(
                &format!(r#"template = "{template}""#),
                &format!(
                    r#"template = "{template}"
verified_rebuildable = true"#
                ),
            )
            .replace(r#"risk = "safe""#, r#"risk = "recoverable""#);
            assert!(
                parse_catalog(&[("macos/system/fixture.nuget-cache.toml", &source)]).is_ok(),
                "{template} must remain eligible for an opt-in verified rule"
            );
        }
    }
}
