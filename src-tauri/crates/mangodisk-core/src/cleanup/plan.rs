use std::collections::HashSet;

use blake3::Hasher;

use crate::{
    cleanup::{
        CleanupAutomationProfile, CleanupPlan, CleanupRequest, CleanupScanResult, RiskLevel,
        ScanItemStatus, CLEANUP_AUTOMATION_PROFILE_SCHEMA_VERSION, CLEANUP_PLAN_SCHEMA_VERSION,
    },
    filesystem::metadata::now_ms,
};

pub struct CleanupPlanService;

impl CleanupPlanService {
    pub fn parse_profile(content: &str) -> Result<CleanupAutomationProfile, String> {
        let profile = toml::from_str::<CleanupAutomationProfile>(content)
            .map_err(|error| format!("cleanup automation profile is invalid: {error}"))?;
        Self::validate_profile(&profile)?;
        Ok(profile)
    }

    pub fn create(
        scan: &CleanupScanResult,
        profile: CleanupAutomationProfile,
    ) -> Result<CleanupPlan, String> {
        Self::validate_profile(&profile)?;
        let excluded = profile
            .excluded_rule_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut candidates = scan
            .rules
            .iter()
            .filter(|rule| {
                rule.status == ScanItemStatus::Found
                    && rule.risk == RiskLevel::Safe
                    && rule.default_selected
                    && rule.selectable
                    && rule.bytes > 0
                    && !excluded.contains(rule.rule_id.as_str())
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .bytes
                .cmp(&left.bytes)
                .then_with(|| left.rule_id.cmp(&right.rule_id))
        });

        let mut rule_ids = Vec::new();
        let mut expected_bytes = 0u64;
        for candidate in candidates.into_iter().take(profile.max_rule_count) {
            let next_bytes = expected_bytes.saturating_add(candidate.bytes);
            if profile
                .max_reclaimable_bytes
                .is_some_and(|limit| next_bytes > limit)
            {
                continue;
            }
            expected_bytes = next_bytes;
            rule_ids.push(candidate.rule_id.clone());
        }
        if rule_ids.is_empty() {
            return Err(
                "no low-risk default cleanup items match the automation profile".to_string(),
            );
        }

        let created_at_ms = now_ms();
        let plan_hash = plan_hash(
            created_at_ms,
            scan.scanned_at_ms,
            &scan.schema_version,
            &rule_ids,
            expected_bytes,
            &profile,
        );
        Ok(CleanupPlan {
            schema_version: CLEANUP_PLAN_SCHEMA_VERSION,
            plan_id: format!("cleanup-plan-{}", &plan_hash[..16]),
            plan_hash,
            created_at_ms,
            source_scan_at_ms: scan.scanned_at_ms,
            source_scan_schema_version: scan.schema_version.clone(),
            rule_ids,
            expected_bytes,
            profile,
        })
    }

    pub fn execution_request(plan: &CleanupPlan, dry_run: bool) -> Result<CleanupRequest, String> {
        Self::validate(plan)?;
        Ok(CleanupRequest {
            rule_ids: plan.rule_ids.clone(),
            dry_run,
            project_roots: Vec::new(),
            source_selections: Vec::new(),
        })
    }

    /// Revalidates automation eligibility against a fresh read-only scan.
    ///
    /// A plan hash detects accidental edits, but it is not an authorization
    /// token. Rechecking live rule metadata prevents a hand-crafted plan or a
    /// later catalog change from promoting recoverable and manual rules into
    /// unattended cleanup.
    pub fn validate_against_scan(
        plan: &CleanupPlan,
        scan: &CleanupScanResult,
    ) -> Result<(), String> {
        Self::validate(plan)?;
        for rule_id in &plan.rule_ids {
            let rule = scan
                .rules
                .iter()
                .find(|rule| rule.rule_id == *rule_id)
                .ok_or_else(|| format!("cleanup plan rule is no longer available: {rule_id}"))?;
            if rule.status != ScanItemStatus::Found
                || rule.risk != RiskLevel::Safe
                || !rule.default_selected
                || !rule.selectable
            {
                return Err(format!(
                    "cleanup plan rule is no longer eligible for automation: {rule_id}"
                ));
            }
        }
        Ok(())
    }

    pub fn validate(plan: &CleanupPlan) -> Result<(), String> {
        if plan.schema_version != CLEANUP_PLAN_SCHEMA_VERSION {
            return Err(format!(
                "unsupported cleanup plan schema version: {}",
                plan.schema_version
            ));
        }
        Self::validate_profile(&plan.profile)?;
        if plan.rule_ids.is_empty() {
            return Err("cleanup plan contains no rules".to_string());
        }
        if plan.rule_ids.iter().collect::<HashSet<_>>().len() != plan.rule_ids.len() {
            return Err("cleanup plan contains duplicate rules".to_string());
        }
        let expected_hash = plan_hash(
            plan.created_at_ms,
            plan.source_scan_at_ms,
            &plan.source_scan_schema_version,
            &plan.rule_ids,
            plan.expected_bytes,
            &plan.profile,
        );
        if plan.plan_hash != expected_hash
            || plan.plan_id != format!("cleanup-plan-{}", &expected_hash[..16])
        {
            return Err("cleanup plan integrity validation failed".to_string());
        }
        Ok(())
    }

