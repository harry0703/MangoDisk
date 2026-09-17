use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use mangodisk_platform::{current_platform, Platform};

use super::{models::ExecutionSpec, CompiledRule, MatcherSpec};
use crate::cleanup::rules::maximum_match_depth;
use crate::cleanup::CleanupCategory;

/// A scan plan stores rule activation boundaries rather than a complete file
/// tree. Its lifetime is limited to one scan so reducing duplicate I/O does not
/// duplicate the current-process storage-analysis snapshot.
#[derive(Debug)]
pub(crate) struct ScanPlan {
    pub(crate) plan_id: String,
    pub(crate) rules: Vec<CompiledRule>,
    pub(crate) root_tasks: Vec<RootScanTask>,
    pub(crate) completed_without_io: Vec<usize>,
}

#[derive(Debug)]
pub(crate) struct RootScanTask {
    pub(crate) root: PathBuf,
    pub(crate) volume_root: PathBuf,
    activations: Vec<RuleActivation>,
    dispatch: PathTrie,
}

#[derive(Debug, Clone)]
pub(crate) struct RuleActivation {
    pub(crate) rule_index: usize,
    pub(crate) root: PathBuf,
    root_depth: usize,
}

#[derive(Debug, Default)]
struct PathTrie {
    activation_indices: Vec<usize>,
    children: BTreeMap<String, PathTrie>,
}

/// Applicability evaluation already checks root existence. Plan compilation
/// does not access directory contents again, so inapplicable rules finish
/// without another filesystem probe as rule coverage grows.
pub(crate) fn compile_scan_plan(
    rules: Vec<CompiledRule>,
    availability: &[bool],
    volume_roots: &[PathBuf],
) -> Result<ScanPlan, String> {
    if rules.len() != availability.len() {
        return Err("scan plan rule count does not match applicability results".to_string());
    }

    let mut completed_without_io = Vec::new();
    let mut activations = Vec::new();
    for (rule_index, (rule, available)) in rules.iter().zip(availability).enumerate() {
        if !available || rule.roots.is_empty() {
            completed_without_io.push(rule_index);
            continue;
        }
        for root in &rule.roots {
            activations.push(RuleActivation {
                rule_index,
                root: root.clone(),
                root_depth: path_depth(root),
            });
        }
    }

    activations.sort_by(|left, right| {
        left.root_depth
            .cmp(&right.root_depth)
            .then_with(|| path_identity(&left.root).cmp(&path_identity(&right.root)))
            .then_with(|| rules[left.rule_index].id.cmp(&rules[right.rule_index].id))
    });
    validate_ownership_conflicts(&rules, &activations)?;

    let mut root_tasks: Vec<RootScanTask> = Vec::new();
    for activation in activations {
        let volume_root = resolve_volume_root(&activation.root, volume_roots);
        if let Some(task) = root_tasks.iter_mut().find(|task| {
            same_path(&task.volume_root, &volume_root)
                && path_contains(&task.root, &activation.root)
        }) {
            task.insert_activation(activation)?;
            continue;
        }
        let mut task = RootScanTask {
            root: activation.root.clone(),
            volume_root,
            activations: Vec::new(),
            dispatch: PathTrie::default(),
        };
        task.insert_activation(activation)?;
        root_tasks.push(task);
    }
    root_tasks = interleave_tasks_by_rule(root_tasks);

    let plan_id = plan_digest(&rules, &root_tasks, &completed_without_io);
    Ok(ScanPlan {
        plan_id,
        rules,
        root_tasks,
        completed_without_io,
    })
}

impl ScanPlan {
    /// Returns roots owned by rules that are active in this operation.
    ///
    /// Consumers that subtract declarative ownership from a dynamic inventory
    /// must not use every compiled root. An installed-application probe can
    /// intentionally deactivate a rule while its stale cache still exists; in
    /// that case the dynamic inventory should remain able to report the cache.
    pub(crate) fn active_rule_roots(&self) -> Vec<PathBuf> {
        let mut active = vec![true; self.rules.len()];
        for rule_index in &self.completed_without_io {
            active[*rule_index] = false;
        }
        self.rules
            .iter()
            .zip(active)
            .filter(|(_, active)| *active)
            .flat_map(|(rule, _)| rule.roots.iter().cloned())
            .collect()
    }

    /// Pre-cleanup validation must preserve the same unique ownership used by
    /// scanning. Otherwise a parent rule could delete files owned by an
    /// unselected child rule even though the UI never double-counted them.
    pub(crate) fn rule_owns_path(
        &self,
        rule_index: usize,
        path: &Path,
        metadata: &fs::Metadata,
    ) -> bool {
        self.root_tasks
            .iter()
            .find(|task| task.contains(path))
            .and_then(|task| task.matching_owner(path, metadata, &self.rules))
            .is_some_and(|owner| owner.rule_index == rule_index)
    }

