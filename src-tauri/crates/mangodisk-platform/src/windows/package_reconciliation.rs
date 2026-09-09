use std::{collections::HashMap, fs, path::Path};

use crate::{ApplicationInventorySource, ApplicationSourceIdentity, InstalledApplication};

use super::{package_sources::PackageSourceFact, path_identity};

/// Reconciles optional package-manager facts with the authoritative Windows
/// registry and AppX catalog. Match evidence is intentionally kept inside this
/// module so presentation deduplication cannot accidentally grant native
/// uninstall authority.
pub(super) fn merge(
    applications: &mut HashMap<String, InstalledApplication>,
    facts: Vec<PackageSourceFact>,
) {
    let mut unmatched_chocolatey = 0_usize;
    let mut unmatched_winget = 0_usize;
    for fact in facts {
        if let Some((key, evidence)) = package_source_match(applications, &fact) {
            if let Some(application) = applications.get_mut(&key) {
                merge_fact(application, &fact, evidence);
            }
            continue;
        }

        // Scoop and Steam expose an authoritative package identity plus an
        // installation directory. Keeping unmatched entries visible improves
        // inventory coverage. Steam remains view-only because its interactive
        // client protocol cannot provide synchronous completion evidence.
        // Chocolatey graph roots carry explicit package-manager authority;
        // dependencies remain hidden. A raw WinGet export is not itself proof
        // that an item is an application.
        match fact.source {
            ApplicationInventorySource::Scoop | ApplicationInventorySource::Steam => {}
            ApplicationInventorySource::Chocolatey => {
                if !fact.surface_when_unmatched {
                    unmatched_chocolatey = unmatched_chocolatey.saturating_add(1);
                    continue;
                }
            }
            ApplicationInventorySource::Winget => {
                unmatched_winget = unmatched_winget.saturating_add(1);
                continue;
            }
            _ => continue,
        }

        let stable_source = inventory_source_code(fact.source);
        let identity = format!(
            "package-source:{stable_source}:{}",
            fact.identifier.to_ascii_lowercase()
        );
        let install_path = fact.install_path.clone();
        applications.insert(
            identity,
            InstalledApplication {
                #[cfg(windows)]
                system_signed: false,
                uninstall_diagnostic: None,
                catalog_identifier: format!(
                    "windows-{stable_source}:{}",
                    fact.identifier.to_ascii_lowercase()
                ),
                source_identities: vec![ApplicationSourceIdentity {
                    source: fact.source,
                    identifier: fact.identifier.clone(),
                }],
                primary_identifier: fact.identifier.clone(),
                identifiers: vec![fact.identifier.clone(), fact.name.clone()],
                name: fact.name,
                version: fact.version,
                publisher: None,
                estimated_bytes: fact.estimated_bytes,
                last_used_at_ms: None,
                installed_at_ms: fact.installed_at_ms,
                icon_path: install_path.clone(),
                bundle_path: install_path,
                executable_paths: Vec::new(),
                uninstall_registration: fact.uninstall_registration,
            },
        );
    }
    if unmatched_chocolatey > 0 {
        log::debug!(
            "windows_package_source_facts_not_surfaced source=chocolatey count={unmatched_chocolatey}"
        );
    }
    if unmatched_winget > 0 {
        log::debug!(
            "windows_package_source_facts_not_surfaced source=winget count={unmatched_winget}"
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MatchEvidence {
    ExactSourceIdentity,
    ExactInstallPath,
    WingetIdentityAndVersion,
    UniqueNameAndVersion,
}

fn package_source_match(
    applications: &HashMap<String, InstalledApplication>,
    fact: &PackageSourceFact,
) -> Option<(String, MatchEvidence)> {
    let exact_source_matches = applications
        .iter()
        .filter(|(_, application)| {
            application.source_identities.iter().any(|identity| {
                identity.source == fact.source
                    && identity.identifier.eq_ignore_ascii_case(&fact.identifier)
            })
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    if exact_source_matches.len() == 1 {
        return exact_source_matches
            .into_iter()
            .next()
            .map(|key| (key, MatchEvidence::ExactSourceIdentity));
    }

    if let Some(install_path) = fact.install_path.as_deref() {
        let exact_path_matches = applications
            .iter()
            .filter(|(_, application)| {
                application
                    .bundle_path
                    .as_deref()
                    .is_some_and(|path| windows_paths_match(path, install_path))
                    || application
                        .executable_paths
                        .iter()
                        .filter_map(|path| path.parent())
                        .any(|path| windows_paths_match(path, install_path))
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        if exact_path_matches.len() == 1 {
            return exact_path_matches
                .into_iter()
                .next()
                .map(|key| (key, MatchEvidence::ExactInstallPath));
        }
    }

    if fact.source == ApplicationInventorySource::Winget {
        let winget_matches = applications
            .iter()
            .filter(|(_, application)| winget_identity_matches(application, fact))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        if winget_matches.len() == 1 {
            return winget_matches
                .into_iter()
                .next()
                .map(|key| (key, MatchEvidence::WingetIdentityAndVersion));
        }
    }

    // Scoop always provides an exact installation directory. Falling back to
    // a display-name match could collapse a current-user package and a global
    // package into the same unrelated registry row, hiding one installation
    // and its scope-specific uninstall capability.
    if fact.source == ApplicationInventorySource::Scoop {
        return None;
    }

    let name_version_matches = applications
        .iter()
        .filter(|(_, application)| {
            !application.source_identities.iter().any(|identity| {
                identity.source == fact.source
                    && !identity.identifier.eq_ignore_ascii_case(&fact.identifier)
            }) && application.name.eq_ignore_ascii_case(&fact.name)
                && versions_match(application.version.as_deref(), fact.version.as_deref())
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    (name_version_matches.len() == 1)
        .then(|| name_version_matches.into_iter().next())
        .flatten()
        .map(|key| (key, MatchEvidence::UniqueNameAndVersion))
}

fn winget_identity_matches(application: &InstalledApplication, fact: &PackageSourceFact) -> bool {
    if !versions_match(application.version.as_deref(), fact.version.as_deref()) {
        return false;
    }
    let segments = fact
        .identifier
        .split('.')
        // The first segment is normally a publisher and is too broad to prove
        // product identity. Numeric architecture/version segments are also
        // intentionally ignored.
        .skip(1)
        .map(normalize_package_name)
        .filter(|segment| segment.chars().count() >= 4)
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return false;
    }
    std::iter::once(application.name.as_str())
        .chain(application.identifiers.iter().map(String::as_str))
        .map(normalize_package_name)
        .any(|name| {
            segments
                .iter()
                .any(|segment| name == *segment || name.contains(segment))
        })
}

fn normalize_package_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn merge_fact(
    application: &mut InstalledApplication,
    fact: &PackageSourceFact,
    evidence: MatchEvidence,
) {
    // A unique name/version match is presentation-only evidence. Persisting a
    // source identity or installation path from that match would turn the next
    // scan into an apparent exact match and could silently escalate it into
    // native uninstall authority.
    let evidence_authorizes_identity = !matches!(evidence, MatchEvidence::UniqueNameAndVersion);
    if evidence_authorizes_identity {
        merge_source_identity(
            &mut application.source_identities,
            ApplicationSourceIdentity {
                source: fact.source,
                identifier: fact.identifier.clone(),
            },
        );
        if !application
            .identifiers
            .iter()
            .any(|identifier| identifier.eq_ignore_ascii_case(&fact.identifier))
        {
            application.identifiers.push(fact.identifier.clone());
        }
    }
    application.estimated_bytes = application.estimated_bytes.max(fact.estimated_bytes);
    if application.version.is_none() {
        application.version.clone_from(&fact.version);
    }
    if application.installed_at_ms.is_none() {
        application.installed_at_ms = fact.installed_at_ms;
    }
    if evidence_authorizes_identity && application.bundle_path.is_none() {
        application.bundle_path.clone_from(&fact.install_path);
    }

    // A package-manager registration can execute code and therefore requires
    // exact source or installation-path evidence. Name/version and WinGet
    // heuristics are sufficient only for read-only catalog deduplication.
    let evidence_authorizes_registration = matches!(
        evidence,
        MatchEvidence::ExactSourceIdentity | MatchEvidence::ExactInstallPath
    );
    if evidence_authorizes_registration && application.uninstall_registration.is_none() {
        application
            .uninstall_registration
            .clone_from(&fact.uninstall_registration);
    }
}

fn merge_source_identity(
    identities: &mut Vec<ApplicationSourceIdentity>,
    candidate: ApplicationSourceIdentity,
) {
    if !identities.iter().any(|identity| {
        identity.source == candidate.source
            && identity
                .identifier
                .eq_ignore_ascii_case(&candidate.identifier)
    }) {
        identities.push(candidate);
    }
}

fn windows_paths_match(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    path_identity::equal(&left, &right)
}

fn versions_match(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.trim().eq_ignore_ascii_case(right.trim()),
        _ => false,
    }
}

const fn inventory_source_code(source: ApplicationInventorySource) -> &'static str {
    match source {
        ApplicationInventorySource::MacosBundle => "macos-bundle",
        ApplicationInventorySource::WindowsRegistry => "registry",
        ApplicationInventorySource::WindowsMsi => "msi",
        ApplicationInventorySource::WindowsAppx => "appx",
        ApplicationInventorySource::Winget => "winget",
        ApplicationInventorySource::Steam => "steam",
        ApplicationInventorySource::Scoop => "scoop",
        ApplicationInventorySource::Chocolatey => "chocolatey",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
    };

    use crate::{
        ApplicationInstallScope, ApplicationInventorySource, ApplicationSourceIdentity,
        ApplicationUninstallRegistration, InstalledApplication, WindowsRegisteredUninstallKind,
        WindowsRegistryView,
    };

    use super::{merge, PackageSourceFact};

    #[test]
    fn exact_path_enriches_without_replacing_existing_authority() {
        let registration = ApplicationUninstallRegistration::WindowsRegistered {
            key_name: "Example.App".to_string(),
            scope: ApplicationInstallScope::CurrentUser,
            registry_view: WindowsRegistryView::Registry64,
            command_kind: WindowsRegisteredUninstallKind::Executable,
            command_digest: "verified-command".to_string(),
            estimated_bytes: 1_024,
        };
        let mut applications = HashMap::from([(
            "current-user:example.app".to_string(),
            application(
                "Example",
                "1.2.3",
                Some(PathBuf::from(
                    r"C:\Users\developer\scoop\apps\example\current",
                )),
                Some(registration.clone()),
            ),
        )]);

        merge(
            &mut applications,
            vec![fact(
                ApplicationInventorySource::Scoop,
                "example",
                "Example",
                "1.2.3",
                Some(PathBuf::from(
                    r"C:\Users\developer\scoop\apps\example\current",
                )),
                None,
            )],
        );

        let application = &applications["current-user:example.app"];
        assert_eq!(application.uninstall_registration, Some(registration));
        assert_eq!(application.estimated_bytes, 2_048);
        assert!(application.source_identities.iter().any(|identity| {
            identity.source == ApplicationInventorySource::Scoop && identity.identifier == "example"
        }));
    }

    #[test]
    fn weak_match_never_escalates_on_later_scans() {
        let mut applications = HashMap::from([(
            "registry-example".to_string(),
            application(
                "Example",
                "1.2.3",
                Some(PathBuf::from(r"C:\Program Files\Example")),
                None,
            ),
        )]);
        let weak_fact = || {
            fact(
                ApplicationInventorySource::Scoop,
                "example",
                "Example",
                "1.2.3",
                Some(PathBuf::from(
                    r"C:\Users\developer\scoop\apps\example\current",
                )),
                Some(scoop_registration()),
            )
        };

        merge(&mut applications, vec![weak_fact()]);
        merge(&mut applications, vec![weak_fact()]);

        let application = &applications["registry-example"];
        assert!(application.uninstall_registration.is_none());
        assert!(!application.source_identities.iter().any(|identity| {
            identity.source == ApplicationInventorySource::Scoop && identity.identifier == "example"
        }));
        assert_eq!(
            application.bundle_path.as_deref(),
            Some(Path::new(r"C:\Program Files\Example"))
        );
    }

    #[test]
    fn exact_install_path_can_grant_package_manager_authority() {
        let install_path = PathBuf::from(r"C:\Users\developer\scoop\apps\example\current");
        let registration = scoop_registration();
        let mut applications = HashMap::from([(
            "registry-example".to_string(),
            application("Example", "1.2.3", Some(install_path.clone()), None),
        )]);

        merge(
            &mut applications,
            vec![fact(
                ApplicationInventorySource::Scoop,
                "example",
                "Example",
                "1.2.3",
                Some(install_path),
                Some(registration.clone()),
            )],
        );

        assert_eq!(
            applications["registry-example"].uninstall_registration,
            Some(registration)
        );
    }

    #[test]
    fn unmatched_policy_keeps_apps_but_hides_chocolatey_dependencies() {
        let mut applications = HashMap::new();
        merge(
            &mut applications,
            vec![
                fact(
                    ApplicationInventorySource::Scoop,
                    "jq",
                    "jq",
                    "1.7.1",
                    Some(PathBuf::from(r"C:\Users\developer\scoop\apps\jq\current")),
                    None,
                ),
                fact(
                    ApplicationInventorySource::Chocolatey,
                    "chocolatey-core.extension",
                    "chocolatey-core.extension",
                    "1.4.0",
                    Some(PathBuf::from(
                        r"C:\ProgramData\chocolatey\lib\chocolatey-core.extension",
                    )),
                    None,
                ),
            ],
        );

        assert_eq!(applications.len(), 1);
        assert_eq!(
            applications
                .values()
                .next()
                .map(|app| app.catalog_identifier.as_str()),
            Some("windows-scoop:jq")
        );
    }

    #[test]
    fn unmatched_chocolatey_graph_root_is_surfaced() {
        let mut package = fact(
            ApplicationInventorySource::Chocolatey,
            "jq",
            "jq",
            "1.8.1",
            Some(PathBuf::from(r"C:\ProgramData\chocolatey\lib\jq")),
            None,
        );
        package.surface_when_unmatched = true;
        let mut applications = HashMap::new();

        merge(&mut applications, vec![package]);

        assert_eq!(applications.len(), 1);
        assert_eq!(
            applications
                .values()
                .next()
                .map(|app| app.catalog_identifier.as_str()),
            Some("windows-chocolatey:jq")
        );
    }

    #[test]
    fn scoop_user_and_machine_packages_keep_distinct_catalog_identity() {
        let mut applications = HashMap::new();
        merge(
            &mut applications,
            vec![
                fact(
                    ApplicationInventorySource::Scoop,
                    "current-user:example",
                    "example",
                    "1.0.0",
                    Some(PathBuf::from(
                        r"C:\Users\developer\scoop\apps\example\current",
                    )),
                    Some(scoop_registration()),
                ),
                fact(
                    ApplicationInventorySource::Scoop,
                    "machine:example",
                    "example",
                    "1.0.0",
                    Some(PathBuf::from(r"C:\ProgramData\scoop\apps\example\current")),
                    None,
                ),
            ],
        );

        assert_eq!(applications.len(), 2);
        assert!(applications.contains_key("package-source:scoop:current-user:example"));
        assert!(applications.contains_key("package-source:scoop:machine:example"));
        assert!(applications["package-source:scoop:current-user:example"]
            .uninstall_registration
            .is_some());
        assert!(applications["package-source:scoop:machine:example"]
            .uninstall_registration
            .is_none());
    }

    #[test]
    fn scoped_scoop_packages_do_not_hide_behind_a_weak_registry_match() {
        let mut applications = HashMap::from([(
            "registry-example".to_string(),
            application("example", "1.0.0", None, None),
        )]);
        merge(
            &mut applications,
            vec![
                fact(
                    ApplicationInventorySource::Scoop,
                    "current-user:example",
                    "example",
                    "1.0.0",
                    Some(PathBuf::from(
                        r"C:\Users\developer\scoop\apps\example\current",
                    )),
                    Some(scoop_registration()),
                ),
                fact(
                    ApplicationInventorySource::Scoop,
                    "machine:example",
                    "example",
                    "1.0.0",
                    Some(PathBuf::from(r"C:\ProgramData\scoop\apps\example\current")),
                    None,
                ),
            ],
        );

        assert_eq!(applications.len(), 3);
        assert!(applications.contains_key("registry-example"));
        assert!(applications.contains_key("package-source:scoop:current-user:example"));
        assert!(applications.contains_key("package-source:scoop:machine:example"));
    }

    #[test]
    fn winget_identity_requires_unique_name_segment_and_exact_version() {
        let mut applications = HashMap::from([
            (
                "visual-studio-code".to_string(),
                application("Microsoft Visual Studio Code (User)", "1.131.0", None, None),
            ),
            (
                "powershell".to_string(),
                application("PowerShell", "7.5.0", None, None),
            ),
        ]);

        merge(
            &mut applications,
            vec![fact(
                ApplicationInventorySource::Winget,
                "Microsoft.VisualStudioCode",
                "Microsoft.VisualStudioCode",
                "1.131.0",
                None,
                None,
            )],
        );

        assert!(applications["visual-studio-code"]
            .source_identities
            .iter()
            .any(|identity| {
                identity.source == ApplicationInventorySource::Winget
                    && identity.identifier == "Microsoft.VisualStudioCode"
            }));
        assert!(!applications["powershell"]
            .source_identities
            .iter()
            .any(|identity| identity.source == ApplicationInventorySource::Winget));
    }

    fn application(
        name: &str,
        version: &str,
        bundle_path: Option<PathBuf>,
        uninstall_registration: Option<ApplicationUninstallRegistration>,
    ) -> InstalledApplication {
        InstalledApplication {
            #[cfg(windows)]
            system_signed: false,
            uninstall_diagnostic: None,
            catalog_identifier: format!("windows-registry:{}", name.to_ascii_lowercase()),
            source_identities: vec![ApplicationSourceIdentity {
                source: ApplicationInventorySource::WindowsRegistry,
                identifier: name.to_string(),
            }],
            primary_identifier: name.to_string(),
            identifiers: vec![name.to_string()],
            name: name.to_string(),
            version: Some(version.to_string()),
            publisher: None,
            estimated_bytes: 1_024,
            last_used_at_ms: None,
            installed_at_ms: None,
            icon_path: None,
            bundle_path,
            executable_paths: Vec::new(),
            uninstall_registration,
        }
    }

    fn fact(
        source: ApplicationInventorySource,
        identifier: &str,
        name: &str,
        version: &str,
        install_path: Option<PathBuf>,
        uninstall_registration: Option<ApplicationUninstallRegistration>,
    ) -> PackageSourceFact {
        PackageSourceFact {
            source,
            identifier: identifier.to_string(),
            name: name.to_string(),
            version: Some(version.to_string()),
            install_path,
            estimated_bytes: 2_048,
            installed_at_ms: Some(123),
            uninstall_registration,
            surface_when_unmatched: matches!(
                source,
                ApplicationInventorySource::Scoop | ApplicationInventorySource::Steam
            ),
        }
    }

    fn scoop_registration() -> ApplicationUninstallRegistration {
        ApplicationUninstallRegistration::WindowsScoop {
            package_name: "example".to_string(),
            scope: ApplicationInstallScope::CurrentUser,
            install_root: PathBuf::from(r"C:\Users\developer\scoop"),
            package_marker_digest: "package-digest".to_string(),
            scoop_script_digest: "script-digest".to_string(),
            estimated_bytes: 2_048,
        }
    }
}
