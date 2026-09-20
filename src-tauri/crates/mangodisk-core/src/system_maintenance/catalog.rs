use super::{SystemMaintenanceCategory, SystemMaintenanceRiskLevel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MaintenanceResource {
    #[cfg(target_os = "macos")]
    AppAssociations,
    Elevation,
    #[cfg(target_os = "macos")]
    FileSystemPermissions,
    #[cfg(any(target_os = "macos", windows))]
    AudioService,
    #[cfg(any(target_os = "macos", windows))]
    Network,
    #[cfg(windows)]
    PerformanceCounters,
    #[cfg(target_os = "macos")]
    Preferences,
    #[cfg(windows)]
    PrintQueue,
    #[cfg(target_os = "linux")]
    PackageFiles,
    #[cfg(any(target_os = "macos", windows))]
    SearchIndex,
    ShellCache,
    #[cfg(target_os = "macos")]
    StartupDisk,
    #[cfg(windows)]
    Store,
    #[cfg(windows)]
    SystemRepair,
    #[cfg(windows)]
    SystemDisk,
    #[cfg(windows)]
    TimeService,
    #[cfg(windows)]
    WindowsUpdate,
}

#[derive(Clone, Copy)]
pub(super) struct MaintenanceDefinition {
    pub id: &'static str,
    pub category: SystemMaintenanceCategory,
    pub risk_level: SystemMaintenanceRiskLevel,
    pub requires_restart: bool,
    pub estimated_duration_seconds: u64,
    pub resources: &'static [MaintenanceResource],
}

const fn task(
    id: &'static str,
    category: SystemMaintenanceCategory,
    risk_level: SystemMaintenanceRiskLevel,
    requires_restart: bool,
    estimated_duration_seconds: u64,
    resources: &'static [MaintenanceResource],
) -> MaintenanceDefinition {
    MaintenanceDefinition {
        id,
        category,
        risk_level,
        requires_restart,
        estimated_duration_seconds,
        resources,
    }
}

#[cfg(target_os = "macos")]
const DEFINITIONS: &[MaintenanceDefinition] = &[
    task(
        "macos.maintenance.spotlight-index",
        SystemMaintenanceCategory::SearchAndInterface,
        SystemMaintenanceRiskLevel::Caution,
        false,
        120,
        &[MaintenanceResource::SearchIndex],
    ),
    task(
        "macos.maintenance.launch-services",
        SystemMaintenanceCategory::SearchAndInterface,
        SystemMaintenanceRiskLevel::Standard,
        false,
        15,
        &[MaintenanceResource::AppAssociations],
    ),
    task(
        "macos.maintenance.quicklook-cache",
        SystemMaintenanceCategory::SearchAndInterface,
        SystemMaintenanceRiskLevel::Standard,
        false,
        10,
        &[MaintenanceResource::ShellCache],
    ),
    task(
        "macos.maintenance.icon-cache",
        SystemMaintenanceCategory::SearchAndInterface,
        SystemMaintenanceRiskLevel::Standard,
        false,
        10,
        &[MaintenanceResource::ShellCache],
    ),
    task(
        "macos.maintenance.finder-service",
        SystemMaintenanceCategory::SearchAndInterface,
        SystemMaintenanceRiskLevel::Caution,
        false,
        10,
        &[MaintenanceResource::ShellCache],
    ),
    task(
        "macos.maintenance.audio-service",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Caution,
        false,
        10,
        &[MaintenanceResource::AudioService],
    ),
    task(
        "macos.maintenance.user-permissions",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Caution,
        false,
        60,
        &[MaintenanceResource::FileSystemPermissions],
    ),
    task(
        "macos.maintenance.legacy-overrides",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Standard,
        false,
        5,
        &[MaintenanceResource::Preferences],
    ),
    task(
        "macos.maintenance.startup-disk",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Standard,
        false,
        120,
        &[MaintenanceResource::StartupDisk],
    ),
    task(
        "macos.maintenance.dns-cache",
        SystemMaintenanceCategory::Network,
        SystemMaintenanceRiskLevel::Standard,
        false,
        5,
        &[MaintenanceResource::Network],
    ),
];

#[cfg(windows)]
const DEFINITIONS: &[MaintenanceDefinition] = &[
    task(
        "windows.maintenance.system-integrity",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Caution,
        false,
        900,
        &[MaintenanceResource::SystemRepair],
    ),
    task(
        "windows.maintenance.search-index",
        SystemMaintenanceCategory::SearchAndInterface,
        SystemMaintenanceRiskLevel::Caution,
        false,
        120,
        &[MaintenanceResource::SearchIndex],
    ),
    task(
        "windows.maintenance.explorer-cache",
        SystemMaintenanceCategory::SearchAndInterface,
        SystemMaintenanceRiskLevel::Standard,
        false,
        15,
        &[MaintenanceResource::ShellCache],
    ),
    task(
        "windows.maintenance.update-components",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Caution,
        false,
        120,
        &[MaintenanceResource::WindowsUpdate],
    ),
    task(
        "windows.maintenance.print-queue",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Caution,
        false,
        30,
        &[MaintenanceResource::PrintQueue],
    ),
    task(
        "windows.maintenance.performance-counters",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Caution,
        false,
        60,
        &[MaintenanceResource::PerformanceCounters],
    ),
    task(
        "windows.maintenance.system-disk",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Standard,
        false,
        120,
        &[MaintenanceResource::SystemDisk],
    ),
    task(
        "windows.maintenance.audio-service",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Caution,
        false,
        15,
        &[MaintenanceResource::AudioService],
    ),
    task(
        "windows.maintenance.store-cache",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Standard,
        false,
        15,
        &[MaintenanceResource::Store],
    ),
    task(
        "windows.maintenance.dns-cache",
        SystemMaintenanceCategory::Network,
        SystemMaintenanceRiskLevel::Standard,
        false,
        5,
        &[MaintenanceResource::Network],
    ),
    task(
        "windows.maintenance.time-sync",
        SystemMaintenanceCategory::Network,
        SystemMaintenanceRiskLevel::Standard,
        false,
        15,
        &[MaintenanceResource::TimeService],
    ),
];

#[cfg(target_os = "linux")]
const DEFINITIONS: &[MaintenanceDefinition] = &[
    task(
        "linux.maintenance.font-cache",
        SystemMaintenanceCategory::SearchAndInterface,
        SystemMaintenanceRiskLevel::Standard,
        false,
        30,
        &[MaintenanceResource::ShellCache],
    ),
    task(
        "linux.maintenance.package-integrity",
        SystemMaintenanceCategory::SystemRepair,
        SystemMaintenanceRiskLevel::Standard,
        false,
        120,
        &[MaintenanceResource::PackageFiles],
    ),
];

pub(super) fn definitions() -> &'static [MaintenanceDefinition] {
    DEFINITIONS
}

pub(super) fn definition(id: &str) -> Option<&'static MaintenanceDefinition> {
    DEFINITIONS.iter().find(|definition| definition.id == id)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn task_catalog_has_stable_unique_identifiers() {
        let Some(first) = DEFINITIONS.first() else {
            return;
        };
        assert!(first.id.contains(".maintenance."));
        let mut identifiers = BTreeSet::new();
        for definition in DEFINITIONS {
            assert!(identifiers.insert(definition.id));
            assert!(definition.id.contains(".maintenance."));
            assert!(definition.estimated_duration_seconds > 0);
            assert!(!definition.resources.is_empty());
        }
    }

    #[test]
    fn task_identifiers_match_the_compiled_platform() {
        let Some(_first) = DEFINITIONS.first() else {
            return;
        };
        #[cfg(target_os = "macos")]
        let prefix = "macos.maintenance.";
        #[cfg(windows)]
        let prefix = "windows.maintenance.";
        #[cfg(target_os = "linux")]
        let prefix = "linux.maintenance.";

        for definition in DEFINITIONS {
            assert!(definition.id.starts_with(prefix));
        }
    }
}