    /// Resolves ownership for an empty directory without applying file-only
    /// matchers. The deepest active root remains authoritative, while the
    /// existing custom-rule tie breaker keeps same-root behavior consistent
    /// with file ownership.
    pub(crate) fn empty_directory_owner(&self, path: &Path) -> Option<usize> {
        let task = self.root_tasks.iter().find(|task| task.contains(path))?;
        task.empty_directory_owner(path, &self.rules)
    }

    /// Revalidates that an execution rule still owns a scan-authorized empty
    /// directory after applicability and overlapping roots are recomputed.
    pub(crate) fn rule_owns_empty_directory(&self, rule_index: usize, path: &Path) -> bool {
        self.empty_directory_owner(path) == Some(rule_index)
    }

    /// Proves that deleting `root` cannot consume a nested boundary owned by
    /// another active rule. A broader parent activation is harmless because
    /// the exact, deeper activation retains ownership of this complete root.
    pub(crate) fn rule_exclusively_owns_root(&self, rule_index: usize, root: &Path) -> bool {
        self.root_tasks
            .iter()
            .find(|task| task.contains(root))
            .is_some_and(|task| {
                task.activations.iter().any(|activation| {
                    activation.rule_index == rule_index && same_path(&activation.root, root)
                }) && task.activations.iter().all(|activation| {
                    activation.rule_index == rule_index || !path_contains(root, &activation.root)
                })
            })
    }
}

pub(super) fn validate_rule_ownership(rules: &[CompiledRule]) -> Result<(), String> {
    let activations = rules
        .iter()
        .enumerate()
        .flat_map(|(rule_index, rule)| {
            rule.roots.iter().cloned().map(move |root| RuleActivation {
                rule_index,
                root_depth: path_depth(&root),
                root,
            })
        })
        .collect::<Vec<_>>();
    validate_ownership_conflicts(rules, &activations)
}

impl RootScanTask {
    fn contains(&self, path: &Path) -> bool {
        path_contains(&self.root, path)
    }

    fn insert_activation(&mut self, activation: RuleActivation) -> Result<(), String> {
        let relative = relative_components(&self.root, &activation.root).ok_or_else(|| {
            "scan plan contains a rule boundary outside its task root".to_string()
        })?;
        let activation_index = self.activations.len();
        self.dispatch.insert(&relative, activation_index);
        self.activations.push(activation);
        Ok(())
    }

    /// Returns rule roots active for the current path. PathTrie follows only
    /// relative path prefixes, so each file does not iterate the complete rule
    /// catalog as coverage grows.
    pub(crate) fn active_rules(&self, path: &Path) -> Vec<&RuleActivation> {
        let Some(relative) = relative_components(&self.root, path) else {
            return Vec::new();
        };
        self.dispatch
            .active_activation_indices(&relative)
            .into_iter()
            .filter_map(|activation_index| self.activations.get(activation_index))
            .collect()
    }

    /// Directory cleanup does not apply file-only matchers. It assigns each
    /// empty descendant to the most specific active root and never authorizes
    /// the selected root itself, preserving the same overlap boundary used by
    /// file ownership without widening the user's selected scope.
    pub(crate) fn empty_directory_owner(
        &self,
        path: &Path,
        rules: &[CompiledRule],
    ) -> Option<usize> {
        let active_rules = self.active_rules(path);
        let deepest_root = active_rules
            .iter()
            .map(|activation| activation.root_depth)
            .max()?;
        // A deeper root is an explicit ownership boundary even when its rule
        // keeps empty directories. At the same boundary, however, one opted-in
        // custom rule must not be disabled merely because another same-root
        // rule has a more specific file matcher or a later identifier.
        let owner = preferred_owner(
            active_rules.into_iter().filter(|activation| {
                activation.root_depth == deepest_root
                    && rules[activation.rule_index].remove_empty_directories
            }),
            rules,
        )?;
        (!same_path(path, &owner.root)).then_some(owner.rule_index)
    }

    /// A bounded matcher can stop traversal before entering unrelated deep
    /// trees. Nested rule activations still take precedence: a directory is
    /// retained whenever it leads to another rule root, even if an active
    /// parent rule has already reached its own depth limit.
    pub(crate) fn should_descend(&self, path: &Path, rules: &[CompiledRule]) -> bool {
        self.activations.iter().any(|activation| {
            if path_contains(path, &activation.root) && !same_path(path, &activation.root) {
                return true;
            }
            if !path_contains(&activation.root, path) {
                return false;
            }
            let current_depth = relative_components(&activation.root, path)
                .map(|components| components.len())
                .unwrap_or(usize::MAX);
            maximum_match_depth(&rules[activation.rule_index].matcher)
                .is_none_or(|maximum_depth| current_depth < maximum_depth)
        })
    }

