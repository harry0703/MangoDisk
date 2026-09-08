use super::shortcut_overlay;
use std::{collections::BTreeMap, io};

use windows_sys::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
};
use winreg::{enums::*, RegKey};

use crate::{
    PlatformCancellation, PlatformError, PlatformErrorCode, PlatformResult,
    PlatformSystemSettingChangeRequest, PlatformSystemSettingChangeResult,
    PlatformSystemSettingDiagnosticCode, PlatformSystemSettingSnapshot, PlatformSystemSettingState,
    PlatformSystemSettingValue,
};

#[derive(Clone, Copy)]
enum ValueKind {
    Dword,
    Text,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RegistryRoot {
    CurrentUser,
    LocalMachine,
}

#[derive(Clone, Copy)]
enum BuildApplicability {
    Any,
    Before(u32),
    AtLeast(u32),
    Range {
        inclusive_start: u32,
        exclusive_end: u32,
    },
}

#[derive(Clone, Copy)]
struct SettingDefinition {
    id: &'static str,
    root: RegistryRoot,
    requires_elevation: bool,
    subkey: &'static str,
    value_name: &'static str,
    kind: ValueKind,
    applicability: BuildApplicability,
    requires_existing_key: bool,
    delete_tree_subkey: Option<&'static str>,
    composite_text_key: Option<&'static str>,
}

const SETTINGS: &[SettingDefinition] = &[
    machine_setting(
        shortcut_overlay::SETTING_ID,
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Shell Icons",
        "29",
        ValueKind::Text,
    ),
    setting(
        "windows.explorer.show-file-extensions",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "HideFileExt",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.show-hidden-files",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "Hidden",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.launch-this-pc",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "LaunchTo",
        ValueKind::Dword,
    ),
    at_least_build(
        setting(
            "windows.explorer.compact-mode",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "UseCompactMode",
            ValueKind::Dword,
        ),
        22_000,
    ),
    machine_setting(
        "windows.explorer.remove-cast-to-device",
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Blocked",
        "{7AD84985-87B4-4a16-BE58-8B72A5B390F7}",
        ValueKind::Text,
    ),
    setting(
        "windows.explorer.show-full-path",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\CabinetState",
        "FullPath",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.show-item-checkboxes",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "AutoCheckSelect",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.hide-recent-files",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer",
        "ShowRecent",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.hide-frequent-folders",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer",
        "ShowFrequent",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.enable-auto-suggest",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\AutoComplete",
        "AutoSuggest",
        ValueKind::Text,
    ),
    setting(
        "windows.explorer.disable-aero-shake",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "DisallowShaking",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.show-operation-details",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\OperationStatusManager",
        "EnthusiastMode",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.hide-sync-notifications",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "ShowSyncProviderNotifications",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.show-status-bar",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "ShowStatusBar",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.separate-process",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "SeparateProcess",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.restore-folders-at-login",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "PersistBrowsers",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.disable-sharing-wizard",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "SharingWizardOn",
        ValueKind::Dword,
    ),
    elevated_user_setting(
        "windows.explorer.confirm-file-delete",
        r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer",
        "ConfirmFileDelete",
        ValueKind::Dword,
    ),
    setting(
        "windows.explorer.use-manual-default-printer",
        r"Software\Microsoft\Windows NT\CurrentVersion\Windows",
        "LegacyDefaultPrinterMode",
        ValueKind::Dword,
    ),
    at_least_build(
        delete_tree_on_missing(
            setting(
                "windows.explorer.classic-context-menu",
                r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32",
                "",
                ValueKind::Text,
            ),
            r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}",
        ),
        22_000,
    ),
    setting(
        "windows.taskbar.show-seconds",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "ShowSecondsInSystemClock",
        ValueKind::Dword,
    ),
    setting(
        "windows.taskbar.hide-task-view",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "ShowTaskViewButton",
        ValueKind::Dword,
    ),
    build_range(
        machine_setting(
            "windows.taskbar.hide-widgets",
            r"SOFTWARE\Policies\Microsoft\Dsh",
            "AllowNewsAndInterests",
            ValueKind::Dword,
        ),
        22_000,
        26_100,
    ),
    at_least_build(
        machine_setting(
            "windows.taskbar.disable-widgets-board",
            r"SOFTWARE\Policies\Microsoft\Dsh",
            "DisableWidgetsBoard",
            ValueKind::Dword,
        ),
        26_100,
    ),
    at_least_build(
        setting(
            "windows.taskbar.enable-end-task",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced\TaskbarDeveloperSettings",
            "TaskbarEndTask",
            ValueKind::Dword,
        ),
        22_000,
    ),
    before_build(
        setting(
            "windows.taskbar.hide-search",
            r"Software\Microsoft\Windows\CurrentVersion\Search",
            "SearchboxTaskbarMode",
            ValueKind::Dword,
        ),
        26_100,
    ),
    at_least_build(
        machine_setting(
            "windows.taskbar.hide-search-policy",
            r"SOFTWARE\Policies\Microsoft\Windows\Windows Search",
            "ConfigureSearchOnTaskbarMode",
            ValueKind::Dword,
        ),
        26_100,
    ),
    at_least_build(
        setting(
            "windows.taskbar.align-left",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "TaskbarAl",
            ValueKind::Dword,
        ),
        22_000,
    ),
    setting(
        "windows.taskbar.show-all-tray-icons",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer",
        "EnableAutoTray",
        ValueKind::Dword,
    ),
    setting(
        "windows.taskbar.hide-badges",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "TaskbarBadges",
        ValueKind::Dword,
    ),
    at_least_build(
        setting(
            "windows.taskbar.disable-flashing",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "TaskbarFlashing",
            ValueKind::Dword,
        ),
        22_000,
    ),
    at_least_build(
        setting(
            "windows.taskbar.hide-window-sharing",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "TaskbarSn",
            ValueKind::Dword,
        ),
        22_000,
    ),
    at_least_build(
        setting(
            "windows.taskbar.show-desktop-corner",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "TaskbarSd",
            ValueKind::Dword,
        ),
        22_000,
    ),
    before_build(
        machine_setting(
            "windows.taskbar.hide-weather",
            r"SOFTWARE\Policies\Microsoft\Windows\Windows Feeds",
            "EnableFeeds",
            ValueKind::Dword,
        ),
        22_000,
    ),
    at_least_build(
        setting(
            "windows.taskbar.hide-chat",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "TaskbarMn",
            ValueKind::Dword,
        ),
        22_000,
    ),
    build_range(
        setting(
            "windows.taskbar.hide-copilot",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "ShowCopilotButton",
            ValueKind::Dword,
        ),
        22_621,
        26_100,
    ),
    setting(
        "windows.taskbar.disable-animations",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "TaskbarAnimations",
        ValueKind::Dword,
    ),
    setting(
        "windows.taskbar.show-on-all-displays",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "MMTaskbarEnabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.desktop.prefer-performance-effects",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\VisualEffects",
        "VisualFXSetting",
        ValueKind::Dword,
    ),
    at_least_build(
        setting(
            "windows.windowing.disable-snap-assist",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "EnableSnapAssistFlyout",
            ValueKind::Dword,
        ),
        22_000,
    ),
    setting(
        "windows.personalization.dark-apps",
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "AppsUseLightTheme",
        ValueKind::Dword,
    ),
    setting(
        "windows.personalization.dark-system",
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "SystemUsesLightTheme",
        ValueKind::Dword,
    ),
    setting(
        "windows.personalization.disable-transparency",
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "EnableTransparency",
        ValueKind::Dword,
    ),
    setting(
        "windows.gaming.enable-game-mode",
        r"Software\Microsoft\GameBar",
        "AutoGameModeEnabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.gaming.disable-capture",
        r"Software\Microsoft\Windows\CurrentVersion\GameDVR",
        "AppCaptureEnabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.gaming.disable-game-dvr",
        r"System\GameConfigStore",
        "GameDVR_Enabled",
        ValueKind::Dword,
    ),
    at_least_build(
        composite_text_setting(
            setting(
                "windows.gaming.optimize-windowed-games",
                r"Software\Microsoft\DirectX\UserGpuPreferences",
                "DirectXUserGlobalSettings",
                ValueKind::Text,
            ),
            "SwapEffectUpgradeEnable",
        ),
        22_000,
    ),
    setting(
        "windows.gaming.disable-game-bar-controller",
        r"Software\Microsoft\GameBar",
        "UseNexusForGameBarEnabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.gaming.disable-game-bar-tips",
        r"Software\Microsoft\GameBar",
        "ShowStartupPanel",
        ValueKind::Dword,
    ),
    at_least_build(
        setting(
            "windows.gaming.disable-dynamic-lighting",
            r"Software\Microsoft\Lighting",
            "AmbientLightingEnabled",
            ValueKind::Dword,
        ),
        22_000,
    ),
    at_least_build(
        setting(
            "windows.gaming.disable-app-lighting-control",
            r"Software\Microsoft\Lighting",
            "ControlledByForegroundApp",
            ValueKind::Dword,
        ),
        22_000,
    ),
    setting(
        "windows.performance.disable-background-store-apps",
        r"Software\Microsoft\Windows\CurrentVersion\BackgroundAccessApplications",
        "GlobalUserDisabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.privacy.disable-advertising-id",
        r"Software\Microsoft\Windows\CurrentVersion\AdvertisingInfo",
        "Enabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.privacy.disable-tailored-experiences",
        r"Software\Microsoft\Windows\CurrentVersion\Privacy",
        "TailoredExperiencesWithDiagnosticDataEnabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.privacy.disable-activity-publishing",
        r"Software\Microsoft\Windows\CurrentVersion\Privacy",
        "PublishUserActivities",
        ValueKind::Dword,
    ),
    setting(
        "windows.privacy.disable-activity-upload",
        r"Software\Microsoft\Windows\CurrentVersion\Privacy",
        "UploadUserActivities",
        ValueKind::Dword,
    ),
    setting(
        "windows.privacy.disable-input-personalization",
        r"Software\Microsoft\InputPersonalization",
        "RestrictImplicitTextCollection",
        ValueKind::Dword,
    ),
    setting(
        "windows.privacy.disable-ink-personalization",
        r"Software\Microsoft\InputPersonalization",
        "RestrictImplicitInkCollection",
        ValueKind::Dword,
    ),
    setting(
        "windows.privacy.disable-feedback-requests",
        r"Software\Microsoft\Siuf\Rules",
        "NumberOfSIUFInPeriod",
        ValueKind::Dword,
    ),
    setting(
        "windows.privacy.disable-nearby-sharing",
        r"Software\Microsoft\Windows\CurrentVersion\CDP",
        "NearShareChannelUserAuthzPolicy",
        ValueKind::Dword,
    ),
    setting(
        "windows.privacy.disable-cross-device-experiences",
        r"Software\Microsoft\Windows\CurrentVersion\CDP",
        "RomeSdkChannelUserAuthzPolicy",
        ValueKind::Dword,
    ),
    setting(
        "windows.privacy.disable-language-list-sharing",
        r"Control Panel\International\User Profile",
        "HttpAcceptLanguageOptOut",
        ValueKind::Dword,
    ),
    setting(
        "windows.clipboard.disable-history",
        r"Software\Microsoft\Clipboard",
        "EnableClipboardHistory",
        ValueKind::Dword,
    ),
    setting(
        "windows.content.disable-silent-app-installs",
        r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
        "SilentInstalledAppsEnabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.content.disable-suggestions",
        r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
        "SoftLandingEnabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.content.disable-lock-screen-tips",
        r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
        "RotatingLockScreenOverlayEnabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.content.disable-welcome-experience",
        r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
        "SubscribedContent-310093Enabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.content.disable-usage-tips",
        r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
        "SubscribedContent-338389Enabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.content.disable-notification-suggestions",
        r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
        "SystemPaneSuggestionsEnabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.content.disable-device-setup-suggestions",
        r"Software\Microsoft\Windows\CurrentVersion\UserProfileEngagement",
        "ScoobeSystemSettingEnabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.content.disable-spotlight",
        r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
        "RotatingLockScreenEnabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.content.disable-phone-link-suggestions",
        r"Software\Microsoft\Windows\CurrentVersion\Mobility",
        "OptedIn",
        ValueKind::Dword,
    ),
    setting(
        "windows.content.disable-service-suggestions",
        r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\Windows.SystemToast.Suggested",
        "Enabled",
        ValueKind::Dword,
    ),
    setting(
        "windows.content.disable-preinstalled-app-suggestions",
        r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
        "PreInstalledAppsEverEnabled",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.search.disable-highlights",
        r"SOFTWARE\Policies\Microsoft\Windows\Windows Search",
        "EnableDynamicContentInWSB",
        ValueKind::Dword,
    ),
    at_least_build(
        setting(
            "windows.start.disable-recommendations",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "Start_IrisRecommendations",
            ValueKind::Dword,
        ),
        22_000,
    ),
    at_least_build(
        setting(
            "windows.start.disable-account-notifications",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "Start_AccountNotifications",
            ValueKind::Dword,
        ),
        22_000,
    ),
    setting(
        "windows.start.hide-recently-added-apps",
        r"Software\Microsoft\Windows\CurrentVersion\Start",
        "ShowRecentList",
        ValueKind::Dword,
    ),
    at_least_build(
        setting(
            "windows.start.hide-most-used-apps",
            r"Software\Microsoft\Windows\CurrentVersion\Start",
            "ShowFrequentList",
            ValueKind::Dword,
        ),
        22_000,
    ),
    setting(
        "windows.start.hide-recent-items",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "Start_TrackDocs",
        ValueKind::Dword,
    ),
    // Policy hives can grant the interactive user read-only access even below HKCU. Using the
    // machine policy location makes the privilege requirement deterministic and routes these
    // writes through MangoDisk's allowlisted elevated helper instead of failing mid-batch.
    machine_setting(
        "windows.edge.disable-sidebar",
        r"SOFTWARE\Policies\Microsoft\Edge",
        "HubsSidebarEnabled",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.edge.disable-personalization",
        r"SOFTWARE\Policies\Microsoft\Edge",
        "PersonalizationReportingEnabled",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.edge.disable-recommendations",
        r"SOFTWARE\Policies\Microsoft\Edge",
        "SpotlightExperiencesAndRecommendationsEnabled",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.edge.limit-diagnostic-data",
        r"SOFTWARE\Policies\Microsoft\Edge",
        "DiagnosticData",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.edge.disable-web-widget",
        r"SOFTWARE\Policies\Microsoft\Edge",
        "WebWidgetAllowed",
        ValueKind::Dword,
    ),
    elevated_user_setting(
        "windows.office.disable-optional-telemetry",
        r"Software\Policies\Microsoft\Office\16.0\Common\Privacy",
        "SendTelemetry",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.firefox.disable-telemetry",
        r"SOFTWARE\Policies\Mozilla\Firefox",
        "DisableTelemetry",
        ValueKind::Dword,
    ),
    at_least_build(
        machine_setting(
            "windows.ai.disable-recall-snapshots",
            r"SOFTWARE\Policies\Microsoft\Windows\WindowsAI",
            "DisableAIDataAnalysis",
            ValueKind::Dword,
        ),
        26_100,
    ),
    at_least_build(
        machine_setting(
            "windows.ai.disable-copilot",
            r"SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot",
            "TurnOffWindowsCopilot",
            ValueKind::Dword,
        ),
        22_000,
    ),
    setting(
        "windows.typing.disable-autocorrect",
        r"Software\Microsoft\TabletTip\1.7",
        "EnableAutocorrection",
        ValueKind::Dword,
    ),
    setting(
        "windows.typing.disable-spellcheck",
        r"Software\Microsoft\TabletTip\1.7",
        "EnableSpellchecking",
        ValueKind::Dword,
    ),
    setting(
        "windows.typing.disable-text-prediction",
        r"Software\Microsoft\TabletTip\1.7",
        "EnableTextPrediction",
        ValueKind::Dword,
    ),
    setting(
        "windows.typing.disable-double-space-period",
        r"Software\Microsoft\TabletTip\1.7",
        "EnableDoubleTapSpace",
        ValueKind::Dword,
    ),
    setting(
        "windows.accessibility.disable-sticky-keys-shortcut",
        r"Control Panel\Accessibility\StickyKeys",
        "Flags",
        ValueKind::Text,
    ),
    setting(
        "windows.accessibility.disable-filter-keys-shortcut",
        r"Control Panel\Accessibility\Keyboard Response",
        "Flags",
        ValueKind::Text,
    ),
    setting(
        "windows.accessibility.disable-toggle-keys-shortcut",
        r"Control Panel\Accessibility\ToggleKeys",
        "Flags",
        ValueKind::Text,
    ),
    machine_setting(
        "windows.search.disable-web-suggestions",
        r"SOFTWARE\Policies\Microsoft\Windows\Explorer",
        "DisableSearchBoxSuggestions",
        ValueKind::Dword,
    ),
    setting(
        "windows.storage.enable-storage-sense",
        r"Software\Microsoft\Windows\CurrentVersion\StorageSense\Parameters\StoragePolicy",
        "01",
        ValueKind::Dword,
    ),
    setting(
        "windows.desktop.reduce-menu-delay",
        r"Control Panel\Desktop",
        "MenuShowDelay",
        ValueKind::Text,
    ),
    setting(
        "windows.desktop.reduce-hover-delay",
        r"Control Panel\Mouse",
        "MouseHoverTime",
        ValueKind::Text,
    ),
    machine_setting(
        "windows.performance.reduce-crash-dump",
        r"SYSTEM\CurrentControlSet\Control\CrashControl",
        "CrashDumpEnabled",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.privacy.disable-remote-assistance",
        r"SYSTEM\CurrentControlSet\Control\Remote Assistance",
        "fAllowToGetHelp",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.performance.reduce-service-shutdown-timeout",
        r"SYSTEM\CurrentControlSet\Control",
        "WaitToKillServiceTimeout",
        ValueKind::Text,
    ),
    machine_setting(
        "windows.performance.disable-network-throttling",
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile",
        "NetworkThrottlingIndex",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.performance.remove-reserved-bandwidth",
        r"SOFTWARE\Policies\Microsoft\Windows\Psched",
        "NonBestEffortLimit",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.performance.multimedia-responsiveness",
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile",
        "SystemResponsiveness",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.performance.multimedia-no-lazy-mode",
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile",
        "NoLazyMode",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.performance.multimedia-always-on",
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile",
        "AlwaysOn",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.gaming.enable-hardware-gpu-scheduling",
        r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers",
        "HwSchMode",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.compatibility.disable-camera-frame-server",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows Media Foundation",
        "EnableFrameServerMode",
        ValueKind::Dword,
    ),
    existing_machine_setting(
        "windows.services.print-spooler-manual",
        r"SYSTEM\CurrentControlSet\Services\Spooler",
        "Start",
        ValueKind::Dword,
    ),
    existing_machine_setting(
        "windows.services.disable-sysmain",
        r"SYSTEM\CurrentControlSet\Services\SysMain",
        "Start",
        ValueKind::Dword,
    ),
    existing_machine_setting(
        "windows.services.disable-compatibility-assistant",
        r"SYSTEM\CurrentControlSet\Services\PcaSvc",
        "Start",
        ValueKind::Dword,
    ),
    existing_machine_setting(
        "windows.services.disable-search-indexing",
        r"SYSTEM\CurrentControlSet\Services\WSearch",
        "Start",
        ValueKind::Dword,
    ),
    existing_machine_setting(
        "windows.services.disable-diagnostic-tracking",
        r"SYSTEM\CurrentControlSet\Services\DiagTrack",
        "Start",
        ValueKind::Dword,
    ),
    existing_machine_setting(
        "windows.services.disable-error-reporting",
        r"SYSTEM\CurrentControlSet\Services\WerSvc",
        "Start",
        ValueKind::Dword,
    ),
    existing_machine_setting(
        "windows.services.disable-sensors",
        r"SYSTEM\CurrentControlSet\Services\SensrSvc",
        "Start",
        ValueKind::Dword,
    ),
    existing_machine_setting(
        "windows.services.disable-insider",
        r"SYSTEM\CurrentControlSet\Services\wisvc",
        "Start",
        ValueKind::Dword,
    ),
    existing_machine_setting(
        "windows.services.disable-xbox-auth",
        r"SYSTEM\CurrentControlSet\Services\XblAuthManager",
        "Start",
        ValueKind::Dword,
    ),
    existing_machine_setting(
        "windows.services.disable-fax",
        r"SYSTEM\CurrentControlSet\Services\Fax",
        "Start",
        ValueKind::Dword,
    ),
    existing_machine_setting(
        "windows.services.disable-media-player-sharing",
        r"SYSTEM\CurrentControlSet\Services\WMPNetworkSvc",
        "Start",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.security.disable-system-restore",
        r"SOFTWARE\Policies\Microsoft\Windows NT\SystemRestore",
        "DisableSR",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.security.disable-defender",
        r"SOFTWARE\Policies\Microsoft\Windows Defender",
        "DisableAntiVirus",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.security.disable-smartscreen",
        r"SOFTWARE\Policies\Microsoft\Windows\System",
        "EnableSmartScreen",
        ValueKind::Dword,
    ),
    elevated_user_setting(
        "windows.security.disable-autorun",
        r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer",
        "NoDriveTypeAutoRun",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.network.disable-llmnr",
        r"SOFTWARE\Policies\Microsoft\Windows NT\DNSClient",
        "EnableMulticast",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.network.disable-smb1",
        r"SYSTEM\CurrentControlSet\Services\LanmanServer\Parameters",
        "SMB1",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.network.disable-smb2",
        r"SYSTEM\CurrentControlSet\Services\LanmanServer\Parameters",
        "SMB2",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.security.disable-vbs",
        r"SYSTEM\CurrentControlSet\Control\DeviceGuard",
        "EnableVirtualizationBasedSecurity",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.storage.disable-ntfs-last-access",
        r"SYSTEM\CurrentControlSet\Control\FileSystem",
        "NtfsDisableLastAccessUpdate",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.update.notify-before-download",
        r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU",
        "AUOptions",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.update.disable-peer-sharing",
        r"SOFTWARE\Policies\Microsoft\Windows\DeliveryOptimization",
        "DODownloadMode",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.update.disable-preview-builds",
        r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate",
        "ManagePreviewBuilds",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.update.prevent-restart-when-logged-on",
        r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU",
        "NoAutoRebootWithLoggedOnUsers",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.update.enable-microsoft-product-updates",
        r"SOFTWARE\Microsoft\WindowsUpdate\UX\Settings",
        "AllowMUUpdateService",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.update.enable-restart-notifications",
        r"SOFTWARE\Microsoft\WindowsUpdate\UX\Settings",
        "RestartNotificationsAllowed2",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.update.disable-automatic-updates",
        r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU",
        "NoAutoUpdate",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.privacy.limit-diagnostic-data",
        r"SOFTWARE\Policies\Microsoft\Windows\DataCollection",
        "AllowTelemetry",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.update.exclude-driver-updates",
        r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate",
        "ExcludeWUDriversInQualityUpdate",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.update.disable-store-auto-updates",
        r"SOFTWARE\Policies\Microsoft\WindowsStore",
        "AutoDownload",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.cloud.disable-onedrive-sync",
        r"SOFTWARE\Policies\Microsoft\Windows\OneDrive",
        "DisableFileSyncNGSC",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.search.disable-cortana-policy",
        r"SOFTWARE\Policies\Microsoft\Windows\Windows Search",
        "AllowCortana",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.input.disable-windows-ink",
        r"SOFTWARE\Policies\Microsoft\WindowsInkWorkspace",
        "AllowWindowsInkWorkspace",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.clipboard.disable-cross-device-policy",
        r"SOFTWARE\Policies\Microsoft\Windows\System",
        "AllowCrossDeviceClipboard",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.filesystem.enable-long-paths",
        r"SYSTEM\CurrentControlSet\Control\FileSystem",
        "LongPathsEnabled",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.setup.bypass-hardware-checks",
        r"SYSTEM\Setup\MoSetup",
        "AllowUpgradesWithUnsupportedTPMOrCPU",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.time.use-utc-hardware-clock",
        r"SYSTEM\CurrentControlSet\Control\TimeZoneInformation",
        "RealTimeIsUniversal",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.power.disable-modern-standby",
        r"SYSTEM\CurrentControlSet\Control\Power",
        "PlatformAoAcOverride",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.power.disable-hibernation",
        r"SYSTEM\CurrentControlSet\Control\Power",
        "HibernateEnabled",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.recovery.enable-registry-backups",
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Configuration Manager",
        "EnablePeriodicBackup",
        ValueKind::Dword,
    ),
    machine_setting(
        "windows.login.enable-verbose-status",
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System",
        "VerboseStatus",
        ValueKind::Dword,
    ),
];

