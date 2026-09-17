use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use mangodisk_platform::{current_platform, Platform};

use crate::cleanup::rules::models::{
    CompiledRule, RuleLifecycle, RuleRiskLevel, RuleSpec, RULE_SCHEMA_VERSION,
};

use super::scan_plan::validate_rule_ownership;

pub(crate) fn compile_rules(specs: Vec<RuleSpec>) -> Result<Vec<CompiledRule>, String> {
    let mut ids = HashMap::new();
    let mut compiled = Vec::with_capacity(specs.len());

    for spec in specs {
        validate_spec(&spec, &mut ids)?;
        compiled.push(CompiledRule {
            id: spec.id,
            schema_version: spec.schema_version,
            rule_version: spec.rule_version,
            platform: spec.platform,
            category: spec.category,
            risk: spec.risk,
            default_selected: spec.default_selected,
            recommended_selected: spec.recommended_selected,
            applicability: spec.applicability,
            roots: spec
                .roots
                .into_iter()
                .map(|root| root.resolved_path)
                .collect(),
            matcher: spec.matcher,
            execution: spec.execution,
            remove_empty_directories: false,
            required_stopped_processes: spec.required_stopped_processes,
            verification: spec.verification,
        });
    }
    // The scan plan merges nested roots. Identical roots with identical
    // priority must still fail at registry compilation so results never depend
    // on registration order.
    validate_rule_ownership(&compiled)?;
    Ok(compiled)
}

fn validate_spec(spec: &RuleSpec, ids: &mut HashMap<String, String>) -> Result<(), String> {
    if spec.schema_version != RULE_SCHEMA_VERSION {
        return Err(format!(
            "cleanup rule {} uses an unsupported schema version",
            spec.id
        ));
    }
    if spec.rule_version == 0 {
        return Err(format!(
            "cleanup rule {} must have a positive version",
            spec.id
        ));
    }
    if !valid_rule_id(&spec.id) {
        return Err(format!("cleanup rule {} has an invalid ID", spec.id));
    }
    if ids.insert(spec.id.clone(), spec.id.clone()).is_some() {
        return Err(format!("Duplicate cleanup rule ID: {}", spec.id));
    }
    if spec.default_selected && !matches!(spec.risk, RuleRiskLevel::Safe) {
        return Err(format!(
            "cleanup rule {} is not safe and cannot be selected by default",
            spec.id
        ));
    }
    if spec.applicability.is_empty() {
        return Err(format!(
            "cleanup rule {} has no applicability probe",
            spec.id
        ));
    }
    // This registry directly serves ordinary cleanup. High-impact rules
    // require a separate preview, confirmation, and execution contract even
    // when not selected by default, so they must use a dedicated compiler
    // instead of weakening this safety gate.
    if matches!(spec.risk, RuleRiskLevel::HighImpact) {
        return Err(format!(
            "high-impact cleanup rule {} must use a dedicated cleaner",
            spec.id
        ));
    }
    if matches!(
        spec.verification.lifecycle,
        RuleLifecycle::Candidate | RuleLifecycle::Disabled
    ) {
        return Err(format!(
            "unverified or disabled rule {} cannot enter the production catalog",
            spec.id
        ));
    }
    if spec.verification.evidence.trim().is_empty() {
        return Err(format!(
            "cleanup rule {} has no verification evidence",
            spec.id
        ));
    }
    if spec.verification.verified_at.trim().is_empty() {
        return Err(format!("cleanup rule {} has no verification date", spec.id));
    }
    if spec.platform != spec.verification.verified_platform {
        return Err(format!(
            "cleanup rule {} has a mismatched verification platform",
            spec.id
        ));
    }
    // A validated dynamic root may resolve to no directories when its
    // application is not installed. Applicability then reports NotApplicable
    // without traversing the filesystem.
    let mut rule_roots: Vec<PathBuf> = Vec::new();
    for (root_index, root) in spec.roots.iter().enumerate() {
        if !root.resolved_path.is_absolute() {
            return Err(format!(
                "cleanup rule {} contains a non-absolute root at index {}",
                spec.id, root_index
            ));
        }
        for (existing_index, existing_root) in rule_roots.iter().enumerate() {
            if paths_overlap(&root.resolved_path, existing_root) {
                return Err(format!(
                    "cleanup rule {} contains overlapping roots at indexes {} and {}",
                    spec.id, existing_index, root_index
                ));
            }
        }
        rule_roots.push(root.resolved_path.clone());
    }
    Ok(())
}