    /// Most production tasks contain one rule. This fast path avoids allocating
    /// an active-rule set and evaluates matching and ownership directly. Only
    /// merged nested roots use multi-rule PathTrie dispatch.
    pub(crate) fn matching_owner<'a>(
        &'a self,
        path: &Path,
        metadata: &fs::Metadata,
        rules: &[CompiledRule],
    ) -> Option<&'a RuleActivation> {
        if let [activation] = self.activations.as_slice() {
            return crate::cleanup::rules::matches_rule(
                &activation.root,
                path,
                metadata,
                Some(&rules[activation.rule_index].matcher),
            )
            .then_some(activation);
        }
        preferred_owner(
            self.active_rules(path).into_iter().filter(|activation| {
                crate::cleanup::rules::matches_rule(
                    &activation.root,
                    path,
                    metadata,
                    Some(&rules[activation.rule_index].matcher),
                )
            }),
            rules,
        )
    }

    /// An unreadable directory has no file metadata for matcher evaluation.
    /// Ownership still uses root specificity, category, and matcher
    /// specificity so one permission failure is not counted by multiple rules.
    pub(crate) fn fallback_owner<'a>(
        &'a self,
        path: &Path,
        rules: &[CompiledRule],
    ) -> Option<&'a RuleActivation> {
        self.active_rules(path)
            .into_iter()
            .max_by_key(|activation| {
                (
                    ownership_rank(&rules[activation.rule_index], activation.root_depth),
                    (rules[activation.rule_index].category == CleanupCategory::Custom)
                        .then_some(rules[activation.rule_index].id.as_str()),
                )
            })
    }

    pub(crate) fn rule_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.activations
            .iter()
            .enumerate()
            .filter(|(index, activation)| {
                !self.activations[..*index]
                    .iter()
                    .any(|existing| existing.rule_index == activation.rule_index)
            })
            .map(|(_, activation)| activation.rule_index)
    }

    /// Native directory aggregation is safe only when one rule owns the
    /// complete task root and every descendant matches. Keeping this decision
    /// in the scan plan prevents platform implementations from learning rule
    /// IDs or cleanup policy, while nested ownership and filtered matchers
    /// continue through the generic traversal.
    pub(crate) fn complete_root_rule_index(&self, rules: &[CompiledRule]) -> Option<usize> {
        let [activation] = self.activations.as_slice() else {
            return None;
        };
        let rule = rules.get(activation.rule_index)?;
        if !same_path(&self.root, &activation.root)
            || !matches!(rule.matcher, MatcherSpec::All)
            || rule.remove_empty_directories
            || !matches!(
                rule.execution,
                ExecutionSpec::DeleteMatchingContents { .. }
                    | ExecutionSpec::DeleteWholeRoot { .. }
            )
        {
            return None;
        }
        Some(activation.rule_index)
    }

    fn first_rule_index(&self) -> usize {
        self.rule_indices().min().unwrap_or(usize::MAX)
    }
}

fn preferred_owner<'a>(
    activations: impl IntoIterator<Item = &'a RuleActivation>,
    rules: &[CompiledRule],
) -> Option<&'a RuleActivation> {
    let mut owner = None;
    for activation in activations {
        let rank = ownership_rank(&rules[activation.rule_index], activation.root_depth);
        if owner.is_none_or(|current: &RuleActivation| {
            let current_rank = ownership_rank(&rules[current.rule_index], current.root_depth);
            rank > current_rank
                || (rank == current_rank
                    && rules[activation.rule_index].category == CleanupCategory::Custom
                    && rules[activation.rule_index].id > rules[current.rule_index].id)
        }) {
            owner = Some(activation);
        }
    }
    owner
}

fn interleave_tasks_by_rule(tasks: Vec<RootScanTask>) -> Vec<RootScanTask> {
    let mut groups: BTreeMap<usize, Vec<RootScanTask>> = BTreeMap::new();
    for task in tasks {
        groups
            .entry(task.first_rule_index())
            .or_default()
            .push(task);
    }
    for tasks in groups.values_mut() {
        tasks.sort_by(|left, right| path_identity(&left.root).cmp(&path_identity(&right.root)));
    }
    let mut groups = groups
        .into_iter()
        .map(|(rule_index, tasks)| (rule_index, VecDeque::from(tasks)))
        .collect::<BTreeMap<_, _>>();
    let maximum_group_size = groups.values().map(VecDeque::len).max().unwrap_or_default();
    let mut result = Vec::new();
    for _ in 0..maximum_group_size {
        for tasks in groups.values_mut() {
            if let Some(task) = tasks.pop_front() {
                // Take one root per rule in each round so a multi-root rule
                // cannot occupy an entire worker batch.
                result.push(task);
            }
        }
    }
    result
}

