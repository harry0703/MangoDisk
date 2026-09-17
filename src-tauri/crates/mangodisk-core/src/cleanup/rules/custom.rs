use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

use mangodisk_platform::{current_platform, Platform};

use super::{
    models::{
        CompiledRule, ExecutionSpec, PlatformConstraint, RuleLifecycle, VerificationMetadata,
        RULE_SCHEMA_VERSION,
    },
    ApplicabilityProbe, MatcherSpec, RuleRiskLevel,
};
use crate::cleanup::{
    rule_execution::validate_custom_rule_root, CleanupCategory, CustomCleanupModifiedTime,
    CustomCleanupRule, CUSTOM_CLEANUP_RULE_SCHEMA_VERSION,
};

const MAX_CUSTOM_RULES: usize = 20;
const MAX_ROOTS_PER_RULE: usize = 8;
const MAX_PATTERNS_PER_RULE: usize = 16;
const MAX_TEXT_LENGTH: usize = 80;
const MAX_FILTER_DAYS: u64 = 3_650;

/// Saved directories can disappear between visits. Only scan preparation may
/// omit them; execution retains the effective scope published by that scan.
/// Keep the original preferences separate so reconnecting a disk needs no edits.
pub(crate) fn prepare_custom_scan_rules(
    definitions: &[CustomCleanupRule],
) -> Result<(Vec<CustomCleanupRule>, u64), String> {
    if definitions.len() > MAX_CUSTOM_RULES {
        return Err("too many custom cleanup rules".to_string());
    }
    let mut ids = HashSet::new();
    let mut available = Vec::new();
    let mut missing_roots = HashSet::new();
    for (rule_index, definition) in definitions.iter().enumerate() {
        validate_custom_rule(definition, &mut ids)?;
        let mut rule = definition.clone();
        rule.roots.clear();
        for (root_index, raw_root) in definition.roots.iter().enumerate() {
            let root = Path::new(raw_root.trim());
            let exists = custom_scan_root_exists(root).map_err(|error| {
                log::warn!(
                    "custom_cleanup_root_rejected rule_index={rule_index} root_index={root_index} reason=invalidRoot diagnostic={error}"
                );
                "a custom cleanup directory could not be validated".to_string()
            })?;
            if exists {
                rule.roots.push(raw_root.clone());
            } else {
                // Several rules may reference the same directory. Count folders,
                // not rule references, using the platform's path identity policy.
                missing_roots.insert(current_platform().path_identity_key(root));
                log::info!("custom_cleanup_root_skipped rule_index={rule_index} root_index={root_index} reason=notFound");
            }
        }
        if !rule.roots.is_empty() {
            available.push(rule);
        }
    }
    Ok((available, missing_roots.len() as u64))
}

fn custom_scan_root_exists(root: &Path) -> Result<bool, String> {
    if !root.is_absolute()
        || root
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err("custom cleanup directories must be absolute normalized paths".to_string());
    }
    // Inspect ancestors from the volume down so a missing child never conceals
    // a link, reparse point, non-directory or access failure in its parent chain.
    for ancestor in root.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if current_platform().is_link_like(&metadata)
                    && !current_platform().is_allowed_system_path_alias(ancestor)
                {
                    return Err("custom cleanup directories cannot contain links".to_string());
                }
                if !metadata.is_dir() && !current_platform().is_allowed_system_path_alias(ancestor)
                {
                    return Err("custom cleanup roots must be directories".to_string());
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "custom cleanup directory metadata failed: {:?}",
                    error.kind()
                ))
            }
        }
    }
    Ok(true)
}

pub(crate) fn compile_custom_rules(
    definitions: &[CustomCleanupRule],
) -> Result<Vec<CompiledRule>, String> {
    if definitions.len() > MAX_CUSTOM_RULES {
        return Err("too many custom cleanup rules".to_string());
    }

    let mut ids = HashSet::new();
    definitions
        .iter()
        .map(|definition| compile_custom_rule(definition, &mut ids))
        .collect()
}

