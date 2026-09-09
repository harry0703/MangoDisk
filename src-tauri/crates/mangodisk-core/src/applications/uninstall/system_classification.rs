//! Classifies presentation evidence without changing uninstall or record-removal capabilities.
//! Keep the rules local to the application domain and version their diagnostic output so a
//! user's missing row can be traced to the exact policy shipped in that application build.

use mangodisk_platform::InstalledApplication;
use serde::Serialize;

use super::models::ApplicationUninstallCandidate;

const RULE_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationSystemKind {
    /// No positive evidence. This does not claim that the application is safe to remove.
    #[default]
    Unclassified,
    WindowsSystemPackage,
    WindowsBuiltinApp,
    WindowsSharedPackage,
    SharedRuntime,
}

impl ApplicationSystemKind {
    pub(super) const fn stable_code(self) -> &'static str {
        match self {
            Self::Unclassified => "unclassified",
            Self::WindowsSystemPackage => "windows_system_package",
            Self::WindowsBuiltinApp => "windows_builtin_app",
            Self::WindowsSharedPackage => "windows_shared_package",
            Self::SharedRuntime => "shared_runtime",
        }
    }
}

pub(super) fn classify(application: &InstalledApplication) -> ApplicationSystemKind {
    #[cfg(windows)]
    {
        classify_windows(
            &application.source_identities,
            &application.name,
            application.publisher.as_deref(),
            application.system_signed,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = application;
        ApplicationSystemKind::Unclassified
    }
}

// Exact package-family identities include the publisher ID. A display name or a Microsoft
// publisher alone cannot distinguish an inbox application from Office, an IDE, or a Store app.
// These are application families, not versioned full names: Store updates keep the same family.
#[cfg(any(windows, test))]
const BUILTIN_APP_FAMILIES: &[&str] = &[
    "Microsoft.WindowsCalculator_8wekyb3d8bbwe",
    "Microsoft.WindowsNotepad_8wekyb3d8bbwe",
    "Microsoft.Windows.Photos_8wekyb3d8bbwe",
    "Microsoft.Paint_8wekyb3d8bbwe",
    "Microsoft.MSPaint_8wekyb3d8bbwe",
    "Microsoft.ScreenSketch_8wekyb3d8bbwe",
    "Microsoft.WindowsCamera_8wekyb3d8bbwe",
    "Microsoft.WindowsSoundRecorder_8wekyb3d8bbwe",
    "Microsoft.WindowsAlarms_8wekyb3d8bbwe",
    "Microsoft.WindowsMaps_8wekyb3d8bbwe",
    "Microsoft.MicrosoftStickyNotes_8wekyb3d8bbwe",
    "Microsoft.People_8wekyb3d8bbwe",
    "microsoft.windowscommunicationsapps_8wekyb3d8bbwe",
    "MicrosoftCorporationII.QuickAssist_8wekyb3d8bbwe",
    "Microsoft.StartExperiencesApp_8wekyb3d8bbwe",
    "Microsoft.WindowsFeedbackHub_8wekyb3d8bbwe",
    "Microsoft.GetHelp_8wekyb3d8bbwe",
    "Microsoft.Getstarted_8wekyb3d8bbwe",
    "Microsoft.YourPhone_8wekyb3d8bbwe",
    "Microsoft.WindowsStore_8wekyb3d8bbwe",
    "Microsoft.StorePurchaseApp_8wekyb3d8bbwe",
    "Microsoft.DesktopAppInstaller_8wekyb3d8bbwe",
    "Microsoft.WindowsTerminal_8wekyb3d8bbwe",
    "Microsoft.Windows.SecHealthUI_cw5n1h2txyewy",
    "Microsoft.SecHealthUI_8wekyb3d8bbwe",
    "MicrosoftWindows.Client.WebExperience_cw5n1h2txyewy",
    "Microsoft.ZuneMusic_8wekyb3d8bbwe",
    "Microsoft.ZuneVideo_8wekyb3d8bbwe",
];

// Media codecs, language packs and Windows integration packages are shared facilities even
// when AppX does not mark them as framework packages. Keep identities exact so unrelated Store
// applications from the same publisher are not swept into the hidden group.
#[cfg(any(windows, test))]
const SHARED_PACKAGE_FAMILIES: &[&str] = &[
    "Microsoft.HEIFImageExtension_8wekyb3d8bbwe",
    "Microsoft.HEVCVideoExtension_8wekyb3d8bbwe",
    "Microsoft.AV1VideoExtension_8wekyb3d8bbwe",
    "Microsoft.AVCEncoderVideoExtension_8wekyb3d8bbwe",
    "Microsoft.VP9VideoExtensions_8wekyb3d8bbwe",
    "Microsoft.MPEG2VideoExtension_8wekyb3d8bbwe",
    "Microsoft.WebMediaExtensions_8wekyb3d8bbwe",
    "Microsoft.WebpImageExtension_8wekyb3d8bbwe",
    "Microsoft.RawImageExtension_8wekyb3d8bbwe",
    "Microsoft.WidgetsPlatformRuntime_8wekyb3d8bbwe",
    "Microsoft.ApplicationCompatibilityEnhancements_8wekyb3d8bbwe",
    "MicrosoftWindows.CrossDevice_cw5n1h2txyewy",
    "Microsoft.Xbox.TCUI_8wekyb3d8bbwe",
    "Microsoft.XboxIdentityProvider_8wekyb3d8bbwe",
    "Microsoft.XboxSpeechToTextOverlay_8wekyb3d8bbwe",
    "Microsoft.XboxGameOverlay_8wekyb3d8bbwe",
    "Microsoft.Winget.Source_8wekyb3d8bbwe",
    "Microsoft.BingSearch_8wekyb3d8bbwe",
    "MicrosoftCorporationII.WindowsSubsystemForLinux_8wekyb3d8bbwe",
];

#[cfg(any(windows, test))]
fn is_shared_package(family: &str) -> bool {
    if SHARED_PACKAGE_FAMILIES
        .iter()
        .any(|known| known.eq_ignore_ascii_case(family))
    {
        return true;
    }
    let family = family.to_ascii_lowercase();
    let Some(name) = family.strip_suffix("_8wekyb3d8bbwe") else {
        return false;
    };
    if is_windows_app_runtime(name) {
        return true;
    }
    name.strip_prefix("microsoft.languageexperiencepack")
        .is_some_and(|language| {
            (2..=35).contains(&language.len())
                && language
                    .split('-')
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric()))
                && language.starts_with(|c: char| c.is_ascii_alphabetic())
        })
}