impl PathTrie {
    fn insert(&mut self, components: &[String], activation_index: usize) {
        let mut node = self;
        for component in components {
            node = node.children.entry(component.clone()).or_default();
        }
        node.activation_indices.push(activation_index);
    }

    fn active_activation_indices(&self, components: &[String]) -> Vec<usize> {
        let mut result = self.activation_indices.clone();
        let mut node = self;
        for component in components {
            let Some(child) = node.children.get(component) else {
                break;
            };
            node = child;
            result.extend_from_slice(&node.activation_indices);
        }
        result
    }
}

fn validate_ownership_conflicts(
    rules: &[CompiledRule],
    activations: &[RuleActivation],
) -> Result<(), String> {
    for (index, left) in activations.iter().enumerate() {
        for right in &activations[index + 1..] {
            if !same_path(&left.root, &right.root) {
                continue;
            }
            let left_rank = ownership_rank(&rules[left.rule_index], left.root_depth);
            let right_rank = ownership_rank(&rules[right.rule_index], right.root_depth);
            if left_rank == right_rank
                && !(rules[left.rule_index].category == CleanupCategory::Custom
                    && rules[right.rule_index].category == CleanupCategory::Custom)
            {
                return Err(format!(
                    "scan plan ownership conflict: {} and {} use the same root and priority",
                    rules[left.rule_index].id, rules[right.rule_index].id
                ));
            }
        }
    }
    Ok(())
}

fn ownership_rank(rule: &CompiledRule, root_depth: usize) -> (u8, usize, u8, u16) {
    // A user-authored matcher represents narrower explicit intent than catalog
    // ownership. Give it precedence only for paths it actually matches; all
    // remaining descendants retain the existing depth and category ordering.
    (
        u8::from(rule.category == CleanupCategory::Custom),
        root_depth,
        category_priority(rule.category),
        matcher_specificity(&rule.matcher),
    )
}

const fn category_priority(category: CleanupCategory) -> u8 {
    match category {
        // System rules are usually platform-wide fallbacks. Explicit
        // application and tool categories should own their narrower content.
        CleanupCategory::System => 0,
        CleanupCategory::Custom => 2,
        CleanupCategory::Application
        | CleanupCategory::Browser
        | CleanupCategory::Development
        | CleanupCategory::Project
        | CleanupCategory::Xcode
        | CleanupCategory::ApplicationOptimization
        | CleanupCategory::Ai
        | CleanupCategory::Container => 1,
    }
}

fn matcher_specificity(matcher: &MatcherSpec) -> u16 {
    match matcher {
        MatcherSpec::All => 0,
        MatcherSpec::FileOnly => 2,
        MatcherSpec::MaxDepth(_) => 1,
        MatcherSpec::OlderThanDays(_)
        | MatcherSpec::NewerThanDays(_)
        | MatcherSpec::LargerThanBytes(_)
        | MatcherSpec::SmallerThanBytes(_) => 2,
        MatcherSpec::NameEquals(_)
        | MatcherSpec::NameGlob(_)
        | MatcherSpec::ExtensionIn(_)
        | MatcherSpec::PathSegmentIn(_) => 4,
        MatcherSpec::Not(item) => 1u16.saturating_add(matcher_specificity(item)),
        MatcherSpec::AnyOf(items) => 2u16.saturating_add(
            items
                .iter()
                .map(matcher_specificity)
                .min()
                .unwrap_or_default(),
        ),
        MatcherSpec::AllOf(items) => 8u16.saturating_add(
            items
                .iter()
                .map(matcher_specificity)
                .fold(0u16, u16::saturating_add),
        ),
    }
}

fn plan_digest(
    rules: &[CompiledRule],
    tasks: &[RootScanTask],
    completed_without_io: &[usize],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hash_bytes(&mut hasher, b"mangodisk-scan-plan-v3");
    // Hash complete rule semantics before task topology. A rule version or
    // matcher change must produce a new plan ID even when the rule performs no
    // I/O on this machine, preventing baselines from conflating two plans.
    for (rule_index, rule) in rules.iter().enumerate() {
        hash_bytes(&mut hasher, b"compiled-rule");
        hash_text(&mut hasher, &rule.id);
        hasher.update(&rule.rule_version.to_le_bytes());
        hash_text(&mut hasher, rule.platform.as_str());
        hash_text(&mut hasher, rule.category.as_str());
        hash_bytes(
            &mut hasher,
            match rule.execution {
                ExecutionSpec::DeleteMatchingContents { .. } => b"delete-matching-contents",
                ExecutionSpec::DeleteWholeRoot { .. } => b"delete-whole-root",
            },
        );
        hasher.update(&[u8::from(rule.requires_app_close())]);
        hasher.update(&[u8::from(rule.remove_empty_directories)]);
        hash_matcher(&mut hasher, &rule.matcher);
        hasher.update(&(rule.roots.len() as u64).to_le_bytes());
        for root in &rule.roots {
            hash_text(&mut hasher, &path_identity(root));
        }
        hash_bytes(
            &mut hasher,
            if completed_without_io.contains(&rule_index) {
                b"completed-without-io"
            } else {
                b"requires-io"
            },
        );
    }
    for task in tasks {
        hash_bytes(&mut hasher, b"root-task");
        hash_text(&mut hasher, &path_identity(&task.volume_root));
        hash_text(&mut hasher, &path_identity(&task.root));
        for activation in &task.activations {
            let rule = &rules[activation.rule_index];
            hash_bytes(&mut hasher, b"rule-activation");
            hash_text(&mut hasher, &rule.id);
            hash_text(&mut hasher, &path_identity(&activation.root));
        }
    }
    hasher.finalize().to_hex().to_string()
}