fn compile_custom_rule(
    definition: &CustomCleanupRule,
    ids: &mut HashSet<String>,
) -> Result<CompiledRule, String> {
    validate_custom_rule(definition, ids)?;
    let matcher = custom_matcher(definition)?;
    let mut roots = Vec::<PathBuf>::new();
    for raw_root in &definition.roots {
        let canonical = validate_custom_rule_root(PathBuf::from(raw_root.trim()).as_path())?;
        if roots
            .iter()
            .any(|root| current_platform().path_is_same_or_child(&canonical, root))
        {
            continue;
        }
        // A newly selected parent covers descendants that were added earlier.
        // Collapse them in Core as well as the UI because adapter input is not a
        // trusted scan boundary and duplicate activations add hot-path work.
        roots.retain(|root| !current_platform().path_is_same_or_child(root, &canonical));
        roots.push(canonical);
    }
    if roots.is_empty() {
        return Err("a custom cleanup rule requires an available directory".to_string());
    }

    let platform = {
        #[cfg(target_os = "macos")]
        {
            PlatformConstraint::Macos
        }
        #[cfg(windows)]
        {
            PlatformConstraint::Windows
        }
        #[cfg(target_os = "linux")]
        {
            PlatformConstraint::Linux
        }
    };
    Ok(CompiledRule {
        id: custom_rule_id(&definition.id),
        schema_version: RULE_SCHEMA_VERSION,
        rule_version: 1,
        platform,
        category: CleanupCategory::Custom,
        risk: RuleRiskLevel::Recoverable,
        default_selected: false,
        recommended_selected: false,
        applicability: vec![ApplicabilityProbe::AnyRootExists],
        roots,
        matcher,
        execution: ExecutionSpec::DeleteMatchingContents {
            requires_app_close: false,
        },
        remove_empty_directories: definition.remove_empty_directories,
        required_stopped_processes: Vec::new(),
        verification: VerificationMetadata {
            lifecycle: RuleLifecycle::Verified,
            evidence: "user-authored local cleanup rule".to_string(),
            verified_at: "runtime".to_string(),
            verified_platform: platform,
        },
    })
}

fn validate_custom_rule(
    definition: &CustomCleanupRule,
    ids: &mut HashSet<String>,
) -> Result<(), String> {
    if definition.schema_version != CUSTOM_CLEANUP_RULE_SCHEMA_VERSION {
        return Err("unsupported custom cleanup rule schema version".to_string());
    }
    if definition.id.is_empty()
        || definition.id.len() > 64
        || !definition
            .id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || value == b'-')
    {
        return Err("a custom cleanup rule has an invalid identifier".to_string());
    }
    if !ids.insert(definition.id.clone()) {
        return Err("custom cleanup rule identifiers must be unique".to_string());
    }
    let name = definition.name.trim();
    if name.is_empty() || name.chars().count() > MAX_TEXT_LENGTH {
        return Err("a custom cleanup rule has an invalid name".to_string());
    }
    if definition.roots.is_empty() || definition.roots.len() > MAX_ROOTS_PER_RULE {
        return Err("a custom cleanup rule has an invalid directory count".to_string());
    }
    if definition.name_patterns.is_empty() || definition.name_patterns.len() > MAX_PATTERNS_PER_RULE
    {
        return Err("a custom cleanup rule has an invalid filename pattern count".to_string());
    }
    for pattern in &definition.name_patterns {
        let pattern = pattern.trim();
        if pattern.is_empty()
            || pattern.chars().count() > MAX_TEXT_LENGTH
            || pattern.contains('/')
            || pattern.contains('\\')
            || pattern.contains("**")
        {
            return Err("a custom cleanup filename pattern is invalid".to_string());
        }
    }
    if definition
        .minimum_bytes
        .zip(definition.maximum_bytes)
        .is_some_and(|(minimum, maximum)| minimum > maximum)
    {
        return Err("custom cleanup size limits are inconsistent".to_string());
    }
    match definition.modified_time {
        CustomCleanupModifiedTime::Any => {}
        CustomCleanupModifiedTime::OlderThan { days }
        | CustomCleanupModifiedTime::NewerThan { days }
            if days == 0 || days > MAX_FILTER_DAYS =>
        {
            return Err("a custom cleanup modification age is invalid".to_string());
        }
        CustomCleanupModifiedTime::OlderThan { .. }
        | CustomCleanupModifiedTime::NewerThan { .. } => {}
    }
    Ok(())
}

