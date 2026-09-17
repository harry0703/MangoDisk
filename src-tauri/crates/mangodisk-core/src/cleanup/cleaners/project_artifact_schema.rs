use std::{
    collections::HashSet,
    path::{Component, Path},
};

use serde::Deserialize;

const SUPPORTED_SCHEMA_VERSION: u32 = 1;
const SUPPORTED_PLATFORMS: &[&str] = &["macos", "windows", "linux"];
const SUPPORTED_RISK: &str = "recoverable";
const MAX_DESCENDANT_DEPTH: usize = 64;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectArtifactRuleSource {
    pub(crate) id: String,
    pub(crate) schema_version: u32,
    pub(crate) rule_version: u32,
    pub(crate) platforms: Vec<String>,
    pub(crate) category: String,
    pub(crate) risk: String,
    pub(crate) default_selected: bool,
    #[serde(rename = "match")]
    pub(crate) project_match: ProjectMatchSource,
    pub(crate) artifacts: Vec<ProjectArtifactSource>,
    pub(crate) verification: ProjectVerificationSource,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectMatchSource {
    #[serde(default)]
    pub(crate) file_names_any: Vec<String>,
    #[serde(default)]
    pub(crate) file_suffixes_any: Vec<String>,
    #[serde(default)]
    pub(crate) relative_paths_all: Vec<String>,
    #[serde(default)]
    pub(crate) relative_paths_any: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum ProjectArtifactSource {
    RelativeDirectory { path: String },
    DescendantDirectory { name: String, max_depth: usize },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectVerificationSource {
    pub(crate) lifecycle: String,
    pub(crate) evidence: Vec<String>,
    pub(crate) verified_at: String,
}

pub(crate) fn parse_catalog(
    sources: &[(&str, &str)],
) -> Result<Vec<ProjectArtifactRuleSource>, String> {
    let mut ids = HashSet::new();
    let mut rules = Vec::with_capacity(sources.len());
    for (source_name, source) in sources {
        let rule = toml::from_str::<ProjectArtifactRuleSource>(source)
            .map_err(|error| format!("{source_name}: {error}"))?;
        validate_rule(source_name, &rule)?;
        if !ids.insert(rule.id.clone()) {
            return Err(format!(
                "{source_name}: duplicate project rule id {}",
                rule.id
            ));
        }
        rules.push(rule);
    }
    rules.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(rules)
}

fn validate_rule(source_name: &str, rule: &ProjectArtifactRuleSource) -> Result<(), String> {
    if !valid_rule_id(&rule.id) {
        return Err(format!(
            "{source_name}: project rule id must start with project. and use lowercase ASCII tokens"
        ));
    }
    if rule.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Err(format!(
            "{source_name}: unsupported schema_version {}",
            rule.schema_version
        ));
    }
    if rule.rule_version == 0 {
        return Err(format!("{source_name}: rule_version must be positive"));
    }
    if rule.platforms.is_empty()
        || rule
            .platforms
            .iter()
            .any(|platform| !SUPPORTED_PLATFORMS.contains(&platform.as_str()))
    {
        return Err(format!(
            "{source_name}: platforms must contain only macos or windows"
        ));
    }
    if rule.platforms.iter().collect::<HashSet<_>>().len() != rule.platforms.len() {
        return Err(format!(
            "{source_name}: platforms must not contain duplicates"
        ));
    }
    if rule.category != "development" || rule.risk != SUPPORTED_RISK {
        return Err(format!(
            "{source_name}: project artifacts must use category development and risk recoverable"
        ));
    }
    if rule.default_selected {
        return Err(format!(
            "{source_name}: project artifact rules must remain opt-in"
        ));
    }
    validate_match(source_name, &rule.project_match)?;
    if rule.artifacts.is_empty() {
        return Err(format!(
            "{source_name}: at least one artifact directory is required"
        ));
    }
    let mut artifact_keys = HashSet::new();
    for artifact in &rule.artifacts {
        let key = validate_artifact(source_name, artifact)?;
        if !artifact_keys.insert(key) {
            return Err(format!(
                "{source_name}: duplicate artifact directory definition"
            ));
        }
    }
    if rule.verification.lifecycle != "verified" {
        return Err(format!(
            "{source_name}: verification.lifecycle must be verified"
        ));
    }
    if rule.verification.evidence.is_empty()
        || rule
            .verification
            .evidence
            .iter()
            .any(|value| !value.starts_with("https://"))
    {
        return Err(format!(
            "{source_name}: verification.evidence must contain HTTPS sources"
        ));
    }
    if !valid_iso_date(&rule.verification.verified_at) {
        return Err(format!(
            "{source_name}: verification.verified_at must use YYYY-MM-DD"
        ));
    }
    Ok(())
}

fn validate_match(source_name: &str, project_match: &ProjectMatchSource) -> Result<(), String> {
    if project_match.file_names_any.is_empty() && project_match.file_suffixes_any.is_empty() {
        return Err(format!(
            "{source_name}: at least one marker file name or suffix is required"
        ));
    }
    for name in &project_match.file_names_any {
        validate_single_component(source_name, "marker file name", name)?;
    }
    for suffix in &project_match.file_suffixes_any {
        if !suffix.starts_with('.') || suffix.len() < 2 || suffix.contains(['/', '\\']) {
            return Err(format!(
                "{source_name}: marker suffix must be a single extension-like suffix"
            ));
        }
    }
    for path in project_match
        .relative_paths_all
        .iter()
        .chain(&project_match.relative_paths_any)
    {
        validate_relative_path(source_name, "required project path", path)?;
    }
    Ok(())
}

fn validate_artifact(
    source_name: &str,
    artifact: &ProjectArtifactSource,
) -> Result<String, String> {
    match artifact {
        ProjectArtifactSource::RelativeDirectory { path } => {
            validate_relative_path(source_name, "artifact path", path)?;
            Ok(format!("relative:{path}"))
        }
        ProjectArtifactSource::DescendantDirectory { name, max_depth } => {
            validate_single_component(source_name, "descendant directory name", name)?;
            if *max_depth == 0 || *max_depth > MAX_DESCENDANT_DEPTH {
                return Err(format!(
                    "{source_name}: descendant max_depth must be between 1 and {MAX_DESCENDANT_DEPTH}"
                ));
            }
            Ok(format!("descendant:{name}:{max_depth}"))
        }
    }
}

fn validate_relative_path(source_name: &str, field: &str, value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{source_name}: {field} must be a normalized relative path"
        ));
    }
    Ok(())
}