/// Plan IDs enter baseline reports and therefore cannot depend on `Debug`
/// output or unframed concatenation. Type tags and length framing give
/// different matcher trees unique input structures and allow explicit protocol
/// upgrades when fields are added.
fn hash_matcher(hasher: &mut blake3::Hasher, matcher: &MatcherSpec) {
    match matcher {
        MatcherSpec::All => hash_bytes(hasher, b"all"),
        MatcherSpec::FileOnly => hash_bytes(hasher, b"file-only"),
        MatcherSpec::NameEquals(values) => hash_text_list(hasher, b"name-equals", values),
        MatcherSpec::NameGlob(values) => hash_text_list(hasher, b"name-glob", values),
        MatcherSpec::ExtensionIn(values) => hash_text_list(hasher, b"extension-in", values),
        MatcherSpec::PathSegmentIn(values) => hash_text_list(hasher, b"path-segment-in", values),
        MatcherSpec::OlderThanDays(value) => hash_number(hasher, b"older-than-days", *value),
        MatcherSpec::NewerThanDays(value) => hash_number(hasher, b"newer-than-days", *value),
        MatcherSpec::LargerThanBytes(value) => hash_number(hasher, b"larger-than-bytes", *value),
        MatcherSpec::SmallerThanBytes(value) => hash_number(hasher, b"smaller-than-bytes", *value),
        MatcherSpec::MaxDepth(value) => hash_number(hasher, b"max-depth", *value as u64),
        MatcherSpec::AllOf(items) => hash_matcher_list(hasher, b"all-of", items),
        MatcherSpec::AnyOf(items) => hash_matcher_list(hasher, b"any-of", items),
        MatcherSpec::Not(item) => {
            hash_bytes(hasher, b"not");
            hash_matcher(hasher, item);
        }
    }
}

fn hash_matcher_list(hasher: &mut blake3::Hasher, tag: &[u8], items: &[MatcherSpec]) {
    hash_bytes(hasher, tag);
    hasher.update(&(items.len() as u64).to_le_bytes());
    for item in items {
        hash_matcher(hasher, item);
    }
}

fn hash_text_list(hasher: &mut blake3::Hasher, tag: &[u8], values: &[String]) {
    hash_bytes(hasher, tag);
    hasher.update(&(values.len() as u64).to_le_bytes());
    for value in values {
        hash_text(hasher, value);
    }
}

fn hash_number(hasher: &mut blake3::Hasher, tag: &[u8], value: u64) {
    hash_bytes(hasher, tag);
    hasher.update(&value.to_le_bytes());
}

fn hash_text(hasher: &mut blake3::Hasher, value: &str) {
    hash_bytes(hasher, value.as_bytes());
}

fn hash_bytes(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn resolve_volume_root(root: &Path, volume_roots: &[PathBuf]) -> PathBuf {
    volume_roots
        .iter()
        .filter(|volume| path_contains(volume, root))
        .max_by_key(|volume| path_depth(volume))
        .cloned()
        .unwrap_or_else(|| root.ancestors().last().unwrap_or(root).to_path_buf())
}

fn relative_components(parent: &Path, child: &Path) -> Option<Vec<String>> {
    current_platform()
        .relative_path(child, parent)?
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .map(|value| current_platform().path_identity_key(Path::new(value)))
        })
        .collect()
}

fn path_contains(parent: &Path, child: &Path) -> bool {
    current_platform().path_is_same_or_child(child, parent)
}

fn same_path(left: &Path, right: &Path) -> bool {
    current_platform().paths_equal(left, right)
}

fn path_depth(path: &Path) -> usize {
    path.components().count()
}

