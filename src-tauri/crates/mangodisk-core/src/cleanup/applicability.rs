use std::cmp::Ordering;

use crate::{
    applications::catalog::{ApplicationInventory, ProcessSnapshot},
    cleanup::rules::{ApplicabilityProbe, CompiledRule},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Applicability {
    Applicable,
    NotApplicable,
    Indeterminate,
}

pub(crate) fn cleanup_rule_application_identifiers(probes: &[ApplicabilityProbe]) -> Vec<String> {
    fn collect(probe: &ApplicabilityProbe, identifiers: &mut Vec<String>) {
        match probe {
            ApplicabilityProbe::ApplicationInstalled(values) => identifiers.extend(values.clone()),
            ApplicabilityProbe::ApplicationVersion { identifier, .. } => {
                identifiers.push(identifier.clone());
            }
            ApplicabilityProbe::AnyOf(items) | ApplicabilityProbe::AllOf(items) => {
                for item in items {
                    collect(item, identifiers);
                }
            }
            // A negated application probe describes something that must not
            // own the rule, so it cannot be trusted as application ownership data.
            ApplicabilityProbe::Not(_)
            | ApplicabilityProbe::AnyRootExists
            | ApplicabilityProbe::PathExists(_)
            | ApplicabilityProbe::ExecutableAvailable(_)
            | ApplicabilityProbe::SystemVersion { .. }
            | ApplicabilityProbe::FileSystemIn(_)
            | ApplicabilityProbe::CapabilityAvailable(_)
            | ApplicabilityProbe::ProcessRunning(_) => {}
        }
    }

    let mut identifiers = Vec::new();
    for probe in probes {
        collect(probe, &mut identifiers);
    }
    identifiers.sort_by_key(|value| value.to_ascii_lowercase());
    identifiers.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    identifiers
}

pub(crate) fn evaluate_rule(
    inventory: &ApplicationInventory,
    rule: &CompiledRule,
    process_snapshot: Option<&ProcessSnapshot>,
) -> Applicability {
    evaluate_all(&rule.applicability, rule, inventory, process_snapshot)
}

fn evaluate_all(
    probes: &[ApplicabilityProbe],
    rule: &CompiledRule,
    inventory: &ApplicationInventory,
    process_snapshot: Option<&ProcessSnapshot>,
) -> Applicability {
    combine_all(
        probes
            .iter()
            .map(|probe| evaluate(probe, rule, inventory, process_snapshot)),
    )
}

fn evaluate(
    probe: &ApplicabilityProbe,
    rule: &CompiledRule,
    inventory: &ApplicationInventory,
    process_snapshot: Option<&ProcessSnapshot>,
) -> Applicability {
    match probe {
        ApplicabilityProbe::AnyRootExists => {
            combine_any(rule.roots.iter().map(|root| path_applicability(root)))
        }
        ApplicabilityProbe::PathExists(path) => path_applicability(path),
        ApplicabilityProbe::ApplicationInstalled(identifiers) => known_fact(
            inventory.applications_complete(),
            inventory.has_application(identifiers),
        ),
        ApplicabilityProbe::ExecutableAvailable(names) => known_fact(
            inventory.developer_tools_complete(),
            inventory.has_executable(names),
        ),
        ApplicabilityProbe::ApplicationVersion {
            identifier,
            minimum,
            maximum_exclusive,
        } => inventory.application_versions(identifier).map_or_else(
            || {
                if inventory.has_application(std::slice::from_ref(identifier)) {
                    Applicability::Indeterminate
                } else {
                    known_fact(inventory.applications_complete(), false)
                }
            },
            |versions| {
                combine_any(versions.iter().map(|version| {
                    version_in_range(version, minimum.as_deref(), maximum_exclusive.as_deref())
                }))
            },
        ),
        ApplicabilityProbe::SystemVersion {
            minimum,
            maximum_exclusive,
        } => version_in_range(
            inventory.os_version(),
            minimum.as_deref(),
            maximum_exclusive.as_deref(),
        ),
        ApplicabilityProbe::FileSystemIn(values) => known_fact(
            inventory.filesystem_complete(),
            inventory.has_filesystem_kind(values),
        ),
        ApplicabilityProbe::CapabilityAvailable(values) => known_fact(
            inventory.capabilities_complete(),
            inventory.has_capability(values),
        ),
        ApplicabilityProbe::ProcessRunning(values) => process_snapshot
            .map(|snapshot| known_fact(true, snapshot.contains_any(values)))
            .unwrap_or(Applicability::Indeterminate),
        ApplicabilityProbe::AnyOf(items) => combine_any(
            items
                .iter()
                .map(|item| evaluate(item, rule, inventory, process_snapshot)),
        ),
        ApplicabilityProbe::AllOf(items) => combine_all(
            items
                .iter()
                .map(|item| evaluate(item, rule, inventory, process_snapshot)),
        ),
        ApplicabilityProbe::Not(item) => match evaluate(item, rule, inventory, process_snapshot) {
            Applicability::Applicable => Applicability::NotApplicable,
            Applicability::NotApplicable => Applicability::Applicable,
            Applicability::Indeterminate => Applicability::Indeterminate,
        },
    }
}

pub(crate) fn rule_requires_process(rule: &CompiledRule) -> bool {
    rule.applicability.iter().any(probe_requires_process)
}

fn probe_requires_process(probe: &ApplicabilityProbe) -> bool {
    match probe {
        ApplicabilityProbe::ProcessRunning(_) => true,
        ApplicabilityProbe::AnyOf(items) | ApplicabilityProbe::AllOf(items) => {
            items.iter().any(probe_requires_process)
        }
        ApplicabilityProbe::Not(item) => probe_requires_process(item),
        _ => false,
    }
}

fn combine_all(values: impl Iterator<Item = Applicability>) -> Applicability {
    let mut indeterminate = false;
    for value in values {
        match value {
            Applicability::NotApplicable => return Applicability::NotApplicable,
            Applicability::Indeterminate => indeterminate = true,
            Applicability::Applicable => {}
        }
    }
    if indeterminate {
        Applicability::Indeterminate
    } else {
        Applicability::Applicable
    }
}

fn combine_any(values: impl Iterator<Item = Applicability>) -> Applicability {
    let mut indeterminate = false;
    for value in values {
        match value {
            Applicability::Applicable => return Applicability::Applicable,
            Applicability::Indeterminate => indeterminate = true,
            Applicability::NotApplicable => {}
        }
    }
    if indeterminate {
        Applicability::Indeterminate
    } else {
        Applicability::NotApplicable
    }
}

fn known_fact(complete: bool, value: bool) -> Applicability {
    if value {
        Applicability::Applicable
    } else if complete {
        Applicability::NotApplicable
    } else {
        Applicability::Indeterminate
    }
}

fn path_applicability(path: &std::path::Path) -> Applicability {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Applicability::Applicable,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Applicability::NotApplicable,
        // An inaccessible path is not evidence that it does not exist. Keep the
        // rule eligible so the scanner can report the existing safety skip.
        Err(_) => Applicability::Indeterminate,
    }
}