fn valid_rule_id(id: &str) -> bool {
    !id.is_empty()
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    current_platform().path_is_same_or_child(left, right)
        || current_platform().path_is_same_or_child(right, left)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::rules::models::{
        ExecutionSpec, MatcherSpec, PlatformConstraint, RootSpec, RuleRiskLevel,
        VerificationMetadata,
    };
    use crate::cleanup::rules::ApplicabilityProbe;
    use crate::cleanup::CleanupCategory;

    fn spec(risk: RuleRiskLevel, lifecycle: RuleLifecycle, default_selected: bool) -> RuleSpec {
        RuleSpec {
            id: "fixture.rule".to_string(),
            schema_version: RULE_SCHEMA_VERSION,
            rule_version: 1,
            platform: current_platform(),
            category: CleanupCategory::System,
            risk,
            default_selected,
            recommended_selected: default_selected,
            applicability: vec![ApplicabilityProbe::AnyRootExists],
            roots: vec![RootSpec {
                resolved_path: std::env::temp_dir().join("mangodisk-rule-fixture"),
            }],
            matcher: MatcherSpec::All,
            execution: ExecutionSpec::DeleteMatchingContents {
                requires_app_close: false,
            },
            required_stopped_processes: Vec::new(),
            verification: VerificationMetadata {
                lifecycle,
                evidence: "fixture".to_string(),
                verified_at: "2026-07-17".to_string(),
                verified_platform: current_platform(),
            },
        }
    }

    #[test]
    fn high_impact_rules_cannot_be_selected_by_default() {
        let error = compile_rules(vec![spec(
            RuleRiskLevel::HighImpact,
            RuleLifecycle::Stable,
            true,
        )])
        .expect_err("high-impact rules must require explicit user selection");
        assert!(error.contains("cannot be selected by default"));
    }

    #[test]
    fn high_impact_rules_cannot_enter_ordinary_cleanup() {
        let error = compile_rules(vec![spec(
            RuleRiskLevel::HighImpact,
            RuleLifecycle::Verified,
            false,
        )])
        .expect_err("high-impact behavior requires an independent preview and confirmation flow");
        assert!(error.contains("must use a dedicated cleaner"));
    }

    #[test]
    fn disabled_rules_cannot_enter_the_production_catalog() {
        let error = compile_rules(vec![spec(
            RuleRiskLevel::Safe,
            RuleLifecycle::Disabled,
            false,
        )])
        .expect_err("disabled rules cannot enter a production cleanup plan");
        assert!(error.contains("cannot enter the production catalog"));
    }

    #[test]
    fn candidate_rules_cannot_enter_the_production_catalog() {
        let error = compile_rules(vec![spec(
            RuleRiskLevel::Safe,
            RuleLifecycle::Candidate,
            false,
        )])
        .expect_err("candidate rules are available only for development validation");
        assert!(error.contains("cannot enter the production catalog"));
    }

    #[cfg(windows)]
    #[test]
    fn overlapping_windows_roots_ignore_verbatim_prefix_and_case() {
        assert!(paths_overlap(
            Path::new(r"\\?\C:\Users\Fixture\Cache"),
            Path::new(r"c:\users\fixture")
        ));
    }

    #[test]
    fn resolved_dynamic_rule_can_have_no_installed_roots() {
        let mut rule = spec(RuleRiskLevel::Safe, RuleLifecycle::Verified, false);
        rule.roots.clear();
        let compiled = compile_rules(vec![rule]).expect("an absent dynamic root is not an error");
        assert!(compiled[0].roots.is_empty());
    }

    #[test]
    fn overlapping_root_error_does_not_expose_resolved_paths() {
        let mut rule = spec(RuleRiskLevel::Safe, RuleLifecycle::Verified, false);
        let parent = rule.roots[0].resolved_path.clone();
        let child = parent.join("private-cache");
        rule.roots.push(RootSpec {
            resolved_path: child.clone(),
        });

        let error = compile_rules(vec![rule]).expect_err("overlapping roots must be rejected");

        assert!(error.contains("overlapping roots at indexes 0 and 1"));
        assert!(!error.contains(&parent.to_string_lossy().to_string()));
        assert!(!error.contains(&child.to_string_lossy().to_string()));
    }

    #[test]
    fn non_absolute_root_error_does_not_expose_resolved_path() {
        let mut rule = spec(RuleRiskLevel::Safe, RuleLifecycle::Verified, false);
        rule.roots[0].resolved_path = PathBuf::from("private-cache");

        let error = compile_rules(vec![rule]).expect_err("relative roots must be rejected");

        assert!(error.contains("non-absolute root at index 0"));
        assert!(!error.contains("private-cache"));
    }

    #[test]
    fn nested_roots_compile_but_equal_priority_ownership_conflicts_fail() {
        let mut parent = spec(RuleRiskLevel::Safe, RuleLifecycle::Verified, false);
        parent.id = "system.parent".to_string();
        let mut child = spec(RuleRiskLevel::Safe, RuleLifecycle::Verified, false);
        child.id = "application.child".to_string();
        child.category = CleanupCategory::Application;
        child.roots[0].resolved_path = parent.roots[0].resolved_path.join("nested");
        compile_rules(vec![parent.clone(), child])
            .expect("nested roots must be merged by the scan plan");

        let mut conflict = parent.clone();
        conflict.id = "system.conflict".to_string();
        let error = compile_rules(vec![parent, conflict])
            .expect_err("equal-priority ownership must be rejected");
        assert!(error.contains("ownership conflict"));
    }

    const fn current_platform() -> PlatformConstraint {
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
    }
}