// Main/Singleton/DDLM supplement the framework and can appear as ordinary removable packages.
// Match their documented identities, not "AppRuntime" display text. Main uses major.minor for
// 1.x and major for 2.x onward; DDLM additionally encodes all four version fields and CPU type.
// Release channels still contain shared runtime dependencies, not developer SDK installations.
#[cfg(any(windows, test))]
fn is_windows_app_runtime(name: &str) -> bool {
    if let Some(version) = name.strip_prefix("microsoftcorporationii.winappruntime.main.") {
        let (version, channel) = version
            .split_once('-')
            .map_or((version, None), |(version, channel)| {
                (version, Some(channel))
            });
        return runtime_version(version, 1, 2)
            && channel.is_none_or(|value| !value.is_empty() && runtime_channel(value));
    }
    if let Some(channel) = name.strip_prefix("microsoftcorporationii.winappruntime.singleton") {
        return channel.is_empty()
            || channel
                .strip_prefix('-')
                .is_some_and(|value| !value.is_empty() && runtime_channel(value));
    }
    if let Some(value) = name.strip_prefix("microsoft.winappruntime.ddlm.") {
        let mut parts = value.split('-');
        let version = parts.next().unwrap_or_default();
        let architecture = parts.next().unwrap_or_default();
        let channel = parts.next();
        return runtime_version(version, 4, 4)
            && matches!(architecture, "x8" | "x6" | "a6")
            && channel.is_none_or(|value| !value.is_empty() && runtime_channel(value))
            && parts.next().is_none();
    }
    false
}