    fn validate_profile(profile: &CleanupAutomationProfile) -> Result<(), String> {
        if profile.schema_version != CLEANUP_AUTOMATION_PROFILE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported cleanup automation profile schema version: {}",
                profile.schema_version
            ));
        }
        if profile.max_rule_count == 0 || profile.max_rule_count > 100 {
            return Err(
                "cleanup automation profile maxRuleCount must be between 1 and 100".to_string(),
            );
        }
        if profile.max_reclaimable_bytes == Some(0) {
            return Err(
                "cleanup automation profile maxReclaimableBytes must be greater than zero"
                    .to_string(),
            );
        }
        if profile
            .excluded_rule_ids
            .iter()
            .any(|rule_id| rule_id.trim().is_empty())
        {
            return Err("cleanup automation profile contains an empty rule ID".to_string());
        }
        if profile
            .excluded_rule_ids
            .iter()
            .collect::<HashSet<_>>()
            .len()
            != profile.excluded_rule_ids.len()
        {
            return Err("cleanup automation profile contains duplicate rule IDs".to_string());
        }
        Ok(())
    }
}

fn plan_hash(
    created_at_ms: u64,
    source_scan_at_ms: u64,
    source_scan_schema_version: &str,
    rule_ids: &[String],
    expected_bytes: u64,
    profile: &CleanupAutomationProfile,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"mangodisk-cleanup-plan-v1");
    hasher.update(&created_at_ms.to_le_bytes());
    hasher.update(&source_scan_at_ms.to_le_bytes());
    hash_string(&mut hasher, source_scan_schema_version);
    hasher.update(&expected_bytes.to_le_bytes());
    hasher.update(&(profile.max_rule_count as u64).to_le_bytes());
    hasher.update(
        &profile
            .max_reclaimable_bytes
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for rule_id in rule_ids {
        hash_string(&mut hasher, rule_id);
    }
    for rule_id in &profile.excluded_rule_ids {
        hash_string(&mut hasher, rule_id);
    }
    hasher.finalize().to_hex().to_string()
}

fn hash_string(hasher: &mut Hasher, value: &str) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CleanupCategory, CleanupScanResult, DiskInfo, ScanRuleResult};

    fn scan_rule(id: &str, risk: RiskLevel, default_selected: bool, bytes: u64) -> ScanRuleResult {
        ScanRuleResult {
            rule_id: id.to_string(),
            category: CleanupCategory::System,
            group: crate::cleanup::CleanupGroup::System,
            risk,
            default_selected,
            recommended_selected: default_selected,
            bytes,
            file_count: 1,
            available: true,
            selectable: true,
            status: ScanItemStatus::Found,
            running_processes: Vec::new(),
            requires_app_close: false,
            sources: Vec::new(),
            source_count: 1,
            sources_truncated: false,
            scan_elapsed_ms: 1,
        }
    }

    fn scan_result() -> CleanupScanResult {
        CleanupScanResult {
            schema_version: "1.7".to_string(),
            custom_scan_id: None,
            missing_custom_root_count: 0,
            scanned_at_ms: 10,
            disk: DiskInfo {
                name: "fixture".to_string(),
                mount_point: "/fixture".to_string(),
                total_bytes: 100,
                available_bytes: 50,
                used_bytes: 50,
            },
            rules: vec![
                scan_rule("safe.large", RiskLevel::Safe, true, 40),
                scan_rule("safe.small", RiskLevel::Safe, true, 20),
                scan_rule("recoverable", RiskLevel::Recoverable, true, 80),
                scan_rule("manual", RiskLevel::Safe, false, 90),
            ],
            application_icons: Vec::new(),
            application_close_identities: Vec::new(),
            warning_count: 0,
            access_limited: false,
            read_failure_count: 0,
            permission_denied_read_failure_count: 0,
            read_failure_details: Vec::new(),
            safe_bytes: 60,
            reclaimable_bytes: 230,
            applicability_elapsed_ms: 1,
            applicable_rule_count: 4,
            filtered_rule_count: 0,
            inventory_application_count: 0,
            inventory_process_count: 0,
            elapsed_ms: 1,
        }
    }

    #[test]
    fn automation_plan_never_promotes_recoverable_or_manual_rules() {
        let plan = CleanupPlanService::create(&scan_result(), CleanupAutomationProfile::default())
            .expect("the fixture plan should be created");

        assert_eq!(plan.rule_ids, vec!["safe.large", "safe.small"]);
        assert_eq!(plan.expected_bytes, 60);
        CleanupPlanService::validate(&plan).expect("the generated plan should validate");
    }

    #[test]
    fn automation_limits_only_reduce_the_safe_default_set() {
        let profile = CleanupAutomationProfile {
            max_rule_count: 10,
            max_reclaimable_bytes: Some(30),
            ..CleanupAutomationProfile::default()
        };
        let plan = CleanupPlanService::create(&scan_result(), profile)
            .expect("the smaller safe rule should fit the limit");

        assert_eq!(plan.rule_ids, vec!["safe.small"]);
        assert_eq!(plan.expected_bytes, 20);
    }

    #[test]
    fn modified_plan_is_rejected() {
        let mut plan =
            CleanupPlanService::create(&scan_result(), CleanupAutomationProfile::default())
                .expect("the fixture plan should be created");
        plan.expected_bytes += 1;

        let error = CleanupPlanService::validate(&plan)
            .expect_err("an edited plan must fail integrity validation");
        assert_eq!(error, "cleanup plan integrity validation failed");
    }

    #[test]
    fn live_scan_cannot_promote_a_planned_rule_after_policy_changes() {
        let mut scan = scan_result();
        let plan = CleanupPlanService::create(&scan, CleanupAutomationProfile::default())
            .expect("the fixture plan should be created");
        scan.rules
            .iter_mut()
            .find(|rule| rule.rule_id == "safe.large")
            .expect("the fixture rule must exist")
            .risk = RiskLevel::Recoverable;

        let error = CleanupPlanService::validate_against_scan(&plan, &scan)
            .expect_err("a policy change must invalidate unattended execution");

        assert!(error.contains("no longer eligible"));
    }
}