fn custom_matcher(definition: &CustomCleanupRule) -> Result<MatcherSpec, String> {
    let mut matchers = vec![
        MatcherSpec::FileOnly,
        MatcherSpec::NameGlob(
            definition
                .name_patterns
                .iter()
                .map(|pattern| pattern.trim().to_string())
                .collect(),
        ),
    ];
    if let Some(bytes) = definition.minimum_bytes.filter(|bytes| *bytes > 0) {
        matchers.push(MatcherSpec::LargerThanBytes(bytes.saturating_sub(1)));
    }
    if let Some(bytes) = definition.maximum_bytes {
        matchers.push(MatcherSpec::SmallerThanBytes(bytes.saturating_add(1)));
    }
    match definition.modified_time {
        CustomCleanupModifiedTime::Any => {}
        CustomCleanupModifiedTime::OlderThan { days } => {
            matchers.push(MatcherSpec::OlderThanDays(days));
        }
        CustomCleanupModifiedTime::NewerThan { days } => {
            matchers.push(MatcherSpec::NewerThanDays(days));
        }
    }
    if !definition.recursive {
        matchers.push(MatcherSpec::MaxDepth(1));
    }
    Ok(MatcherSpec::AllOf(matchers))
}

pub(crate) fn custom_rule_id(id: &str) -> String {
    format!("custom.{id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::metadata::now_ms;
    use std::fs;

    fn fixture(root: PathBuf) -> CustomCleanupRule {
        CustomCleanupRule {
            schema_version: CUSTOM_CLEANUP_RULE_SCHEMA_VERSION,
            id: "fixture-rule".to_string(),
            name: "Fixture files".to_string(),
            roots: vec![root.to_string_lossy().into_owned()],
            name_patterns: vec!["*.tmp".to_string()],
            minimum_bytes: Some(1),
            maximum_bytes: Some(1024),
            modified_time: CustomCleanupModifiedTime::Any,
            recursive: true,
            remove_empty_directories: false,
        }
    }

    #[test]
    fn custom_scan_preparation_counts_distinct_missing_directories_across_rules() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-count-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).expect("create count fixture");
        let missing = root.join("missing");
        let mut first = fixture(root.clone());
        first.roots.push(missing.to_string_lossy().into_owned());
        let mut second = fixture(missing);
        second.id = "second-rule".to_string();
        second
            .roots
            .push(root.join("other-missing").to_string_lossy().into_owned());

        let (available, count) = prepare_custom_scan_rules(&[first.clone(), second])
            .expect("prepare shared missing directories");
        assert_eq!(count, 2, "count each missing directory only once");
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].roots, vec![first.roots[0].clone()]);
        assert_eq!(first.roots.len(), 2, "preserve the saved rule");
        fs::remove_dir_all(root).expect("remove count fixture");
    }

    #[cfg(windows)]
    #[test]
    fn custom_scan_preparation_deduplicates_windows_path_spellings() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-count-windows-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).expect("create Windows count fixture");
        let first = fixture(root.join("missing"));
        let mut second = first.clone();
        second.id = "second-rule".to_string();
        second.roots = vec![format!(
            "{}/",
            first.roots[0].replace('\\', "/").to_uppercase()
        )];

        let (available, count) =
            prepare_custom_scan_rules(&[first, second]).expect("prepare equivalent Windows paths");
        assert!(available.is_empty());
        assert_eq!(count, 1);
        fs::remove_dir_all(root).expect("remove Windows count fixture");
    }

    #[test]
    fn custom_scan_preparation_rejects_invalid_roots_instead_of_skipping_them() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-safety-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).expect("create safety fixture");
        let file = root.join("file");
        fs::write(&file, b"not a directory").expect("write non-directory fixture");
        for invalid in [
            PathBuf::from("relative/missing"),
            file.clone(),
            file.join("missing"),
            root.join("../missing"),
        ] {
            assert!(prepare_custom_scan_rules(&[fixture(invalid)]).is_err());
        }
        let mut invalid_pattern = fixture(root.join("missing"));
        invalid_pattern.name_patterns = vec!["nested/*.tmp".to_string()];
        assert!(prepare_custom_scan_rules(&[invalid_pattern]).is_err());
        let missing = fixture(root.join("missing"));
        assert!(prepare_custom_scan_rules(&[missing.clone(), missing]).is_err());
        fs::remove_dir_all(root).expect("remove safety fixture");
    }

    #[cfg(unix)]
    #[test]
    fn custom_scan_preparation_rejects_links_and_permission_denial() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-link-safety-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).expect("create link fixture");
        let link = root.join("link");
        symlink(root.join("missing-target"), &link).expect("create a dangling link");
        assert!(prepare_custom_scan_rules(&[fixture(link.clone())]).is_err());
        assert!(prepare_custom_scan_rules(&[fixture(link.join("missing"))]).is_err());
        let denied = root.join("denied");
        fs::create_dir(&denied).expect("create inaccessible fixture");
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o000)).expect("restrict fixture");
        let result = prepare_custom_scan_rules(&[fixture(denied.join("missing"))]);
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o700))
            .expect("restore fixture permissions");
        assert!(
            result.is_err(),
            "access denial must not be reported as a missing folder"
        );
        fs::remove_dir_all(root).expect("remove link fixture");
    }

    #[cfg(windows)]
    #[test]
    fn custom_scan_preparation_rejects_windows_junctions() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-junction-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let target = root.join("target");
        let junction = root.join("junction");
        fs::create_dir_all(&target).expect("create junction target");
        let status = std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .output()
            .expect("create junction fixture");
        assert!(status.status.success(), "junction creation must succeed");
        let direct = prepare_custom_scan_rules(&[fixture(junction.clone())]);
        let nested = prepare_custom_scan_rules(&[fixture(junction.join("missing"))]);
        fs::remove_dir(&junction).expect("remove junction without traversing its target");
        fs::remove_dir_all(root).expect("remove junction fixture");
        assert!(direct.is_err());
        assert!(
            nested.is_err(),
            "missing children must not conceal a reparse point"
        );
    }

    #[test]
    fn custom_rule_compiles_only_regular_files_inside_validated_roots() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-rule-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).expect("create the custom rule fixture root");

        let rules = compile_custom_rules(&[fixture(root.clone())])
            .expect("compile a valid custom cleanup rule");

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "custom.fixture-rule");
        assert_eq!(rules[0].category, CleanupCategory::Custom);
        assert!(!rules[0].default_selected);
        assert!(!rules[0].remove_empty_directories);
        assert!(matches!(rules[0].matcher, MatcherSpec::AllOf(_)));

        fs::remove_dir_all(root).expect("remove the custom rule fixture root");
    }

    #[test]
    fn custom_rule_preserves_the_empty_directory_cleanup_option() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-rule-empty-folders-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).expect("create the custom rule fixture root");
        let mut definition = fixture(root.clone());
        definition.remove_empty_directories = true;

        let rules =
            compile_custom_rules(&[definition]).expect("compile a valid custom cleanup rule");

        assert!(rules[0].remove_empty_directories);
        fs::remove_dir_all(root).expect("remove the custom rule fixture root");
    }

    #[test]
    fn custom_rule_rejects_path_globs_and_duplicate_identifiers() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-rule-invalid-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).expect("create the invalid custom rule fixture root");
        let mut invalid = fixture(root.clone());
        invalid.name_patterns = vec!["nested/*.tmp".to_string()];
        assert!(compile_custom_rules(&[invalid]).is_err());

        let valid = fixture(root.clone());
        assert!(compile_custom_rules(&[valid.clone(), valid]).is_err());
        fs::remove_dir_all(root).expect("remove the invalid custom rule fixture root");
    }

    #[test]
    fn custom_rule_collapses_nested_roots_in_any_input_order() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-rule-roots-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let child = root.join("nested/cache");
        fs::create_dir_all(&child).expect("create the nested custom rule fixture");
        let canonical_root = fs::canonicalize(&root).expect("canonicalize the fixture root");

        for input_roots in [
            vec![root.clone(), child.clone()],
            vec![child.clone(), root.clone()],
        ] {
            let mut definition = fixture(root.clone());
            definition.roots = input_roots
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            let rules = compile_custom_rules(&[definition])
                .expect("compile the custom rule with overlapping roots");

            assert_eq!(rules[0].roots.len(), 1);
            assert!(current_platform().paths_equal(&rules[0].roots[0], &canonical_root));
        }

        fs::remove_dir_all(root).expect("remove the nested custom rule fixture");
    }

    #[test]
    fn custom_rule_keeps_sibling_roots_with_shared_text_prefixes() {
        let parent = std::env::temp_dir().join(format!(
            "mangodisk-custom-rule-siblings-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let first = parent.join("cache");
        let second = parent.join("cache-archive");
        fs::create_dir_all(&first).expect("create the first sibling root");
        fs::create_dir_all(&second).expect("create the second sibling root");
        let mut definition = fixture(first.clone());
        definition.roots = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
        ];

        let rules = compile_custom_rules(&[definition])
            .expect("compile sibling roots with shared text prefixes");

        assert_eq!(rules[0].roots.len(), 2);
        fs::remove_dir_all(parent).expect("remove the sibling root fixture");
    }
}