const fn setting(
    id: &'static str,
    subkey: &'static str,
    value_name: &'static str,
    kind: ValueKind,
) -> SettingDefinition {
    SettingDefinition {
        id,
        root: RegistryRoot::CurrentUser,
        requires_elevation: false,
        subkey,
        value_name,
        kind,
        applicability: BuildApplicability::Any,
        requires_existing_key: false,
        delete_tree_subkey: None,
        composite_text_key: None,
    }
}

const fn machine_setting(
    id: &'static str,
    subkey: &'static str,
    value_name: &'static str,
    kind: ValueKind,
) -> SettingDefinition {
    SettingDefinition {
        id,
        root: RegistryRoot::LocalMachine,
        requires_elevation: true,
        subkey,
        value_name,
        kind,
        applicability: BuildApplicability::Any,
        requires_existing_key: false,
        delete_tree_subkey: None,
        composite_text_key: None,
    }
}

/// Marks a user-scoped policy that Windows exposes below a read-only HKCU policy parent.
///
/// The elevated helper still writes the current user's hive; the flag only separates privilege
/// routing from registry scope so user-only ADMX policies are not incorrectly moved to HKLM.
const fn elevated_user_setting(
    id: &'static str,
    subkey: &'static str,
    value_name: &'static str,
    kind: ValueKind,
) -> SettingDefinition {
    let mut definition = setting(id, subkey, value_name, kind);
    definition.requires_elevation = true;
    definition
}