fn version_in_range(
    actual: &str,
    minimum: Option<&str>,
    maximum_exclusive: Option<&str>,
) -> Applicability {
    if actual.trim().is_empty()
        || actual.eq_ignore_ascii_case("unknown")
        || version_components(actual).is_empty()
    {
        return Applicability::Indeterminate;
    }
    if minimum.is_some_and(|minimum| compare_versions(actual, minimum) == Ordering::Less)
        || maximum_exclusive
            .is_some_and(|maximum| compare_versions(actual, maximum) != Ordering::Less)
    {
        Applicability::NotApplicable
    } else {
        Applicability::Applicable
    }
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let left = version_components(left);
    let right = version_components(right);
    let length = left.len().max(right.len());
    (0..length)
        .map(|index| {
            left.get(index)
                .copied()
                .unwrap_or_default()
                .cmp(&right.get(index).copied().unwrap_or_default())
        })
        .find(|ordering| *ordering != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}

fn version_components(value: &str) -> Vec<u64> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .filter(|component| !component.is_empty())
        .filter_map(|component| component.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_supports_common_system_and_application_versions() {
        assert_eq!(compare_versions("15.7.7", "15.7.6"), Ordering::Greater);
        assert_eq!(
            compare_versions("Google Chrome 126.0.1", "126.0.1"),
            Ordering::Equal
        );
        assert_eq!(
            compare_versions("10.0.26200.0", "10.0.26100"),
            Ordering::Greater
        );
        assert_eq!(
            combine_any(["1.0", "3.0"].into_iter().map(|version| version_in_range(
                version,
                Some("2.0"),
                None
            ))),
            Applicability::Applicable,
            "any matching installed version should make the rule applicable"
        );
    }

    #[test]
    fn incomplete_inventory_does_not_reject_unknown_applications() {
        assert_eq!(known_fact(false, false), Applicability::Indeterminate);
        assert_eq!(known_fact(true, false), Applicability::NotApplicable);
    }

    #[test]
    fn portable_application_paths_can_supplement_the_installed_catalog() {
        assert_eq!(
            combine_any(
                [
                    known_fact(true, false),
                    path_applicability(&std::env::temp_dir()),
                ]
                .into_iter()
            ),
            Applicability::Applicable,
            "existing cache roots must keep portable applications eligible"
        );
    }
}