fn path_identity(path: &Path) -> String {
    current_platform().path_identity_key(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::rules::models::{
        ApplicabilityProbe, ExecutionSpec, PlatformConstraint, RuleLifecycle, RuleRiskLevel,
        VerificationMetadata, RULE_SCHEMA_VERSION,
    };

    fn rule(
        id: &str,
        root: PathBuf,
        category: CleanupCategory,
        matcher: MatcherSpec,
    ) -> CompiledRule {
        CompiledRule {
            id: id.to_string(),
            schema_version: RULE_SCHEMA_VERSION,
            rule_version: 1,
            platform: current_platform(),
            category,
            risk: RuleRiskLevel::Safe,
            default_selected: true,
            recommended_selected: true,
            applicability: vec![ApplicabilityProbe::AnyRootExists],
            roots: vec![root],
            matcher,
            execution: ExecutionSpec::DeleteMatchingContents {
                requires_app_close: false,
            },
            remove_empty_directories: false,
            required_stopped_processes: Vec::new(),
            verification: VerificationMetadata {
                lifecycle: RuleLifecycle::Verified,
                evidence: "fixture".to_string(),
                verified_at: "2026-07-17".to_string(),
                verified_platform: current_platform(),
            },
        }
    }

    #[test]
    fn nested_roots_compile_into_one_explicit_root_task() {
        let parent = std::env::temp_dir().join("mangodisk-plan-parent");
        let child = parent.join("nested/cache");
        let rules = vec![
            rule(
                "system.parent",
                parent.clone(),
                CleanupCategory::System,
                MatcherSpec::All,
            ),
            rule(
                "application.child",
                child.clone(),
                CleanupCategory::Application,
                MatcherSpec::All,
            ),
        ];

        let first = compile_scan_plan(rules.clone(), &[true, true], &[std::env::temp_dir()])
            .expect("nested roots must compile");
        let second = compile_scan_plan(rules, &[true, true], &[std::env::temp_dir()])
            .expect("repeated compilation must succeed");

        assert_eq!(first.root_tasks.len(), 1);
        assert!(same_path(&first.root_tasks[0].root, &parent));
        assert_eq!(
            first.root_tasks[0]
                .active_rules(&child.join("file.bin"))
                .len(),
            2
        );
        assert_eq!(first.plan_id, second.plan_id, "plan digest must be stable");
    }

    #[test]
    fn same_rule_nested_roots_keep_distinct_activation_boundaries() {
        let parent = std::env::temp_dir().join("mangodisk-plan-same-rule-parent");
        let child = parent.join("nested/cache");
        let sibling_file = parent.join("other/file.log");
        let child_file = child.join("file.log");
        let mut compiled = rule(
            "custom.same-rule",
            parent.clone(),
            CleanupCategory::Custom,
            MatcherSpec::All,
        );
        compiled.roots.push(child.clone());

        let plan = compile_scan_plan(vec![compiled], &[true], &[std::env::temp_dir()])
            .expect("nested roots owned by one rule must compile");
        let task = &plan.root_tasks[0];

        let sibling_activations = task.active_rules(&sibling_file);
        assert_eq!(sibling_activations.len(), 1);
        assert!(same_path(&sibling_activations[0].root, &parent));
        let child_activations = task.active_rules(&child_file);
        assert_eq!(child_activations.len(), 2);
        assert!(child_activations
            .iter()
            .any(|activation| same_path(&activation.root, &child)));
        assert_eq!(task.rule_indices().collect::<Vec<_>>(), vec![0]);
    }

    #[test]
    fn empty_directory_removal_changes_the_scan_plan_identity() {
        let root = std::env::temp_dir().join("mangodisk-plan-empty-directory-option");
        let disabled = rule(
            "custom.empty-directory-option",
            root.clone(),
            CleanupCategory::Custom,
            MatcherSpec::All,
        );
        let mut enabled = disabled.clone();
        enabled.remove_empty_directories = true;

        let disabled_plan = compile_scan_plan(vec![disabled], &[true], &[std::env::temp_dir()])
            .expect("the disabled option plan must compile");
        let enabled_plan = compile_scan_plan(vec![enabled], &[true], &[std::env::temp_dir()])
            .expect("the enabled option plan must compile");

        assert_ne!(disabled_plan.plan_id, enabled_plan.plan_id);
    }

    #[test]
    fn same_root_rule_cannot_mask_an_empty_directory_opt_in() {
        let root = std::env::temp_dir().join("mangodisk-plan-same-root-empty-directory");
        let descendant = root.join("empty");
        let mut enabled = rule(
            "custom.enabled-empty-directory",
            root.clone(),
            CleanupCategory::Custom,
            MatcherSpec::All,
        );
        enabled.remove_empty_directories = true;
        let disabled = rule(
            "custom.zzz-disabled-empty-directory",
            root,
            CleanupCategory::Custom,
            MatcherSpec::ExtensionIn(vec!["tmp".to_string()]),
        );

        let plan = compile_scan_plan(
            vec![enabled, disabled],
            &[true, true],
            &[std::env::temp_dir()],
        )
        .expect("same-root custom rules must compile");

        assert_eq!(plan.empty_directory_owner(&descendant), Some(0));
    }

    #[test]
    fn matching_owner_does_not_promote_an_unmatched_activation_of_the_same_rule() {
        let parent = std::env::temp_dir().join(format!(
            "mangodisk-plan-activation-match-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .expect("system time must be valid")
                .as_nanos()
        ));
        let child = parent.join("nested/cache");
        let file = child.join("file.log");
        fs::create_dir_all(&child).expect("create the activation fixture");
        fs::write(&file, b"fixture").expect("write the activation fixture");
        let mut compiled = rule(
            "custom.same-rule-filtered",
            parent.clone(),
            CleanupCategory::Custom,
            MatcherSpec::Not(Box::new(MatcherSpec::MaxDepth(1))),
        );
        compiled.roots.push(child.clone());
        let plan = compile_scan_plan(vec![compiled], &[true], &[std::env::temp_dir()])
            .expect("compile nested filtered activations");
        let metadata = fs::metadata(&file).expect("read the activation fixture metadata");

        let owner = plan.root_tasks[0]
            .matching_owner(&file, &metadata, &plan.rules)
            .expect("the parent activation should match");

        assert!(same_path(&owner.root, &parent));
        fs::remove_dir_all(parent).expect("remove the activation fixture");
    }

    #[test]
    fn sibling_roots_do_not_expand_to_a_common_parent() {
        let parent = std::env::temp_dir().join("mangodisk-plan-siblings");
        let left = parent.join("left");
        let right = parent.join("right");
        let plan = compile_scan_plan(
            vec![
                rule(
                    "application.left",
                    left.clone(),
                    CleanupCategory::Application,
                    MatcherSpec::All,
                ),
                rule(
                    "application.right",
                    right.clone(),
                    CleanupCategory::Application,
                    MatcherSpec::All,
                ),
            ],
            &[true, true],
            &[std::env::temp_dir()],
        )
        .expect("sibling roots must remain independent");

        assert_eq!(plan.root_tasks.len(), 2);
        assert!(plan
            .root_tasks
            .iter()
            .any(|task| same_path(&task.root, &left)));
        assert!(plan
            .root_tasks
            .iter()
            .any(|task| same_path(&task.root, &right)));
        assert!(plan
            .root_tasks
            .iter()
            .all(|task| !same_path(&task.root, &parent)));
    }

    #[test]
    fn ownership_prefers_specific_roots_and_specialized_categories() {
        let root = std::env::temp_dir().join("mangodisk-plan-owner");
        let child = root.join("child");
        let plan = compile_scan_plan(
            vec![
                rule(
                    "system.general",
                    root.clone(),
                    CleanupCategory::System,
                    MatcherSpec::All,
                ),
                rule(
                    "application.specific",
                    child.clone(),
                    CleanupCategory::Application,
                    MatcherSpec::All,
                ),
            ],
            &[true, true],
            &[std::env::temp_dir()],
        )
        .expect("nested roots with stable priority must compile");
        let task = &plan.root_tasks[0];
        let owner = preferred_owner(task.active_rules(&child.join("file.bin")), &plan.rules)
            .expect("a matching file must have an owner");

        assert_eq!(owner.rule_index, 1);
    }

    #[test]
    fn matching_custom_rule_overrides_nested_catalog_ownership() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-owner-{}-{}",
            std::process::id(),
            crate::filesystem::metadata::now_ms()
        ));
        let catalog_root = root.join("logs");
        let log_file = catalog_root.join("application.log");
        let catalog_file = catalog_root.join("cache.bin");
        std::fs::create_dir_all(&catalog_root).expect("create the overlapping rule fixture");
        std::fs::write(&log_file, b"diagnostic log").expect("write the overlapping log fixture");
        std::fs::write(&catalog_file, b"catalog cache")
            .expect("write the overlapping catalog fixture");
        let plan = compile_scan_plan(
            vec![
                rule(
                    "application.catalog-cache",
                    catalog_root,
                    CleanupCategory::Application,
                    MatcherSpec::All,
                ),
                rule(
                    "custom.explicit-logs",
                    root.clone(),
                    CleanupCategory::Custom,
                    MatcherSpec::AllOf(vec![
                        MatcherSpec::FileOnly,
                        MatcherSpec::NameGlob(vec!["*.log".to_string()]),
                    ]),
                ),
            ],
            &[true, true],
            &[std::env::temp_dir()],
        )
        .expect("overlapping catalog and custom rules must compile");
        let metadata = std::fs::metadata(&log_file).expect("read the overlapping log metadata");
        let owner = plan.root_tasks[0]
            .matching_owner(&log_file, &metadata, &plan.rules)
            .expect("the matching log must have one owner");

        assert_eq!(plan.rules[owner.rule_index].id, "custom.explicit-logs");
        let catalog_metadata =
            std::fs::metadata(&catalog_file).expect("read the overlapping catalog metadata");
        let catalog_owner = plan.root_tasks[0]
            .matching_owner(&catalog_file, &catalog_metadata, &plan.rules)
            .expect("the nonmatching catalog file must retain an owner");
        assert_eq!(
            plan.rules[catalog_owner.rule_index].id,
            "application.catalog-cache"
        );
        std::fs::remove_dir_all(root).expect("remove the overlapping rule fixture");
    }

    #[test]
    fn identical_roots_and_priorities_are_rejected() {
        let root = std::env::temp_dir().join("mangodisk-plan-conflict");
        let error = compile_scan_plan(
            vec![
                rule(
                    "application.first",
                    root.clone(),
                    CleanupCategory::Application,
                    MatcherSpec::All,
                ),
                rule(
                    "application.second",
                    root,
                    CleanupCategory::Application,
                    MatcherSpec::All,
                ),
            ],
            &[true, true],
            &[std::env::temp_dir()],
        )
        .expect_err("equal-priority ownership cannot depend on registration order");

        assert!(error.contains("ownership conflict"));
    }

    #[test]
    fn inapplicable_rules_complete_without_traversal() {
        let root = std::env::temp_dir().join("mangodisk-plan-unavailable");
        let plan = compile_scan_plan(
            vec![rule(
                "application.unavailable",
                root,
                CleanupCategory::Application,
                MatcherSpec::All,
            )],
            &[false],
            &[std::env::temp_dir()],
        )
        .expect("inapplicable rules must complete directly");

        assert!(plan.root_tasks.is_empty());
        assert_eq!(plan.completed_without_io, vec![0]);
    }

    #[test]
    fn active_rule_roots_exclude_inapplicable_ownership() {
        let active = std::env::temp_dir().join("mangodisk-plan-active");
        let inactive = std::env::temp_dir().join("mangodisk-plan-inactive");
        let plan = compile_scan_plan(
            vec![
                rule(
                    "application.active",
                    active.clone(),
                    CleanupCategory::Application,
                    MatcherSpec::All,
                ),
                rule(
                    "application.inactive",
                    inactive,
                    CleanupCategory::Application,
                    MatcherSpec::All,
                ),
            ],
            &[true, false],
            &[std::env::temp_dir()],
        )
        .expect("mixed applicability must compile");

        assert_eq!(plan.active_rule_roots(), vec![active]);
    }

    #[test]
    fn complete_root_aggregation_requires_one_unfiltered_owner() {
        let root = std::env::temp_dir().join("mangodisk-plan-native-aggregate");
        let complete = compile_scan_plan(
            vec![rule(
                "development.complete",
                root.clone(),
                CleanupCategory::Development,
                MatcherSpec::All,
            )],
            &[true],
            &[std::env::temp_dir()],
        )
        .expect("complete root rule must compile");
        assert_eq!(
            complete.root_tasks[0].complete_root_rule_index(&complete.rules),
            Some(0)
        );
        assert!(complete.rule_exclusively_owns_root(0, &root));

        let filtered = compile_scan_plan(
            vec![rule(
                "development.filtered",
                root.clone(),
                CleanupCategory::Development,
                MatcherSpec::OlderThanDays(7),
            )],
            &[true],
            &[std::env::temp_dir()],
        )
        .expect("filtered root rule must compile");
        assert_eq!(
            filtered.root_tasks[0].complete_root_rule_index(&filtered.rules),
            None
        );

        let nested = compile_scan_plan(
            vec![
                rule(
                    "development.parent",
                    root.clone(),
                    CleanupCategory::Development,
                    MatcherSpec::All,
                ),
                rule(
                    "development.child",
                    root.join("child"),
                    CleanupCategory::Development,
                    MatcherSpec::All,
                ),
            ],
            &[true, true],
            &[std::env::temp_dir()],
        )
        .expect("nested rules must compile");
        assert_eq!(
            nested.root_tasks[0].complete_root_rule_index(&nested.rules),
            None
        );
        assert!(!nested.rule_exclusively_owns_root(0, &root));
        assert!(nested.rule_exclusively_owns_root(1, &root.join("child")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_identity_ignores_extended_prefixes_and_case() {
        assert!(same_path(
            Path::new(r"C:\Users\developer\Cache"),
            Path::new(r"\\?\c:\users\DEVELOPER\cache")
        ));
        assert!(same_path(
            Path::new(r"\\server\share\Cache"),
            Path::new(r"\\?\UNC\SERVER\SHARE\cache")
        ));
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