const fn delete_tree_on_missing(
    mut definition: SettingDefinition,
    tree_subkey: &'static str,
) -> SettingDefinition {
    definition.delete_tree_subkey = Some(tree_subkey);
    definition
}

const fn composite_text_setting(
    mut definition: SettingDefinition,
    composite_text_key: &'static str,
) -> SettingDefinition {
    definition.composite_text_key = Some(composite_text_key);
    definition
}

const fn existing_machine_setting(
    id: &'static str,
    subkey: &'static str,
    value_name: &'static str,
    kind: ValueKind,
) -> SettingDefinition {
    let mut definition = machine_setting(id, subkey, value_name, kind);
    definition.requires_existing_key = true;
    definition
}

const fn before_build(
    mut definition: SettingDefinition,
    exclusive_build: u32,
) -> SettingDefinition {
    definition.applicability = BuildApplicability::Before(exclusive_build);
    definition
}

const fn at_least_build(
    mut definition: SettingDefinition,
    inclusive_build: u32,
) -> SettingDefinition {
    definition.applicability = BuildApplicability::AtLeast(inclusive_build);
    definition
}

const fn build_range(
    mut definition: SettingDefinition,
    inclusive_start: u32,
    exclusive_end: u32,
) -> SettingDefinition {
    definition.applicability = BuildApplicability::Range {
        inclusive_start,
        exclusive_end,
    };
    definition
}

pub(crate) fn scan(
    setting_ids: &[&str],
    cancellation: &PlatformCancellation,
) -> PlatformResult<Vec<PlatformSystemSettingState>> {
    let mut states = Vec::with_capacity(setting_ids.len());
    let windows_build = current_windows_build();
    for setting_id in setting_ids {
        if cancellation.is_cancelled() {
            // Core owns the public cancellation protocol. Returning the partial read allows it to
            // convert the shared cancellation flag into the stable operation-cancelled error.
            break;
        }
        let Some(definition) = definition(setting_id) else {
            states.push(unsupported_state(setting_id));
            continue;
        };
        if !is_applicable(definition, windows_build) {
            states.push(unsupported_state_for_definition(definition));
            continue;
        }
        match native_key_available(definition) {
            Ok(true) => {}
            Ok(false) => {
                states.push(unsupported_state_for_definition(definition));
                continue;
            }
            Err(error) => {
                states.push(error_state_for_definition(definition, error));
                continue;
            }
        }
        states.push(read_state(definition));
    }
    Ok(states)
}

pub(crate) fn change(
    request: &PlatformSystemSettingChangeRequest,
) -> PlatformResult<PlatformSystemSettingChangeResult> {
    let definition = definition(&request.setting_id).ok_or_else(|| {
        PlatformError::new(
            PlatformErrorCode::Unsupported,
            "system setting identifier is unsupported",
        )
    })?;
    if !is_applicable(definition, current_windows_build()) {
        return Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "system setting is unavailable on this Windows version",
        ));
    }
    if !native_key_available(definition)? {
        return Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "system setting component is not installed",
        ));
    }
    validate_value(definition, &request.desired_value)?;
    let before = read_value(definition)?;
    if value_matches_desired(definition, &before, &request.desired_value) {
        return Ok(PlatformSystemSettingChangeResult {
            value: before,
            changed: false,
            verified: true,
        });
    }
    if before != request.expected_value {
        return Err(PlatformError::item_changed(
            "system setting changed after plan creation",
        ));
    }

    write_value(definition, &request.desired_value)
        .map_err(PlatformError::with_possible_side_effects)?;
    let after = read_value(definition).map_err(PlatformError::with_possible_side_effects)?;
    Ok(PlatformSystemSettingChangeResult {
        changed: after != before,
        verified: value_matches_desired(definition, &after, &request.desired_value),
        value: after,
    })
}

pub(crate) fn change_many(
    requests: &[PlatformSystemSettingChangeRequest],
) -> PlatformResult<Vec<PlatformResult<PlatformSystemSettingChangeResult>>> {
    change_many_with(
        requests,
        change,
        crate::system_settings_helper::change_many_with_privileges,
    )
}

fn change_many_with<DirectChange, PrivilegedChange>(
    requests: &[PlatformSystemSettingChangeRequest],
    mut direct_change: DirectChange,
    privileged_change: PrivilegedChange,
) -> PlatformResult<Vec<PlatformResult<PlatformSystemSettingChangeResult>>>
where
    DirectChange: FnMut(
        &PlatformSystemSettingChangeRequest,
    ) -> PlatformResult<PlatformSystemSettingChangeResult>,
    PrivilegedChange:
        FnOnce(
            &[&PlatformSystemSettingChangeRequest],
        ) -> PlatformResult<Vec<PlatformResult<PlatformSystemSettingChangeResult>>>,
{
    let mut results = std::iter::repeat_with(|| None)
        .take(requests.len())
        .collect::<Vec<_>>();
    let mut direct_indexes = Vec::new();
    let mut privileged_indexes = Vec::new();
    let mut privileged_requests = Vec::new();

    for (index, request) in requests.iter().enumerate() {
        match definition(&request.setting_id) {
            Some(definition) if definition.requires_elevation => {
                privileged_indexes.push(index);
                privileged_requests.push(request);
            }
            _ => direct_indexes.push(index),
        }
    }

    if !privileged_requests.is_empty() {
        let user_policy_item_count = privileged_requests
            .iter()
            .filter(|request| {
                definition(&request.setting_id)
                    .is_some_and(|definition| definition.root == RegistryRoot::CurrentUser)
            })
            .count();
        log::info!(
            "windows_system_settings_elevated_batch_started item_count={} machine_item_count={} user_policy_item_count={}",
            privileged_requests.len(),
            privileged_requests.len() - user_policy_item_count,
            user_policy_item_count
        );
        match privileged_change(&privileged_requests) {
            Ok(privileged_results) if privileged_results.len() == privileged_indexes.len() => {
                // The elevated response is advisory until the ordinary process re-reads every
                // value. This closes the response-file race and prevents a forged or truncated
                // response from discarding recovery for a setting that did not reach its target.
                let verified_results = privileged_requests
                    .iter()
                    .zip(privileged_results)
                    .map(|(request, result)| verify_privileged_change_result(request, result))
                    .collect::<Vec<_>>();
                let uncertain_mutation_count = verified_results
                    .iter()
                    .filter(|result| {
                        result.as_ref().is_err_and(|error| {
                            error.mutation_state() == crate::PlatformMutationState::MayHaveChanged
                        })
                    })
                    .count();
                log::info!(
                    "windows_system_settings_elevated_batch_finished item_count={} user_policy_item_count={} uncertain_mutation_count={}",
                    verified_results.len(),
                    user_policy_item_count,
                    uncertain_mutation_count
                );
                for (index, result) in privileged_indexes.into_iter().zip(verified_results) {
                    results[index] = Some(result);
                }
            }
            Ok(_) => {
                for index in privileged_indexes {
                    results[index] = Some(Err(PlatformError::new(
                        PlatformErrorCode::InvalidData,
                        "system settings helper returned an invalid result count",
                    )
                    .with_possible_side_effects()));
                }
            }
            Err(error) => {
                let code = error.code();
                let mutation_state = error.mutation_state();
                log::warn!(
                    "windows_system_settings_elevated_batch_failed item_count={} code={code:?} mutation_state={mutation_state:?}",
                    privileged_indexes.len(),
                );
                if code == PlatformErrorCode::UserCancelled {
                    // The UAC prompt is the authorization boundary for the whole user action. Do
                    // not apply current-user changes first: cancelling the prompt must leave the
                    // complete batch untouched instead of producing a surprising partial result.
                    return Ok(requests
                        .iter()
                        .map(|_| {
                            Err(PlatformError::new(
                                PlatformErrorCode::UserCancelled,
                                "system settings authorization was cancelled",
                            ))
                        })
                        .collect());
                }
                for index in privileged_indexes {
                    let item_error =
                        PlatformError::new(code, "system settings privileged batch failed");
                    results[index] = Some(Err(
                        if mutation_state == crate::PlatformMutationState::MayHaveChanged {
                            item_error.with_possible_side_effects()
                        } else {
                            item_error
                        },
                    ));
                }
            }
        }
    }

    // Current-user writes are deliberately delayed until any required UAC prompt succeeds or
    // fails normally. This preserves the cancellation semantics without forcing harmless HKCU
    // settings through the elevated helper.
    for index in direct_indexes {
        results[index] = Some(direct_change(&requests[index]));
    }

    results
        .into_iter()
        .map(|result| {
            result.ok_or_else(|| {
                PlatformError::new(
                    PlatformErrorCode::InvalidData,
                    "system settings batch result is missing",
                )
            })
        })
        .collect()
}