fn validate_single_component(source_name: &str, field: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\'])
        || Path::new(value).components().count() != 1
    {
        return Err(format!(
            "{source_name}: {field} must be one safe path component"
        ));
    }
    Ok(())
}

fn valid_rule_id(value: &str) -> bool {
    value.strip_prefix("project.").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.split('.').all(|token| {
                !token.is_empty()
                    && !token.starts_with('-')
                    && !token.ends_with('-')
                    && token.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            })
    })
}

fn valid_iso_date(value: &str) -> bool {
    if value.len() != 10
        || value.as_bytes()[4] != b'-'
        || value.as_bytes()[7] != b'-'
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
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
    let leap_year =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    day > 0 && day <= days_in_month
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_RULE: &str = r#"
id = "project.rust-build-artifacts"
schema_version = 1
rule_version = 1
platforms = ["macos", "windows"]
category = "development"
risk = "recoverable"
default_selected = false

[match]
file_names_any = ["Cargo.toml"]

[[artifacts]]
kind = "relativeDirectory"
path = "target"

[verification]
lifecycle = "verified"
evidence = ["https://github.com/tbillington/kondo/blob/master/kondo-lib/src/lib.rs"]
verified_at = "2026-07-18"
"#;

    #[test]
    fn accepts_a_safe_declarative_project_rule() {
        let rules = parse_catalog(&[("rust.toml", VALID_RULE)]).expect("rule must parse");
        assert_eq!(rules[0].id, "project.rust-build-artifacts");
    }

    #[test]
    fn rejects_parent_traversal_in_artifact_paths() {
        let source = VALID_RULE.replace("path = \"target\"", "path = \"../target\"");
        let error = parse_catalog(&[("unsafe.toml", &source)]).expect_err("rule must fail");
        assert!(error.contains("normalized relative path"));
    }

    #[test]
    fn rejects_default_selected_project_cleanup() {
        let source = VALID_RULE.replace("default_selected = false", "default_selected = true");
        let error = parse_catalog(&[("unsafe.toml", &source)]).expect_err("rule must fail");
        assert!(error.contains("must remain opt-in"));
    }

    #[test]
    fn rejects_invalid_rule_ids_and_calendar_dates() {
        let invalid_id = VALID_RULE.replace(
            "project.rust-build-artifacts",
            "project..rust-build-artifacts",
        );
        assert!(parse_catalog(&[("invalid-id.toml", &invalid_id)]).is_err());

        let invalid_date = VALID_RULE.replace("2026-07-18", "2026-02-30");
        assert!(parse_catalog(&[("invalid-date.toml", &invalid_date)]).is_err());
    }
}