#[cfg(any(windows, test))]
fn runtime_version(value: &str, minimum_fields: usize, maximum_fields: usize) -> bool {
    (minimum_fields..=maximum_fields).contains(&value.split('.').count())
        && value.split('.').all(|part| {
            !part.is_empty()
                && (part.len() == 1 || !part.starts_with('0'))
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && part.parse::<u16>().is_ok()
        })
}

#[cfg(any(windows, test))]
fn runtime_channel(value: &str) -> bool {
    value.is_empty()
        || ["preview", "experimental", "p", "e"].iter().any(|prefix| {
            value.strip_prefix(prefix).is_some_and(|build| {
                (1..=2).contains(&build.len())
                    && build.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
        })
}

#[cfg(any(windows, test))]
fn classify_windows(
    identities: &[mangodisk_platform::ApplicationSourceIdentity],
    name: &str,
    publisher: Option<&str>,
    system_signed: bool,
) -> ApplicationSystemKind {
    use mangodisk_platform::ApplicationInventorySource;
    if system_signed {
        return ApplicationSystemKind::WindowsSystemPackage;
    }
    if identities.iter().any(|identity| {
        identity.source == ApplicationInventorySource::WindowsAppx
            && BUILTIN_APP_FAMILIES
                .iter()
                .any(|family| family.eq_ignore_ascii_case(&identity.identifier))
    }) {
        return ApplicationSystemKind::WindowsBuiltinApp;
    }
    if identities.iter().any(|identity| {
        identity.source == ApplicationInventorySource::WindowsAppx
            && is_shared_package(&identity.identifier)
    }) {
        return ApplicationSystemKind::WindowsSharedPackage;
    }
    // Registry display names are advisory and are never an execution boundary. Limit these
    // legacy runtime rules to Microsoft-published registry/MSI entries and explicit product
    // grammars. Developer SDKs, IDEs and arbitrary "Microsoft"-named applications stay visible.
    if identities.iter().any(|identity| {
        matches!(
            identity.source,
            ApplicationInventorySource::WindowsRegistry | ApplicationInventorySource::WindowsMsi
        )
    }) && publisher.is_some_and(|publisher| {
        publisher
            .trim()
            .eq_ignore_ascii_case("Microsoft Corporation")
    }) && is_shared_runtime(name.trim())
    {
        return ApplicationSystemKind::SharedRuntime;
    }
    ApplicationSystemKind::Unclassified
}

#[cfg(any(windows, test))]
fn is_shared_runtime(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    // SQL CLR Types is redistributed as a dependency independently of the SQL Server product.
    // Accept only the product's year and optional architecture, never arbitrary SQL tools.
    if let Some(version) = name.strip_prefix("microsoft system clr types for sql server ") {
        let year = [" (x64)", " (x86)", " (arm64)"]
            .iter()
            .find_map(|suffix| version.strip_suffix(suffix))
            .unwrap_or(version);
        if year.len() == 4
            && year.bytes().all(|byte| byte.is_ascii_digit())
            && year
                .parse::<u16>()
                .is_ok_and(|year| (2000..=2099).contains(&year))
        {
            return true;
        }
    }
    if name == "microsoft edge webview2 runtime" {
        return true;
    }
    if let Some(versioned) = name.strip_prefix("microsoft visual c++ ") {
        return versioned.starts_with(|character: char| character.is_ascii_digit())
            && [
                " redistributable",
                " minimum runtime",
                " additional runtime",
            ]
            .iter()
            .any(|role| versioned.contains(role));
    }
    [
        "microsoft .net runtime - ",
        "microsoft .net host - ",
        "microsoft .net host fx resolver - ",
        "microsoft windows desktop runtime - ",
        "microsoft asp.net core ",
        "microsoft .net framework ",
    ]
    .iter()
    .any(|prefix| {
        name.strip_prefix(prefix).is_some_and(|versioned| {
            versioned.starts_with(|character: char| character.is_ascii_digit())
                && !["sdk", "targeting", "developer", "preview"]
                    .iter()
                    .any(|excluded| versioned.contains(excluded))
                && (!prefix.contains("asp.net") || versioned.contains("shared framework"))
        })
    })
}

pub(super) fn log_catalog(
    operation_id: u64,
    candidates: &[ApplicationUninstallCandidate],
    include_details: bool,
) {
    let mut system_packages = 0;
    let mut builtin_apps = 0;
    let mut runtimes = 0;
    let mut shared_packages = 0;
    for candidate in candidates {
        match candidate.system_kind {
            ApplicationSystemKind::Unclassified => continue,
            ApplicationSystemKind::WindowsSystemPackage => system_packages += 1,
            ApplicationSystemKind::WindowsBuiltinApp => builtin_apps += 1,
            ApplicationSystemKind::SharedRuntime => runtimes += 1,
            ApplicationSystemKind::WindowsSharedPackage => shared_packages += 1,
        }
        // Full scans keep per-item evidence; execution preflight only needs totals.
        if include_details {
            log::info!(
            "application_system_classified operation_id={} application_id={} rule_version={} reason={}",
            operation_id, candidate.application_id, RULE_VERSION, candidate.system_kind.stable_code()
        );
        }
    }
    log::info!(
        "application_system_classification_summary operation_id={} rule_version={} candidate_count={} system_package_count={} builtin_app_count={} shared_runtime_count={} shared_package_count={} unclassified_count={}",
        operation_id, RULE_VERSION, candidates.len(), system_packages, builtin_apps, runtimes, shared_packages,
        candidates.len() - system_packages - builtin_apps - runtimes - shared_packages
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_platform::{ApplicationInventorySource, ApplicationSourceIdentity};

    fn identity(
        source: ApplicationInventorySource,
        identifier: &str,
    ) -> Vec<ApplicationSourceIdentity> {
        vec![ApplicationSourceIdentity {
            source,
            identifier: identifier.into(),
        }]
    }

    #[test]
    fn builtin_identity_survives_localized_names_and_requires_matching_source_and_publisher_id() {
        let family = "microsoft.windowscalculator_8wekyb3d8bbwe";
        let source = identity(ApplicationInventorySource::WindowsAppx, family);
        assert_eq!(
            classify_windows(&source, "Calculatrice", None, false),
            ApplicationSystemKind::WindowsBuiltinApp
        );
        for source in [
            identity(ApplicationInventorySource::WindowsRegistry, family),
            identity(
                ApplicationInventorySource::WindowsAppx,
                "Microsoft.WindowsCalculator_otherpublisher",
            ),
            identity(
                ApplicationInventorySource::WindowsAppx,
                "Microsoft.OfficeHub_8wekyb3d8bbwe",
            ),
        ] {
            assert_eq!(
                classify_windows(&source, "Calculator", Some("Microsoft Corporation"), false),
                ApplicationSystemKind::Unclassified
            );
        }
    }

    #[test]
    fn shared_package_rules_include_codecs_and_language_packs_without_publisher_wide_matching() {
        for family in [
            "Microsoft.HEIFImageExtension_8wekyb3d8bbwe",
            "Microsoft.LanguageExperiencePackzh-CN_8wekyb3d8bbwe",
            "Microsoft.LanguageExperiencePacken-GB_8wekyb3d8bbwe",
            "Microsoft.Winget.Source_8wekyb3d8bbwe",
        ] {
            assert_eq!(
                classify_windows(
                    &identity(ApplicationInventorySource::WindowsAppx, family),
                    "",
                    None,
                    false
                ),
                ApplicationSystemKind::WindowsSharedPackage
            );
        }
        for family in [
            "Microsoft.LanguageExperiencePack_8wekyb3d8bbwe",
            "Microsoft.LanguageExperiencePacken--GB_8wekyb3d8bbwe",
            "Microsoft.LanguageExperiencePackzh-CN_other",
            "Microsoft.HEIFImageExtension_other",
            "Microsoft.OutlookForWindows_8wekyb3d8bbwe",
        ] {
            assert_eq!(
                classify_windows(
                    &identity(ApplicationInventorySource::WindowsAppx, family),
                    "",
                    None,
                    false
                ),
                ApplicationSystemKind::Unclassified
            );
        }
    }

    #[test]
    fn windows_inventory_gaps_are_classified_by_exact_family_and_source() {
        for (family, expected) in [
            (
                "MicrosoftCorporationII.QuickAssist_8wekyb3d8bbwe",
                ApplicationSystemKind::WindowsBuiltinApp,
            ),
            (
                "microsoft.windowscommunicationsapps_8wekyb3d8bbwe",
                ApplicationSystemKind::WindowsBuiltinApp,
            ),
            (
                "MicrosoftCorporationII.WindowsSubsystemForLinux_8wekyb3d8bbwe",
                ApplicationSystemKind::WindowsSharedPackage,
            ),
            (
                "Microsoft.BingSearch_8wekyb3d8bbwe",
                ApplicationSystemKind::WindowsSharedPackage,
            ),
        ] {
            assert_eq!(
                classify_windows(
                    &identity(ApplicationInventorySource::WindowsAppx, family),
                    "",
                    None,
                    false
                ),
                expected
            );
            assert_eq!(
                classify_windows(
                    &identity(ApplicationInventorySource::WindowsRegistry, family),
                    "",
                    None,
                    false
                ),
                ApplicationSystemKind::Unclassified
            );
            assert_eq!(
                classify_windows(
                    &identity(
                        ApplicationInventorySource::WindowsAppx,
                        &family.replace("8wekyb3d8bbwe", "otherpublisher")
                    ),
                    "",
                    None,
                    false
                ),
                ApplicationSystemKind::Unclassified
            );
        }
    }

    #[test]
    fn app_runtime_package_versions_channels_and_architectures_have_bounded_grammars() {
        for name in [
            "MicrosoftCorporationII.WinAppRuntime.Main.1.8",
            "MicrosoftCorporationII.WinAppRuntime.Main.2",
            "MicrosoftCorporationII.WinAppRuntime.Main.2-preview1",
            "MicrosoftCorporationII.WinAppRuntime.Main.1.2-p1",
            "MicrosoftCorporationII.WinAppRuntime.Singleton",
            "MicrosoftCorporationII.WinAppRuntime.Singleton-e2",
            "Microsoft.WinAppRuntime.DDLM.3000.882.2207.0-x6",
            "Microsoft.WinAppRuntime.DDLM.2.0.0.0-x8-p1",
            "Microsoft.WinAppRuntime.DDLM.2.1.0.0-a6",
        ] {
            let family = format!("{name}_8wekyb3d8bbwe");
            assert_eq!(
                classify_windows(
                    &identity(ApplicationInventorySource::WindowsAppx, &family),
                    "",
                    None,
                    false
                ),
                ApplicationSystemKind::WindowsSharedPackage,
                "{name}"
            );
            assert!(
                !is_shared_package(&format!("{name}_otherpublisher")),
                "{name}"
            );
        }
        for name in [
            "MicrosoftCorporationII.WinAppRuntime.Main",
            "MicrosoftCorporationII.WinAppRuntime.Main.1..8",
            "MicrosoftCorporationII.WinAppRuntime.Main.01.8",
            "MicrosoftCorporationII.WinAppRuntime.Main.65536",
            "MicrosoftCorporationII.WinAppRuntime.Main.2-",
            "MicrosoftCorporationII.WinAppRuntime.Main.2-tools",
            "MicrosoftCorporationII.WinAppRuntime.Main.2-preview123",
            "MicrosoftCorporationII.WinAppRuntime.SingletonTools",
            "MicrosoftCorporationII.WinAppRuntime.Singleton-",
            "Microsoft.WinAppRuntime.DDLM.2.0.0-x6",
            "Microsoft.WinAppRuntime.DDLM.2.0.0.0-x64",
            "Microsoft.WinAppRuntime.DDLM.2.0.0.0-a6-",
            "Microsoft.WinAppRuntime.DDLM.2.0.0.0-a6-p1-extra",
            "Microsoft.OutlookForWindows",
            "Microsoft.Windows.DevHome",
            "MSTeams",
        ] {
            assert!(
                !is_shared_package(&format!("{name}_8wekyb3d8bbwe")),
                "{name}"
            );
        }
    }

    #[test]
    fn native_system_signature_is_positive_evidence_without_a_name_rule() {
        assert_eq!(
            classify_windows(&[], "", None, true),
            ApplicationSystemKind::WindowsSystemPackage
        );
    }

    #[test]
    fn runtime_rules_cover_shared_dependencies_without_hiding_developer_tools() {
        let source = identity(ApplicationInventorySource::WindowsRegistry, "fixture");
        for name in [
            "Microsoft Visual C++ 2015-2022 Redistributable (x64) - 14.44.35211",
            "Microsoft Visual C++ 2022 X64 Minimum Runtime - 14.44.35211",
            "Microsoft Windows Desktop Runtime - 8.0.20 (x64)",
            "Microsoft .NET Runtime - 9.0.9 (x64)",
            "Microsoft ASP.NET Core 8.0.20 - Shared Framework (x64)",
            "Microsoft .NET Framework 4.8",
            "Microsoft Edge WebView2 Runtime",
            "Microsoft System CLR Types for SQL Server 2019",
            "Microsoft System CLR Types for SQL Server 2012 (x64)",
        ] {
            assert_eq!(
                classify_windows(&source, name, Some("Microsoft Corporation"), false),
                ApplicationSystemKind::SharedRuntime,
                "{name}"
            );
            assert_eq!(
                classify_windows(&source, name, Some("Other publisher"), false),
                ApplicationSystemKind::Unclassified,
                "{name}"
            );
        }
        for name in [
            "Microsoft Visual Studio Code",
            "Microsoft Office",
            "Microsoft Edge",
            "Microsoft Teams",
            "Windows Software Development Kit - Windows 10.0.26100.3233",
            "Microsoft Visual C++ Build Tools",
            "Microsoft .NET SDK 9.0.305 (x64)",
            "Microsoft .NET Framework 4.8 Developer Pack",
            "Microsoft ASP.NET Core Module V2",
            "Example Microsoft .NET Runtime - 8.0",
            "Microsoft Visual C++ Runtime Editor",
            "Microsoft System CLR Types for SQL Server Tools",
            "Microsoft System CLR Types for SQL Server 2019 SDK",
            "Microsoft System CLR Types for SQL Server 2019.1",
            "Microsoft System CLR Types for SQL Server 9999",
        ] {
            assert_eq!(
                classify_windows(&source, name, Some("Microsoft Corporation"), false),
                ApplicationSystemKind::Unclassified,
                "{name}"
            );
        }
    }

    #[test]
    fn classification_serialization_has_stable_frontend_values() {
        for (kind, value) in [
            (ApplicationSystemKind::Unclassified, "unclassified"),
            (
                ApplicationSystemKind::WindowsSystemPackage,
                "windowsSystemPackage",
            ),
            (
                ApplicationSystemKind::WindowsBuiltinApp,
                "windowsBuiltinApp",
            ),
            (ApplicationSystemKind::SharedRuntime, "sharedRuntime"),
            (
                ApplicationSystemKind::WindowsSharedPackage,
                "windowsSharedPackage",
            ),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), value);
        }
    }
}