fn verify_privileged_change_result(
    request: &PlatformSystemSettingChangeRequest,
    reported: PlatformResult<PlatformSystemSettingChangeResult>,
) -> PlatformResult<PlatformSystemSettingChangeResult> {
    let definition = definition(&request.setting_id).ok_or_else(|| {
        PlatformError::new(
            PlatformErrorCode::Unsupported,
            "system setting identifier is unsupported during post-elevation verification",
        )
    })?;
    let current = read_value(definition).map_err(PlatformError::with_possible_side_effects)?;
    if value_matches_desired(definition, &current, &request.desired_value) {
        return Ok(PlatformSystemSettingChangeResult {
            changed: current != request.expected_value,
            verified: true,
            value: current,
        });
    }
    match reported {
        Err(error) => Err(error),
        Ok(_) => Err(PlatformError::operation_failed(
            "elevated system setting did not match the desired value during parent verification",
        )
        .with_possible_side_effects()),
    }
}

pub(crate) fn helper_change_many(
    requests: &[PlatformSystemSettingChangeRequest],
) -> Vec<PlatformResult<PlatformSystemSettingChangeResult>> {
    requests
        .iter()
        .map(|request| {
            let definition = definition(&request.setting_id).ok_or_else(|| {
                PlatformError::new(
                    PlatformErrorCode::Unsupported,
                    "system setting identifier is unsupported by the elevated helper",
                )
            })?;
            if !definition.requires_elevation {
                return Err(PlatformError::new(
                    PlatformErrorCode::Unsupported,
                    "system setting does not require the elevated helper",
                ));
            }
            change(request)
        })
        .collect()
}

fn definition(setting_id: &str) -> Option<SettingDefinition> {
    SETTINGS.iter().copied().find(|item| item.id == setting_id)
}

fn unsupported_state(setting_id: &str) -> PlatformSystemSettingState {
    PlatformSystemSettingState {
        setting_id: setting_id.to_string(),
        value: PlatformSystemSettingValue::Missing,
        effective_value: PlatformSystemSettingValue::Missing,
        requires_elevation: false,
        diagnostic: Some(PlatformSystemSettingDiagnosticCode::Unsupported),
    }
}

fn unsupported_state_for_definition(definition: SettingDefinition) -> PlatformSystemSettingState {
    PlatformSystemSettingState {
        setting_id: definition.id.to_string(),
        value: PlatformSystemSettingValue::Missing,
        effective_value: PlatformSystemSettingValue::Missing,
        requires_elevation: definition.requires_elevation,
        diagnostic: Some(PlatformSystemSettingDiagnosticCode::Unsupported),
    }
}

fn error_state_for_definition(
    definition: SettingDefinition,
    error: PlatformError,
) -> PlatformSystemSettingState {
    log::warn!(
        "windows_system_setting_availability_failed setting_id={} code={:?}",
        definition.id,
        error.code()
    );
    PlatformSystemSettingState {
        setting_id: definition.id.to_string(),
        value: PlatformSystemSettingValue::Missing,
        effective_value: PlatformSystemSettingValue::Missing,
        requires_elevation: definition.requires_elevation,
        diagnostic: Some(diagnostic_for_error(error.code())),
    }
}

/// Prevents optional Windows services from being fabricated by a generic registry write.
///
/// Service availability varies by Windows edition and installed components. Creating a missing
/// `Services` key would make verification pass even though no service exists, so those definitions
/// fail closed and remain hidden on machines where the native component is absent.
fn native_key_available(definition: SettingDefinition) -> PlatformResult<bool> {
    if !definition.requires_existing_key {
        return Ok(true);
    }
    match registry_root(definition.root).open_subkey_with_flags(definition.subkey, KEY_READ) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(registry_error("check system setting component", error)),
    }
}

fn is_applicable(definition: SettingDefinition, windows_build: Option<u32>) -> bool {
    match definition.applicability {
        BuildApplicability::Any => true,
        // Version-scoped registry contracts fail closed when Windows does not expose a readable
        // build. Writing a policy for the wrong generation can report success without changing UI.
        BuildApplicability::Before(exclusive_build) => {
            windows_build.is_some_and(|build| build < exclusive_build)
        }
        BuildApplicability::AtLeast(inclusive_build) => {
            windows_build.is_some_and(|build| build >= inclusive_build)
        }
        BuildApplicability::Range {
            inclusive_start,
            exclusive_end,
        } => windows_build.is_some_and(|build| (inclusive_start..exclusive_end).contains(&build)),
    }
}

fn current_windows_build() -> Option<u32> {
    let root = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = root
        .open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            KEY_READ | KEY_WOW64_64KEY,
        )
        .ok()?;
    key.get_value::<String, _>("CurrentBuildNumber")
        .ok()?
        .parse()
        .ok()
}

fn read_state(definition: SettingDefinition) -> PlatformSystemSettingState {
    match read_value(definition) {
        Ok(value) => {
            let effective_value = effective_value(definition, &value);
            PlatformSystemSettingState {
                setting_id: definition.id.to_string(),
                value,
                effective_value,
                requires_elevation: definition.requires_elevation,
                diagnostic: None,
            }
        }
        Err(error) => {
            log::warn!(
                "windows_system_setting_read_failed setting_id={} code={:?}",
                definition.id,
                error.code()
            );
            PlatformSystemSettingState {
                setting_id: definition.id.to_string(),
                value: PlatformSystemSettingValue::Missing,
                effective_value: PlatformSystemSettingValue::Missing,
                requires_elevation: definition.requires_elevation,
                diagnostic: Some(diagnostic_for_error(error.code())),
            }
        }
    }
}

fn diagnostic_for_error(code: PlatformErrorCode) -> PlatformSystemSettingDiagnosticCode {
    match code {
        PlatformErrorCode::AccessDenied => PlatformSystemSettingDiagnosticCode::AccessDenied,
        PlatformErrorCode::Unsupported => PlatformSystemSettingDiagnosticCode::Unsupported,
        _ => PlatformSystemSettingDiagnosticCode::StateUnavailable,
    }
}

fn read_value(definition: SettingDefinition) -> PlatformResult<PlatformSystemSettingValue> {
    if definition.id == shortcut_overlay::SETTING_ID {
        return shortcut_overlay::read();
    }
    if uses_windows_11_tray_contract(definition) {
        return read_windows_11_tray_icons();
    }

    let root = registry_root(definition.root);
    let key = match root.open_subkey_with_flags(definition.subkey, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(PlatformSystemSettingValue::Missing)
        }
        Err(error) => return Err(registry_error("read system setting key", error)),
    };
    match definition.kind {
        ValueKind::Dword => match key.get_value::<u32, _>(definition.value_name) {
            Ok(value) => Ok(PlatformSystemSettingValue::Integer(i64::from(value))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(PlatformSystemSettingValue::Missing)
            }
            Err(error) => Err(registry_error("read system setting value", error)),
        },
        ValueKind::Text => match key.get_value::<String, _>(definition.value_name) {
            Ok(value) if definition.composite_text_key.is_some() => Ok(
                PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::Text(value)),
            ),
            Ok(value) => Ok(PlatformSystemSettingValue::Text(value)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(PlatformSystemSettingValue::Missing)
            }
            Err(error) => Err(registry_error("read system setting value", error)),
        },
    }
}

fn write_value(
    definition: SettingDefinition,
    value: &PlatformSystemSettingValue,
) -> PlatformResult<()> {
    if definition.id == shortcut_overlay::SETTING_ID {
        return shortcut_overlay::write(value);
    }
    if uses_windows_11_tray_contract(definition) {
        return write_windows_11_tray_icons(value);
    }

    let root = registry_root(definition.root);
    if matches!(value, PlatformSystemSettingValue::Missing) {
        if let Some(tree_subkey) = definition.delete_tree_subkey {
            let result = match root.delete_subkey_all(tree_subkey) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(registry_error("delete system setting registry tree", error)),
            };
            if result.is_ok() {
                log::info!(
                    "windows_system_setting_tree_removed setting_id={}",
                    definition.id
                );
            }
            return result;
        }
    }
    let (key, _) = root
        .create_subkey_with_flags(
            definition.subkey,
            if definition.composite_text_key.is_some() {
                KEY_READ | KEY_SET_VALUE
            } else {
                KEY_SET_VALUE
            },
        )
        .map_err(|error| registry_error("open system setting key for writing", error))?;
    let result = match value {
        PlatformSystemSettingValue::Missing => match key.delete_value(definition.value_name) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(registry_error("delete system setting value", error)),
        },
        PlatformSystemSettingValue::Integer(value) => {
            let value = u32::try_from(*value).map_err(|_| {
                PlatformError::operation_failed("system setting integer is outside DWORD range")
            })?;
            key.set_value(definition.value_name, &value)
                .map_err(|error| registry_error("write system setting value", error))
        }
        PlatformSystemSettingValue::Text(value) if definition.composite_text_key.is_some() => {
            let existing = match key.get_value::<String, _>(definition.value_name) {
                Ok(existing) => existing,
                Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
                Err(error) => {
                    return Err(registry_error(
                        "read composite system setting before writing",
                        error,
                    ))
                }
            };
            let merged = update_composite_text(
                &existing,
                definition
                    .composite_text_key
                    .expect("composite text key must be present"),
                value,
            )?;
            let result = key
                .set_value(definition.value_name, &merged)
                .map_err(|error| registry_error("write composite system setting value", error));
            if result.is_ok() {
                log::info!(
                    "windows_composite_system_setting_changed setting_id={} retained_entry_count={}",
                    definition.id,
                    merged.split(';').filter(|entry| !entry.is_empty()).count()
                );
            }
            result
        }
        PlatformSystemSettingValue::Text(value) => key
            .set_value(definition.value_name, value)
            .map_err(|error| registry_error("write system setting value", error)),
        PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::Text(value))
            if definition.composite_text_key.is_some() =>
        {
            let result = key
                .set_value(definition.value_name, value)
                .map_err(|error| registry_error("restore composite system setting value", error));
            if result.is_ok() {
                log::info!(
                    "windows_composite_system_setting_restored setting_id={} restored_entry_count={}",
                    definition.id,
                    value.split(';').filter(|entry| !entry.is_empty()).count()
                );
            }
            result
        }
        PlatformSystemSettingValue::Boolean(_) | PlatformSystemSettingValue::Snapshot(_) => Err(
            PlatformError::operation_failed("boolean value is invalid for this system setting"),
        ),
    };
    if result.is_ok() && requires_immersive_color_refresh(definition) {
        broadcast_setting_change("ImmersiveColorSet");
    }
    result
}

fn effective_value(
    definition: SettingDefinition,
    value: &PlatformSystemSettingValue,
) -> PlatformSystemSettingValue {
    if definition.id == shortcut_overlay::SETTING_ID {
        return shortcut_overlay::effective(value);
    }
    if uses_windows_11_tray_contract(definition) {
        return windows_11_tray_effective_value(value);
    }
    let Some(composite_key) = definition.composite_text_key else {
        return value.clone();
    };
    let PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::Text(value)) = value
    else {
        return value.clone();
    };
    composite_entry(value, composite_key)
        .map(|entry| PlatformSystemSettingValue::Text(format!("{composite_key}={entry};")))
        .unwrap_or_else(|| PlatformSystemSettingValue::Text(String::new()))
}

fn value_matches_desired(
    definition: SettingDefinition,
    current: &PlatformSystemSettingValue,
    desired: &PlatformSystemSettingValue,
) -> bool {
    match desired {
        PlatformSystemSettingValue::Missing if definition.id == shortcut_overlay::SETTING_ID => {
            current == desired
        }
        PlatformSystemSettingValue::Snapshot(_) => current == desired,
        _ => effective_value(definition, current) == *desired,
    }
}

fn composite_entry<'a>(value: &'a str, key: &str) -> Option<&'a str> {
    value.split(';').find_map(|entry| {
        let (entry_key, entry_value) = entry.split_once('=')?;
        entry_key.eq_ignore_ascii_case(key).then_some(entry_value)
    })
}

/// Updates one field inside Windows' semicolon-delimited DirectX value while preserving sibling
/// fields such as VRR and Auto HDR. An empty desired value removes only the owned field.
fn update_composite_text(existing: &str, key: &str, desired: &str) -> PlatformResult<String> {
    let desired_value = composite_entry(desired, key);
    let mut entries = existing
        .split(';')
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let is_owned = entry
                .split_once('=')
                .is_some_and(|(entry_key, _)| entry_key.eq_ignore_ascii_case(key));
            (!is_owned).then(|| entry.to_string())
        })
        .collect::<Vec<_>>();
    if let Some(desired_value) = desired_value {
        entries.push(format!("{key}={desired_value}"));
    } else if !desired.is_empty() {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "composite system setting does not contain the owned field",
        ));
    }
    Ok(entries
        .into_iter()
        .map(|entry| format!("{entry};"))
        .collect())
}

const TRAY_CHEVRON_SNAPSHOT_KEY: &str = "chevron";
const TRAY_ICON_SNAPSHOT_PREFIX: &str = "icon:";

fn uses_windows_11_tray_contract(definition: SettingDefinition) -> bool {
    definition.id == "windows.taskbar.show-all-tray-icons"
        && current_windows_build().is_some_and(|build| build >= 22_000)
}

/// Reads the Windows 11 per-application tray visibility contract.
///
/// Windows 10 exposes one `EnableAutoTray` switch. Windows 11 keeps an `IsPromoted` value for
/// every known notification icon instead, so continuing to read the legacy value reports success
/// without reflecting what the taskbar actually renders. The product switch is considered on only
/// when every known icon is promoted; a mixed user configuration remains off until the user
/// explicitly chooses to show all icons.
fn read_windows_11_tray_icons() -> PlatformResult<PlatformSystemSettingValue> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let mut snapshot = BTreeMap::new();
    snapshot.insert(
        TRAY_CHEVRON_SNAPSHOT_KEY.to_string(),
        read_windows_11_tray_chevron_raw()?,
    );
    let settings = match root.open_subkey_with_flags(r"Control Panel\NotifyIconSettings", KEY_READ)
    {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(PlatformSystemSettingValue::Snapshot(
                PlatformSystemSettingSnapshot::IntegerMap(snapshot),
            ))
        }
        Err(error) => return Err(registry_error("read notification icon settings", error)),
    };

    for subkey_name in settings.enum_keys() {
        let subkey_name = subkey_name
            .map_err(|error| registry_error("enumerate notification icon settings", error))?;
        let subkey = settings
            .open_subkey_with_flags(&subkey_name, KEY_READ)
            .map_err(|error| registry_error("read notification icon setting", error))?;
        let value = match subkey.get_value::<u32, _>("IsPromoted") {
            Ok(value) => Some(i64::from(value)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(registry_error("read notification icon value", error)),
        };
        snapshot.insert(format!("{TRAY_ICON_SNAPSHOT_PREFIX}{subkey_name}"), value);
    }
    Ok(PlatformSystemSettingValue::Snapshot(
        PlatformSystemSettingSnapshot::IntegerMap(snapshot),
    ))
}

fn read_windows_11_tray_chevron_raw() -> PlatformResult<Option<i64>> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let key = match root.open_subkey_with_flags(
        r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\TrayNotify",
        KEY_READ,
    ) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(registry_error("read notification area state", error)),
    };
    match key.get_value::<u32, _>("SystemTrayChevronVisibility") {
        Ok(value) => Ok(Some(i64::from(value))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(registry_error("read notification area value", error)),
    }
}

fn windows_11_tray_effective_value(
    value: &PlatformSystemSettingValue,
) -> PlatformSystemSettingValue {
    let PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::IntegerMap(snapshot)) =
        value
    else {
        return value.clone();
    };
    let icons = snapshot
        .iter()
        .filter(|(key, _)| key.starts_with(TRAY_ICON_SNAPSHOT_PREFIX))
        .map(|(_, value)| value);
    let mut known_count = 0_usize;
    let mut every_icon_promoted = true;
    for value in icons {
        known_count += 1;
        every_icon_promoted &= *value == Some(1);
    }
    let show_all = if known_count == 0 {
        snapshot.get(TRAY_CHEVRON_SNAPSHOT_KEY).copied().flatten() == Some(0)
    } else {
        every_icon_promoted
    };
    PlatformSystemSettingValue::Integer(i64::from(!show_all))
}

/// Applies the Windows 11 tray choice to the chevron and every icon Windows currently knows.
///
/// The per-icon registry layout is dynamic: applications create their own opaque subkeys. MangoDisk
/// therefore enumerates existing entries instead of persisting application identifiers or guessing
/// paths. Newly installed applications continue to follow Windows defaults until the user applies
/// this switch again.
fn write_windows_11_tray_icons(value: &PlatformSystemSettingValue) -> PlatformResult<()> {
    if let PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::IntegerMap(
        snapshot,
    )) = value
    {
        return restore_windows_11_tray_icons(snapshot);
    }
    let show_all = match value {
        PlatformSystemSettingValue::Integer(0) => true,
        PlatformSystemSettingValue::Integer(1) => false,
        _ => {
            return Err(PlatformError::operation_failed(
                "notification icon setting requires a zero or one value",
            ))
        }
    };
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let (chevron, _) = root
        .create_subkey_with_flags(
            r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\TrayNotify",
            KEY_SET_VALUE,
        )
        .map_err(|error| registry_error("open notification area state for writing", error))?;
    chevron
        .set_value("SystemTrayChevronVisibility", &u32::from(!show_all))
        .map_err(|error| registry_error("write notification area state", error))?;

    let settings = match root.open_subkey_with_flags(
        r"Control Panel\NotifyIconSettings",
        KEY_READ | KEY_SET_VALUE,
    ) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            log::info!("windows_tray_visibility_changed show_all={show_all} known_icon_count=0");
            broadcast_setting_change("TraySettings");
            return Ok(());
        }
        Err(error) => return Err(registry_error("open notification icon settings", error)),
    };

    let mut changed_count = 0_u32;
    for subkey_name in settings.enum_keys() {
        let subkey_name = subkey_name
            .map_err(|error| registry_error("enumerate notification icon settings", error))?;
        let subkey = settings
            .open_subkey_with_flags(&subkey_name, KEY_SET_VALUE)
            .map_err(|error| registry_error("open notification icon setting for writing", error))?;
        subkey
            .set_value("IsPromoted", &u32::from(show_all))
            .map_err(|error| registry_error("write notification icon setting", error))?;
        changed_count += 1;
    }
    log::info!(
        "windows_tray_visibility_changed show_all={show_all} known_icon_count={changed_count}"
    );
    broadcast_setting_change("TraySettings");
    Ok(())
}

fn restore_windows_11_tray_icons(snapshot: &BTreeMap<String, Option<i64>>) -> PlatformResult<()> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let (chevron, _) = root
        .create_subkey_with_flags(
            r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\TrayNotify",
            KEY_SET_VALUE,
        )
        .map_err(|error| registry_error("open notification area state for restoration", error))?;
    match snapshot.get(TRAY_CHEVRON_SNAPSHOT_KEY).copied().flatten() {
        Some(value) => chevron
            .set_value("SystemTrayChevronVisibility", &(value as u32))
            .map_err(|error| registry_error("restore notification area state", error))?,
        None => match chevron.delete_value("SystemTrayChevronVisibility") {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(registry_error("clear notification area state", error)),
        },
    }

    let expected_icon_count = snapshot
        .keys()
        .filter(|key| key.starts_with(TRAY_ICON_SNAPSHOT_PREFIX))
        .count();
    let settings = match root.open_subkey_with_flags(
        r"Control Panel\NotifyIconSettings",
        KEY_READ | KEY_SET_VALUE,
    ) {
        Ok(settings) => settings,
        Err(error) if error.kind() == io::ErrorKind::NotFound && expected_icon_count == 0 => {
            log::info!(
                "windows_tray_visibility_restored restored_icon_count=0 missing_icon_count=0"
            );
            broadcast_setting_change("TraySettings");
            return Ok(());
        }
        Err(error) => {
            return Err(registry_error(
                "open notification icons for restoration",
                error,
            ))
        }
    };
    let mut restored_count = 0_u32;
    let mut missing_count = 0_u32;
    for (key, value) in snapshot
        .iter()
        .filter(|(key, _)| key.starts_with(TRAY_ICON_SNAPSHOT_PREFIX))
    {
        let subkey_name = &key[TRAY_ICON_SNAPSHOT_PREFIX.len()..];
        let subkey = match settings.open_subkey_with_flags(subkey_name, KEY_SET_VALUE) {
            Ok(subkey) => subkey,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing_count += 1;
                continue;
            }
            Err(error) => {
                return Err(registry_error(
                    "open notification icon for restoration",
                    error,
                ))
            }
        };
        match value {
            Some(value) => subkey
                .set_value("IsPromoted", &(*value as u32))
                .map_err(|error| registry_error("restore notification icon state", error))?,
            None => match subkey.delete_value("IsPromoted") {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(registry_error("clear notification icon state", error)),
            },
        }
        restored_count += 1;
    }
    log::info!(
        "windows_tray_visibility_restored restored_icon_count={restored_count} missing_icon_count={missing_count}"
    );
    broadcast_setting_change("TraySettings");
    Ok(())
}

fn requires_immersive_color_refresh(definition: SettingDefinition) -> bool {
    matches!(
        definition.id,
        "windows.personalization.dark-apps" | "windows.personalization.dark-system"
    )
}

/// Notifies Explorer and theme-aware applications after the registry value has been verified.
///
/// A bounded broadcast mirrors the notification sent by Windows Settings without restarting
/// Explorer. A non-responsive recipient must not turn a successfully persisted preference into a
/// failed operation, so refresh failure is logged as a diagnostic and the caller can still rely on
/// the verified registry state.
fn broadcast_setting_change(area_name: &str) {
    let mut area = area_name.encode_utf16().collect::<Vec<_>>();
    area.push(0);
    let mut result = 0_usize;
    // SAFETY: `area` is a live, NUL-terminated UTF-16 buffer for the duration of the synchronous
    // broadcast. The call does not retain the pointer and the timeout bounds hung recipients.
    let sent = unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            area.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            1_000,
            &mut result,
        )
    };
    if sent == 0 {
        log::warn!("windows_system_setting_refresh_failed area={area_name}");
    } else {
        log::info!("windows_system_setting_refresh_sent area={area_name}");
    }
}

fn registry_root(root: RegistryRoot) -> RegKey {
    match root {
        RegistryRoot::CurrentUser => RegKey::predef(HKEY_CURRENT_USER),
        RegistryRoot::LocalMachine => RegKey::predef(HKEY_LOCAL_MACHINE),
    }
}

fn validate_value(
    definition: SettingDefinition,
    value: &PlatformSystemSettingValue,
) -> PlatformResult<()> {
    if definition.id == shortcut_overlay::SETTING_ID {
        return if shortcut_overlay::valid_value(value) {
            Ok(())
        } else {
            Err(PlatformError::new(
                PlatformErrorCode::InvalidData,
                "shortcut overlay value is not allowlisted",
            ))
        };
    }
    let valid_snapshot = match value {
        PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::Text(value)) => {
            definition.composite_text_key.is_some() && value.len() <= 16 * 1024
        }
        PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::IntegerMap(values)) => {
            uses_windows_11_tray_contract(definition)
                && values.len() <= 512
                && values.iter().all(|(key, value)| {
                    !key.is_empty()
                        && key.len() <= 256
                        && value.is_none_or(|value| (0..=i64::from(u32::MAX)).contains(&value))
                })
        }
        _ => false,
    };
    let valid = valid_snapshot
        || matches!(value, PlatformSystemSettingValue::Missing)
        || matches!(
            (definition.kind, value),
            (ValueKind::Dword, PlatformSystemSettingValue::Integer(_))
                | (ValueKind::Text, PlatformSystemSettingValue::Text(_))
        );
    if valid && privileged_value_is_allowlisted(definition, value) {
        Ok(())
    } else {
        Err(PlatformError::operation_failed(
            "system setting value has an invalid type",
        ))
    }
}

/// Restricts the elevated helper to values meaningful for each compiled setting.
///
/// The desktop process normally supplies values from Core's catalog. This second allowlist exists
/// because the helper can also be launched directly from a command line after a UAC confirmation;
/// accepting an arbitrary DWORD there would make a known setting ID broader than the product UI.
fn privileged_value_is_allowlisted(
    definition: SettingDefinition,
    value: &PlatformSystemSettingValue,
) -> bool {
    if !definition.requires_elevation || matches!(value, PlatformSystemSettingValue::Missing) {
        return true;
    }
    match (definition.id, value) {
        (
            "windows.performance.reduce-service-shutdown-timeout",
            PlatformSystemSettingValue::Text(value),
        ) => value
            .parse::<u32>()
            .is_ok_and(|value| (1_000..=600_000).contains(&value)),
        ("windows.performance.reduce-crash-dump", PlatformSystemSettingValue::Integer(value)) => {
            matches!(*value, 0..=3 | 7)
        }
        (
            "windows.performance.disable-network-throttling",
            PlatformSystemSettingValue::Integer(value),
        ) => (0..=70).contains(value) || *value == i64::from(u32::MAX),
        (
            "windows.performance.remove-reserved-bandwidth"
            | "windows.performance.multimedia-responsiveness",
            PlatformSystemSettingValue::Integer(value),
        ) => (0..=100).contains(value),
        (
            "windows.services.print-spooler-manual"
            | "windows.services.disable-sysmain"
            | "windows.services.disable-compatibility-assistant"
            | "windows.services.disable-search-indexing"
            | "windows.services.disable-diagnostic-tracking"
            | "windows.services.disable-error-reporting"
            | "windows.services.disable-sensors"
            | "windows.services.disable-insider"
            | "windows.services.disable-xbox-auth"
            | "windows.services.disable-fax"
            | "windows.services.disable-media-player-sharing",
            PlatformSystemSettingValue::Integer(value),
        ) => (0..=4).contains(value),
        (
            "windows.storage.disable-ntfs-last-access",
            PlatformSystemSettingValue::Integer(value),
        ) => {
            // Windows 10 version 1803 and later uses bit 31 as an initialized marker and bit 1
            // for system-managed behavior. Recovery must accept those native values so toggling
            // the setting can restore the exact policy mode instead of only its enabled bit.
            matches!(*value, 0..=3 | 0x8000_0000..=0x8000_0003)
        }
        ("windows.update.notify-before-download", PlatformSystemSettingValue::Integer(value)) => {
            (2..=5).contains(value)
        }
        ("windows.update.disable-peer-sharing", PlatformSystemSettingValue::Integer(value)) => {
            matches!(*value, 0..=3 | 99 | 100)
        }
        ("windows.update.disable-preview-builds", PlatformSystemSettingValue::Integer(value)) => {
            (0..=2).contains(value)
        }
        (
            "windows.privacy.limit-diagnostic-data" | "windows.taskbar.hide-search-policy",
            PlatformSystemSettingValue::Integer(value),
        ) => (0..=3).contains(value),
        ("windows.edge.limit-diagnostic-data", PlatformSystemSettingValue::Integer(value)) => {
            (0..=2).contains(value)
        }
        (
            "windows.office.disable-optional-telemetry",
            PlatformSystemSettingValue::Integer(value),
        ) => (1..=3).contains(value),
        ("windows.security.disable-autorun", PlatformSystemSettingValue::Integer(value)) => {
            matches!(*value, 145 | 255)
        }
        ("windows.explorer.remove-cast-to-device", PlatformSystemSettingValue::Text(value)) => {
            value == "Play to Menu"
        }
        (
            "windows.update.disable-store-auto-updates",
            PlatformSystemSettingValue::Integer(value),
        ) => matches!(*value, 2 | 4),
        (
            "windows.gaming.enable-hardware-gpu-scheduling",
            PlatformSystemSettingValue::Integer(value),
        ) => matches!(*value, 1 | 2),
        (_, PlatformSystemSettingValue::Integer(value)) => matches!(*value, 0 | 1),
        _ => false,
    }
}

fn registry_error(operation: &'static str, error: io::Error) -> PlatformError {
    if error.kind() == io::ErrorKind::PermissionDenied {
        PlatformError::new(PlatformErrorCode::AccessDenied, operation)
    } else {
        PlatformError::io(operation, &error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_identifiers_are_unique_and_registry_scoped() {
        let mut ids = SETTINGS.iter().map(|item| item.id).collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SETTINGS.len());
        assert!(ids.iter().all(|id| id.starts_with("windows.")));
        assert!(SETTINGS.iter().all(|item| !item.subkey.is_empty()));
    }

    #[test]
    fn composite_directx_setting_preserves_unowned_fields() {
        let existing =
            "SwapEffectUpgradeEnable=0;VRROptimizeEnable=1;legacy-token;AutoHDREnable=1;";

        let enabled = update_composite_text(
            existing,
            "SwapEffectUpgradeEnable",
            "SwapEffectUpgradeEnable=1;",
        )
        .expect("the owned DirectX field should update");
        let disabled = update_composite_text(&enabled, "SwapEffectUpgradeEnable", "")
            .expect("the owned DirectX field should be removable");

        assert!(enabled.contains("SwapEffectUpgradeEnable=1;"));
        assert!(enabled.contains("VRROptimizeEnable=1;"));
        assert!(enabled.contains("AutoHDREnable=1;"));
        assert!(enabled.contains("legacy-token;"));
        assert!(!disabled.contains("SwapEffectUpgradeEnable="));
        assert!(disabled.contains("VRROptimizeEnable=1;"));
        assert!(disabled.contains("AutoHDREnable=1;"));
        assert!(disabled.contains("legacy-token;"));
    }

    #[test]
    fn special_registry_definitions_use_lossless_storage_contracts() {
        let context_menu = definition("windows.explorer.classic-context-menu")
            .expect("the classic context-menu setting should exist");
        let windowed_games = definition("windows.gaming.optimize-windowed-games")
            .expect("the windowed-games setting should exist");

        assert_eq!(
            context_menu.delete_tree_subkey,
            Some(r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}")
        );
        assert_eq!(
            windowed_games.composite_text_key,
            Some("SwapEffectUpgradeEnable")
        );
    }

    #[test]
    fn expanded_catalog_uses_expected_registry_contracts() {
        let cases = [
            (
                "windows.explorer.confirm-file-delete",
                RegistryRoot::CurrentUser,
                r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer",
                "ConfirmFileDelete",
            ),
            (
                "windows.edge.limit-diagnostic-data",
                RegistryRoot::LocalMachine,
                r"SOFTWARE\Policies\Microsoft\Edge",
                "DiagnosticData",
            ),
            (
                "windows.network.disable-llmnr",
                RegistryRoot::LocalMachine,
                r"SOFTWARE\Policies\Microsoft\Windows NT\DNSClient",
                "EnableMulticast",
            ),
            (
                "windows.update.enable-microsoft-product-updates",
                RegistryRoot::LocalMachine,
                r"SOFTWARE\Microsoft\WindowsUpdate\UX\Settings",
                "AllowMUUpdateService",
            ),
        ];

        for (setting_id, root, subkey, value_name) in cases {
            let definition = definition(setting_id).expect("the expanded setting should exist");
            assert!(definition.root == root);
            assert_eq!(definition.subkey, subkey);
            assert_eq!(definition.value_name, value_name);
            assert!(matches!(definition.kind, ValueKind::Dword));
        }
    }

    #[test]
    fn tray_snapshot_preserves_mixed_icon_state_while_exposing_one_switch_value() {
        let definition = definition("windows.taskbar.show-all-tray-icons")
            .expect("the tray setting should exist");
        let snapshot = PlatformSystemSettingValue::Snapshot(
            PlatformSystemSettingSnapshot::IntegerMap(BTreeMap::from([
                (TRAY_CHEVRON_SNAPSHOT_KEY.to_string(), Some(1)),
                (format!("{TRAY_ICON_SNAPSHOT_PREFIX}one"), Some(1)),
                (format!("{TRAY_ICON_SNAPSHOT_PREFIX}two"), Some(0)),
            ])),
        );

        assert_eq!(
            windows_11_tray_effective_value(&snapshot),
            PlatformSystemSettingValue::Integer(1)
        );
        assert!(value_matches_desired(definition, &snapshot, &snapshot));
        assert!(!value_matches_desired(
            definition,
            &snapshot,
            &PlatformSystemSettingValue::Integer(0)
        ));
    }

    #[test]
    fn elevated_settings_reject_values_outside_their_semantic_range() {
        let service = definition("windows.services.disable-search-indexing")
            .expect("the service setting should exist");
        let error = validate_value(service, &PlatformSystemSettingValue::Integer(5))
            .expect_err("an invalid service start mode must be rejected");
        assert_eq!(error.code(), PlatformErrorCode::OperationFailed);

        let timeout = definition("windows.performance.reduce-service-shutdown-timeout")
            .expect("the timeout setting should exist");
        assert!(validate_value(
            timeout,
            &PlatformSystemSettingValue::Text("2000".to_string())
        )
        .is_ok());
        assert!(
            validate_value(timeout, &PlatformSystemSettingValue::Text("1".to_string())).is_err()
        );

        assert!(validate_value(service, &PlatformSystemSettingValue::Missing).is_ok());

        let last_access = definition("windows.storage.disable-ntfs-last-access")
            .expect("the NTFS last-access setting should exist");
        assert!(validate_value(
            last_access,
            &PlatformSystemSettingValue::Integer(0x8000_0003)
        )
        .is_ok());
        assert!(validate_value(
            last_access,
            &PlatformSystemSettingValue::Integer(0x4000_0003)
        )
        .is_err());

        let delivery_optimization = definition("windows.update.disable-peer-sharing")
            .expect("the Delivery Optimization policy should exist");
        assert!(validate_value(
            delivery_optimization,
            &PlatformSystemSettingValue::Integer(100)
        )
        .is_ok());
        assert!(validate_value(
            delivery_optimization,
            &PlatformSystemSettingValue::Integer(4)
        )
        .is_err());

        let search_policy = definition("windows.taskbar.hide-search-policy")
            .expect("the taskbar search policy should exist");
        assert!(validate_value(search_policy, &PlatformSystemSettingValue::Integer(3)).is_ok());
        assert!(validate_value(search_policy, &PlatformSystemSettingValue::Integer(4)).is_err());

        let edge_diagnostics = definition("windows.edge.limit-diagnostic-data")
            .expect("the Edge diagnostic data policy should exist");
        assert!(validate_value(edge_diagnostics, &PlatformSystemSettingValue::Integer(2)).is_ok());
        assert!(validate_value(edge_diagnostics, &PlatformSystemSettingValue::Integer(3)).is_err());

        let delete_confirmation = definition("windows.explorer.confirm-file-delete")
            .expect("the delete confirmation policy should exist");
        assert!(
            validate_value(delete_confirmation, &PlatformSystemSettingValue::Integer(1)).is_ok()
        );
        assert!(
            validate_value(delete_confirmation, &PlatformSystemSettingValue::Integer(2)).is_err()
        );

        let autorun = definition("windows.security.disable-autorun")
            .expect("the autorun policy should exist");
        assert!(validate_value(autorun, &PlatformSystemSettingValue::Integer(255)).is_ok());
        assert!(validate_value(autorun, &PlatformSystemSettingValue::Integer(254)).is_err());
    }

    #[test]
    fn protected_policy_settings_are_routed_through_the_elevated_helper() {
        let policy_ids = [
            "windows.edge.disable-sidebar",
            "windows.edge.disable-personalization",
            "windows.edge.disable-recommendations",
            "windows.edge.limit-diagnostic-data",
            "windows.ai.disable-recall-snapshots",
            "windows.ai.disable-copilot",
            "windows.search.disable-web-suggestions",
            "windows.search.disable-highlights",
            "windows.taskbar.hide-weather",
            "windows.taskbar.hide-widgets",
            "windows.taskbar.disable-widgets-board",
            "windows.taskbar.hide-search-policy",
            "windows.explorer.remove-cast-to-device",
            "windows.edge.disable-web-widget",
            "windows.firefox.disable-telemetry",
            "windows.update.disable-peer-sharing",
            "windows.update.disable-preview-builds",
            "windows.update.prevent-restart-when-logged-on",
            "windows.update.disable-automatic-updates",
            "windows.update.enable-microsoft-product-updates",
            "windows.update.enable-restart-notifications",
            "windows.privacy.limit-diagnostic-data",
            "windows.network.disable-llmnr",
        ];

        for setting_id in policy_ids {
            let definition = definition(setting_id).expect("the policy setting should exist");
            assert!(definition.root == RegistryRoot::LocalMachine);
            assert!(definition.requires_elevation);
        }
    }

    #[test]
    fn protected_current_user_policies_keep_the_user_hive_and_require_elevation() {
        for setting_id in [
            "windows.explorer.confirm-file-delete",
            "windows.office.disable-optional-telemetry",
            "windows.security.disable-autorun",
        ] {
            let definition = definition(setting_id).expect("the user policy setting should exist");
            assert!(definition.root == RegistryRoot::CurrentUser);
            assert!(definition.requires_elevation);
        }
    }

    #[test]
    fn elevated_helper_rejects_settings_that_do_not_require_privileges() {
        let results = helper_change_many(&[PlatformSystemSettingChangeRequest {
            setting_id: "windows.explorer.show-file-extensions".to_string(),
            expected_value: PlatformSystemSettingValue::Missing,
            desired_value: PlatformSystemSettingValue::Integer(0),
        }]);

        let error = results
            .into_iter()
            .next()
            .expect("one result should exist")
            .expect_err("a direct user preference must be rejected by the helper");
        assert_eq!(error.code(), PlatformErrorCode::Unsupported);
    }

    #[test]
    fn optional_services_require_the_native_service_key() {
        for setting_id in [
            "windows.services.disable-fax",
            "windows.services.disable-media-player-sharing",
        ] {
            let definition = definition(setting_id).expect("the optional service should exist");
            assert!(definition.requires_existing_key);
            assert!(definition.root == RegistryRoot::LocalMachine);
        }
    }

    #[test]
    fn taskbar_weather_uses_the_machine_feeds_policy() {
        let definition = definition("windows.taskbar.hide-weather")
            .expect("the taskbar weather setting should exist");

        assert!(definition.root == RegistryRoot::LocalMachine);
        assert_eq!(
            definition.subkey,
            r"SOFTWARE\Policies\Microsoft\Windows\Windows Feeds"
        );
        assert_eq!(definition.value_name, "EnableFeeds");
        assert!(validate_value(definition, &PlatformSystemSettingValue::Integer(0)).is_ok());
        assert!(validate_value(definition, &PlatformSystemSettingValue::Integer(2)).is_err());
        assert!(validate_value(definition, &PlatformSystemSettingValue::Missing).is_ok());
        assert!(is_applicable(definition, Some(19_045)));
        assert!(!is_applicable(definition, Some(22_000)));
        assert!(!is_applicable(definition, None));
    }

    #[test]
    fn search_highlights_uses_the_supported_machine_policy() {
        let definition = definition("windows.search.disable-highlights")
            .expect("the search highlights setting should exist");

        assert!(definition.root == RegistryRoot::LocalMachine);
        assert_eq!(
            definition.subkey,
            r"SOFTWARE\Policies\Microsoft\Windows\Windows Search"
        );
        assert_eq!(definition.value_name, "EnableDynamicContentInWSB");
        assert!(validate_value(definition, &PlatformSystemSettingValue::Integer(0)).is_ok());
        assert!(validate_value(definition, &PlatformSystemSettingValue::Integer(2)).is_err());
        assert!(validate_value(definition, &PlatformSystemSettingValue::Missing).is_ok());
    }

    #[test]
    fn taskbar_search_uses_the_supported_contract_for_each_windows_generation() {
        let legacy = definition("windows.taskbar.hide-search")
            .expect("the legacy taskbar search setting should exist");
        let policy = definition("windows.taskbar.hide-search-policy")
            .expect("the taskbar search policy should exist");

        assert!(is_applicable(legacy, Some(19_045)));
        assert!(is_applicable(legacy, Some(22_631)));
        assert!(!is_applicable(legacy, Some(26_100)));
        assert!(!is_applicable(policy, Some(22_631)));
        assert!(is_applicable(policy, Some(26_100)));
        assert!(policy.root == RegistryRoot::LocalMachine);
    }

    #[test]
    fn widgets_use_the_supported_policy_for_each_windows_generation() {
        let legacy = definition("windows.taskbar.hide-widgets")
            .expect("the stable widgets policy should exist");
        let current = definition("windows.taskbar.disable-widgets-board")
            .expect("the current widgets policy should exist");

        assert!(is_applicable(legacy, Some(22_631)));
        assert!(!is_applicable(legacy, Some(26_100)));
        assert!(!is_applicable(current, Some(22_631)));
        assert!(is_applicable(current, Some(26_100)));
        assert_eq!(current.value_name, "DisableWidgetsBoard");
    }

    #[test]
    fn windows_11_registry_contracts_fail_closed_on_windows_10() {
        let windows_11_ids = [
            "windows.explorer.compact-mode",
            "windows.explorer.classic-context-menu",
            "windows.taskbar.hide-widgets",
            "windows.taskbar.enable-end-task",
            "windows.taskbar.align-left",
            "windows.taskbar.hide-chat",
            "windows.windowing.disable-snap-assist",
            "windows.gaming.optimize-windowed-games",
            "windows.gaming.disable-dynamic-lighting",
            "windows.gaming.disable-app-lighting-control",
            "windows.start.hide-most-used-apps",
            "windows.ai.disable-copilot",
        ];

        for setting_id in windows_11_ids {
            let definition = definition(setting_id).expect("the Windows 11 setting should exist");
            assert!(
                !is_applicable(definition, Some(19_045)),
                "{setting_id} must not be offered on Windows 10"
            );
            assert!(
                is_applicable(definition, Some(22_631)),
                "{setting_id} should remain available on a supported Windows 11 build"
            );
        }

        let recall = definition("windows.ai.disable-recall-snapshots")
            .expect("the Recall policy setting should exist");
        assert!(!is_applicable(recall, Some(22_631)));
        assert!(is_applicable(recall, Some(26_100)));
    }

    #[test]
    fn legacy_copilot_button_is_limited_to_its_supported_build_range() {
        let definition = definition("windows.taskbar.hide-copilot")
            .expect("the legacy Copilot taskbar setting should exist");

        assert!(!is_applicable(definition, Some(19_045)));
        assert!(!is_applicable(definition, Some(22_000)));
        assert!(is_applicable(definition, Some(22_621)));
        assert!(is_applicable(definition, Some(26_099)));
        assert!(!is_applicable(definition, Some(26_100)));
    }

    #[test]
    fn cancelling_the_uac_prompt_leaves_current_user_changes_untouched() {
        use std::cell::Cell;

        let requests = [
            PlatformSystemSettingChangeRequest {
                setting_id: "windows.explorer.show-file-extensions".to_string(),
                expected_value: PlatformSystemSettingValue::Integer(1),
                desired_value: PlatformSystemSettingValue::Integer(0),
            },
            PlatformSystemSettingChangeRequest {
                setting_id: "windows.explorer.confirm-file-delete".to_string(),
                expected_value: PlatformSystemSettingValue::Missing,
                desired_value: PlatformSystemSettingValue::Integer(1),
            },
        ];
        let direct_called = Cell::new(false);

        let results = change_many_with(
            &requests,
            |_| {
                direct_called.set(true);
                unreachable!("current-user changes must wait for the UAC result")
            },
            |_| {
                Err(PlatformError::new(
                    PlatformErrorCode::UserCancelled,
                    "test cancellation",
                ))
            },
        )
        .expect("a cancelled batch should still return per-item results");

        assert!(!direct_called.get());
        assert_eq!(results.len(), requests.len());
        assert!(results.into_iter().all(|result| {
            result
                .expect_err("every item should preserve the cancellation")
                .code()
                == PlatformErrorCode::UserCancelled
        }));
    }

    #[test]
    fn elevated_batch_preserves_uncertain_mutation_state_for_each_item() {
        let requests = [PlatformSystemSettingChangeRequest {
            setting_id: "windows.taskbar.hide-weather".to_string(),
            expected_value: PlatformSystemSettingValue::Missing,
            desired_value: PlatformSystemSettingValue::Integer(0),
        }];

        let results = change_many_with(
            &requests,
            |_| unreachable!("the machine policy must use the elevated helper"),
            |_| {
                Err(PlatformError::operation_failed("test response failure")
                    .with_possible_side_effects())
            },
        )
        .expect("the batch should return one typed item failure");
        let error = results
            .into_iter()
            .next()
            .expect("one result should exist")
            .expect_err("the item should report the helper failure");

        assert_eq!(
            error.mutation_state(),
            crate::PlatformMutationState::MayHaveChanged
        );
    }

    #[test]
    #[ignore = "temporarily changes and exactly restores three composite Windows 11 user settings"]
    fn actual_composite_user_settings_roundtrip_and_restore() {
        if current_windows_build().is_none_or(|build| build < 22_000) {
            return;
        }
        let mut failures = Vec::new();
        for setting_id in [
            "windows.explorer.classic-context-menu",
            "windows.gaming.optimize-windowed-games",
            "windows.taskbar.show-all-tray-icons",
        ] {
            let definition = definition(setting_id).expect("the composite setting should exist");
            let original = read_value(definition).expect("the original setting should be readable");
            let desired = match setting_id {
                "windows.explorer.classic-context-menu" => {
                    if effective_value(definition, &original)
                        == PlatformSystemSettingValue::Text(String::new())
                    {
                        PlatformSystemSettingValue::Missing
                    } else {
                        PlatformSystemSettingValue::Text(String::new())
                    }
                }
                "windows.gaming.optimize-windowed-games" => {
                    let enabled =
                        PlatformSystemSettingValue::Text("SwapEffectUpgradeEnable=1;".to_string());
                    if effective_value(definition, &original) == enabled {
                        PlatformSystemSettingValue::Text(String::new())
                    } else {
                        enabled
                    }
                }
                _ => {
                    if effective_value(definition, &original)
                        == PlatformSystemSettingValue::Integer(0)
                    {
                        PlatformSystemSettingValue::Integer(1)
                    } else {
                        PlatformSystemSettingValue::Integer(0)
                    }
                }
            };
            let expected_composite_value = match (&original, &desired) {
                (
                    PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::Text(
                        original,
                    )),
                    PlatformSystemSettingValue::Text(desired),
                ) if setting_id == "windows.gaming.optimize-windowed-games" => Some(
                    PlatformSystemSettingValue::Snapshot(PlatformSystemSettingSnapshot::Text(
                        update_composite_text(original, "SwapEffectUpgradeEnable", desired)
                            .expect("the test DirectX value should be valid"),
                    )),
                ),
                _ => None,
            };
            let apply = change(&PlatformSystemSettingChangeRequest {
                setting_id: setting_id.to_string(),
                expected_value: original.clone(),
                desired_value: desired.clone(),
            });
            let after = read_value(definition).expect("the changed setting should remain readable");
            let restore = (after != original).then(|| {
                change(&PlatformSystemSettingChangeRequest {
                    setting_id: setting_id.to_string(),
                    expected_value: after.clone(),
                    desired_value: original.clone(),
                })
            });
            let restored = read_value(definition).expect("the restored setting should be readable");

            if !apply.is_ok_and(|result| result.verified) {
                failures.push(format!("{setting_id}: apply failed verification"));
            }
            if expected_composite_value.is_some_and(|expected| after != expected) {
                failures.push(format!(
                    "{setting_id}: applying the owned field changed sibling DirectX values"
                ));
            }
            if restore.is_some_and(|result| !result.is_ok_and(|result| result.verified)) {
                failures.push(format!("{setting_id}: restore failed verification"));
            }
            if restored != original {
                failures.push(format!(
                    "{setting_id}: original native snapshot was not restored"
                ));
            }
        }

        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    #[ignore = "temporarily toggles the file-extension preference and restores its exact value"]
    fn actual_file_extension_preference_roundtrips_and_restores() {
        let definition = definition("windows.explorer.show-file-extensions")
            .expect("the file-extension setting should exist");
        let original = read_value(definition).expect("the original preference should be readable");
        let desired =
            if effective_value(definition, &original) == PlatformSystemSettingValue::Integer(0) {
                PlatformSystemSettingValue::Integer(1)
            } else {
                PlatformSystemSettingValue::Integer(0)
            };
        let apply = change(&PlatformSystemSettingChangeRequest {
            setting_id: definition.id.to_string(),
            expected_value: original.clone(),
            desired_value: desired,
        });
        let after = read_value(definition).expect("the changed preference should be readable");
        let restore = (after != original).then(|| {
            change(&PlatformSystemSettingChangeRequest {
                setting_id: definition.id.to_string(),
                expected_value: after,
                desired_value: original.clone(),
            })
        });
        let restored = read_value(definition).expect("the restored preference should be readable");

        assert!(apply.is_ok_and(|result| result.verified));
        assert!(restore.is_none_or(|result| result.is_ok_and(|result| result.verified)));
        assert_eq!(restored, original);
    }

    #[test]
    #[ignore = "temporarily changes and restores newly added Windows user preferences"]
    fn actual_expanded_user_settings_roundtrip_and_restore() {
        let windows_build = current_windows_build();
        let mut failures = Vec::new();

        for setting_id in [
            "windows.explorer.use-manual-default-printer",
            "windows.taskbar.show-on-all-displays",
            "windows.gaming.disable-game-bar-controller",
            "windows.gaming.disable-game-bar-tips",
            "windows.gaming.disable-dynamic-lighting",
            "windows.gaming.disable-app-lighting-control",
            "windows.start.hide-recently-added-apps",
            "windows.start.hide-most-used-apps",
            "windows.start.hide-recent-items",
        ] {
            let definition = definition(setting_id).expect("the expanded setting should exist");
            if !is_applicable(definition, windows_build) {
                continue;
            }
            let original = read_value(definition).expect("the original setting should be readable");
            let desired = if original == PlatformSystemSettingValue::Integer(0) {
                PlatformSystemSettingValue::Integer(1)
            } else {
                PlatformSystemSettingValue::Integer(0)
            };

            let apply = change(&PlatformSystemSettingChangeRequest {
                setting_id: setting_id.to_string(),
                expected_value: original.clone(),
                desired_value: desired,
            });
            let after = read_value(definition).expect("the changed setting should remain readable");
            let restore = (after != original).then(|| {
                change(&PlatformSystemSettingChangeRequest {
                    setting_id: setting_id.to_string(),
                    expected_value: after,
                    desired_value: original.clone(),
                })
            });
            let restored = read_value(definition).expect("the restored setting should be readable");

            if !apply.is_ok_and(|result| result.verified) {
                failures.push(format!("{setting_id}: apply failed verification"));
            }
            if restore.is_some_and(|result| !result.is_ok_and(|result| result.verified)) {
                failures.push(format!("{setting_id}: restore failed verification"));
            }
            if restored != original {
                failures.push(format!("{setting_id}: original value was not restored"));
            }
        }

        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    #[ignore = "temporarily changes the supported Windows search highlights policy and restores it"]
    fn actual_search_highlights_policy_roundtrips_and_restores() {
        let definition = definition("windows.search.disable-highlights")
            .expect("the search highlights setting should exist");
        let original = read_value(definition)
            .expect("the supported search highlights policy should be readable");
        let desired = if original == PlatformSystemSettingValue::Integer(0) {
            PlatformSystemSettingValue::Missing
        } else {
            PlatformSystemSettingValue::Integer(0)
        };

        // Always attempt restoration before asserting the apply result. This keeps the
        // ignored machine test safe when verification fails after the registry write.
        let apply_result = change(&PlatformSystemSettingChangeRequest {
            setting_id: definition.id.to_string(),
            expected_value: original.clone(),
            desired_value: desired.clone(),
        });
        let value_after_apply = read_value(definition)
            .expect("the search highlights policy should remain readable after the change");
        let restore_result = if value_after_apply == original {
            None
        } else {
            Some(change(&PlatformSystemSettingChangeRequest {
                setting_id: definition.id.to_string(),
                expected_value: value_after_apply,
                desired_value: original.clone(),
            }))
        };
        let restored = read_value(definition)
            .expect("the search highlights policy should remain readable after restoration");

        assert_eq!(
            restored, original,
            "the original policy value must be restored"
        );
        let applied = apply_result.expect("the supported search highlights policy should change");
        assert!(applied.verified);
        assert_eq!(applied.value, desired);
        if let Some(result) = restore_result {
            let restored = result.expect("the original search highlights policy should restore");
            assert!(restored.verified);
            assert_eq!(restored.value, original);
        }
    }

    #[test]
    #[ignore = "reads the current Windows user registry"]
    fn actual_catalog_reads_every_known_setting() {
        let ids = SETTINGS.iter().map(|item| item.id).collect::<Vec<_>>();
        let states = scan(&ids, &PlatformCancellation::new(|| false))
            .expect("the Windows settings catalog should be readable");
        let windows_build = current_windows_build();

        assert_eq!(states.len(), SETTINGS.len());
        for (definition, state) in SETTINGS.iter().zip(states) {
            if !is_applicable(*definition, windows_build) {
                assert_eq!(
                    state.diagnostic,
                    Some(PlatformSystemSettingDiagnosticCode::Unsupported),
                    "{} should fail closed on this Windows build",
                    definition.id
                );
            } else if definition.requires_existing_key
                && state.diagnostic == Some(PlatformSystemSettingDiagnosticCode::Unsupported)
            {
                // Optional Windows components legitimately disappear from the registry when the
                // corresponding feature is not installed on this machine.
                continue;
            } else {
                assert_eq!(
                    state.diagnostic, None,
                    "{} should be readable",
                    definition.id
                );
            }
        }
    }
}
