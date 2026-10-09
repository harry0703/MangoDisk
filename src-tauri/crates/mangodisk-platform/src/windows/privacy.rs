use std::{
    fs,
    io::{BufRead, BufReader},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, CountClipboardFormats, EmptyClipboard, GetClipboardSequenceNumber,
    OpenClipboard,
};
use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_READ},
    types::FromRegValue,
    RegKey, RegValue,
};

use crate::{
    browser_profile::{chromium_display_name, firefox_display_names},
    vscode_history, PlatformCancellation, PlatformError, PlatformErrorCode,
    PlatformPrivacyApplication, PlatformPrivacyApplicationNativeTraceKind,
    PlatformPrivacyApplicationTrace, PlatformPrivacyApplicationTraceAvailability,
    PlatformPrivacyApplicationTraceKind, PlatformPrivacyBrowser, PlatformPrivacyBrowserKind,
    PlatformPrivacyDetailEntry, PlatformPrivacyDiscovery, PlatformPrivacyProfile,
    PlatformPrivacySystemTrace, PlatformPrivacySystemTraceKind, PlatformResult,
};

mod media_history;
mod office_history;
mod packaged_app_history;

const RUN_DIALOG_MRU: &[&str] = &[r"Software\Microsoft\Windows\CurrentVersion\Explorer\RunMRU"];
const FILE_DIALOG_MRU: &[&str] = &[
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32\OpenSaveMRU",
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32\OpenSavePidlMRU",
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32\LastVisitedMRU",
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32\LastVisitedPidlMRU",
];
const EXPLORER_SEARCH_MRU: &[&str] =
    &[r"Software\Microsoft\Windows\CurrentVersion\Explorer\WordWheelQuery"];
const EXPLORER_TYPED_PATHS: &[&str] =
    &[r"Software\Microsoft\Windows\CurrentVersion\Explorer\TypedPaths"];
const RECENT_DOCUMENT_HISTORY: &[&str] =
    &[r"Software\Microsoft\Windows\CurrentVersion\Explorer\RecentDocs"];
const APPLICATION_USAGE_HISTORY: &[&str] = &[
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist\{5E6AB780-7743-11CF-A12B-00AA004AE837}\Count",
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist\{75048700-EF1F-11D0-9888-006097DEACF9}\Count",
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist\{CEBFF5CD-ACE2-4F4F-9178-9926F41749EA}\Count",
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist\{F4E57C4B-2036-45F0-A9AB-443BCFE33D9F}\Count",
];
const NETWORK_CONNECTION_HISTORY: &[&str] = &[
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\Map Network Drive MRU",
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\FindComputerMRU",
];
const FOLDER_VIEW_BAGS: &str =
    r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags";
const FOLDER_VIEW_BAG_MRU: &str =
    r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU";
const FOLDER_VIEW_HISTORY: &[&str] = &[FOLDER_VIEW_BAGS, FOLDER_VIEW_BAG_MRU];
const PRINTER_HISTORY: &[&str] =
    &[r"Software\Microsoft\Windows\CurrentVersion\Explorer\PrnPortsMRU"];

pub(super) fn discover(
    cancellation: &PlatformCancellation,
) -> PlatformResult<PlatformPrivacyDiscovery> {
    let local = dirs::data_local_dir().ok_or_else(|| {
        PlatformError::invalid_path("local application data directory is unavailable")
    })?;
    let roaming = dirs::data_dir().ok_or_else(|| {
        PlatformError::invalid_path("roaming application data directory is unavailable")
    })?;
    let opera_root = preferred_directory([
        roaming.join("Opera Software/Opera Stable"),
        local.join("Opera Software/Opera Stable"),
    ]);
    let safe_browser_root = preferred_directory([
        local.join("360Chrome/Chrome/User Data"),
        local.join("360ChromeX/Chrome/User Data"),
        roaming.join("360se6/User Data"),
    ]);
    let qq_browser_root = preferred_directory([
        local.join("Tencent/QQBrowser/User Data"),
        local.join("QQBrowser/User Data"),
        roaming.join("Tencent/QQBrowser/User Data"),
    ]);
    let browsers = vec![
        chromium_browser(
            "chrome",
            "Google Chrome",
            &local.join("Google/Chrome/User Data"),
            windows_application_path(
                &local.join("Google/Chrome/Application/chrome.exe"),
                "Google/Chrome/Application/chrome.exe",
            ),
            vec!["chrome.exe".into()],
            cancellation,
        )?,
        chromium_browser(
            "edge",
            "Microsoft Edge",
            &local.join("Microsoft/Edge/User Data"),
            windows_application_path(
                &local.join("Microsoft/Edge/Application/msedge.exe"),
                "Microsoft/Edge/Application/msedge.exe",
            ),
            vec!["msedge.exe".into()],
            cancellation,
        )?,
        chromium_browser(
            "brave",
            "Brave",
            &local.join("BraveSoftware/Brave-Browser/User Data"),
            windows_application_path(
                &local.join("BraveSoftware/Brave-Browser/Application/brave.exe"),
                "BraveSoftware/Brave-Browser/Application/brave.exe",
            ),
            vec!["brave.exe".into()],
            cancellation,
        )?,
        chromium_browser(
            "opera",
            "Opera",
            &opera_root,
            windows_application_path(&local.join("Programs/Opera/opera.exe"), "Opera/opera.exe"),
            vec!["opera.exe".into()],
            cancellation,
        )?,
        chromium_browser(
            "360_safe_browser",
            "360 Safe Browser",
            &safe_browser_root,
            first_existing_file(
                [
                    local.join("360Chrome/Chrome/Application/360chrome.exe"),
                    local.join("360ChromeX/Chrome/Application/360ChromeX.exe"),
                ]
                .into_iter()
                .chain(windows_program_files_paths(&[
                    "360/360Chrome/Chrome/Application/360chrome.exe",
                    "360/360ChromeX/Chrome/Application/360ChromeX.exe",
                ])),
            ),
            vec!["360chrome.exe".into(), "360ChromeX.exe".into()],
            cancellation,
        )?,
        chromium_browser(
            "qq_browser",
            "QQ Browser",
            &qq_browser_root,
            first_existing_file(
                [
                    local.join("Tencent/QQBrowser/Application/QQBrowser.exe"),
                    local.join("Tencent/QQBrowser/QQBrowser.exe"),
                    local.join("QQBrowser/Application/QQBrowser.exe"),
                ]
                .into_iter()
                .chain(windows_program_files_paths(&[
                    "Tencent/QQBrowser/QQBrowser.exe",
                    "Tencent/QQBrowser/Application/QQBrowser.exe",
                ])),
            ),
            vec!["qqbrowser.exe".into()],
            cancellation,
        )?,
        chromium_browser(
            "yandex",
            "Yandex Browser",
            &local.join("Yandex/YandexBrowser/User Data"),
            windows_application_path(
                &local.join("Yandex/YandexBrowser/Application/browser.exe"),
                "Yandex/YandexBrowser/Application/browser.exe",
            ),
            vec!["browser.exe".into()],
            cancellation,
        )?,
        chromium_browser(
            "vivaldi",
            "Vivaldi",
            &local.join("Vivaldi/User Data"),
            windows_application_path(
                &local.join("Vivaldi/Application/vivaldi.exe"),
                "Vivaldi/Application/vivaldi.exe",
            ),
            vec!["vivaldi.exe".into()],
            cancellation,
        )?,
        chromium_browser(
            "chromium",
            "Chromium",
            &local.join("Chromium/User Data"),
            windows_application_path(
                &local.join("Chromium/Application/chrome.exe"),
                "Chromium/Application/chrome.exe",
            ),
            vec!["chrome.exe".into()],
            cancellation,
        )?,
        firefox_browser(
            &roaming.join("Mozilla/Firefox/Profiles"),
            windows_application_path(
                &local.join("Mozilla Firefox/firefox.exe"),
                "Mozilla Firefox/firefox.exe",
            ),
            cancellation,
        )?,
    ];
    let recent = super::directories::recent_items_directory()?;
    let mut traces = vec![
        PlatformPrivacySystemTrace {
            provider_key: "current_clipboard".into(),
            display_name: "Current clipboard".into(),
            kind: PlatformPrivacySystemTraceKind::CurrentClipboard,
            roots: Vec::new(),
            all_time_only: false,
            available: true,
            item_count: clipboard_format_count(),
            revision: clipboard_revision(),
        },
        clipboard_history_trace(),
        recent_document_history_trace(&recent),
    ];
    if let Some(trace) = jump_lists_trace(&roaming) {
        traces.push(trace);
    }
    if let Some(trace) = shell_history_trace(&roaming, dirs::home_dir().as_deref()) {
        traces.push(trace);
    }
    traces.extend([
        registry_trace(
            "run_dialog_history",
            "Run dialog history",
            PlatformPrivacySystemTraceKind::RunDialogHistory,
            RUN_DIALOG_MRU,
        ),
        registry_trace(
            "file_dialog_history",
            "File dialog history",
            PlatformPrivacySystemTraceKind::FileDialogHistory,
            FILE_DIALOG_MRU,
        ),
        registry_trace(
            "explorer_search_history",
            "File Explorer search history",
            PlatformPrivacySystemTraceKind::ExplorerSearchHistory,
            EXPLORER_SEARCH_MRU,
        ),
        registry_trace(
            "explorer_path_history",
            "File Explorer path history",
            PlatformPrivacySystemTraceKind::ExplorerPathHistory,
            EXPLORER_TYPED_PATHS,
        ),
        registry_trace(
            "application_usage_history",
            "Application usage history",
            PlatformPrivacySystemTraceKind::ApplicationUsageHistory,
            APPLICATION_USAGE_HISTORY,
        ),
        registry_trace(
            "network_connection_history",
            "Network location history",
            PlatformPrivacySystemTraceKind::NetworkConnectionHistory,
            NETWORK_CONNECTION_HISTORY,
        ),
        folder_view_history_trace(),
        registry_trace(
            "printer_history",
            "Printer connection history",
            PlatformPrivacySystemTraceKind::PrinterHistory,
            PRINTER_HISTORY,
        ),
    ]);
    Ok(PlatformPrivacyDiscovery {
        browsers,
        applications: application_privacy_sources(&local, &roaming),
        system_traces: traces,
    })
}

pub(super) fn clear(trace: PlatformPrivacySystemTraceKind) -> PlatformResult<bool> {
    match trace {
        PlatformPrivacySystemTraceKind::CurrentClipboard => clear_clipboard(),
        PlatformPrivacySystemTraceKind::ClipboardHistory => {
            windows::ApplicationModel::DataTransfer::Clipboard::ClearHistory()
                .map_err(|_| PlatformError::operation_failed("clear clipboard history failed"))
        }
        // Core owns the fixed-directory mutation so it can revalidate every entry and refuse
        // reparse points. The native adapter never receives an arbitrary path from the UI.
        PlatformPrivacySystemTraceKind::RecentItems => Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "recent items are cleared through the Core file safety boundary",
        )),
        PlatformPrivacySystemTraceKind::ShellHistory
        | PlatformPrivacySystemTraceKind::JumpLists => Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "file-backed privacy traces are cleared through the Core safety boundary",
        )),
        PlatformPrivacySystemTraceKind::RunDialogHistory => clear_registry_paths(RUN_DIALOG_MRU),
        PlatformPrivacySystemTraceKind::FileDialogHistory => clear_registry_paths(FILE_DIALOG_MRU),
        PlatformPrivacySystemTraceKind::ExplorerSearchHistory => {
            clear_registry_paths(EXPLORER_SEARCH_MRU)
        }
        PlatformPrivacySystemTraceKind::ExplorerPathHistory => {
            clear_registry_paths(EXPLORER_TYPED_PATHS)
        }
        PlatformPrivacySystemTraceKind::RecentDocumentHistory => {
            clear_registry_paths(RECENT_DOCUMENT_HISTORY)
        }
        PlatformPrivacySystemTraceKind::ApplicationUsageHistory => {
            clear_registry_paths(APPLICATION_USAGE_HISTORY)
        }
        PlatformPrivacySystemTraceKind::NetworkConnectionHistory => {
            clear_registry_paths(NETWORK_CONNECTION_HISTORY)
        }
        PlatformPrivacySystemTraceKind::FolderViewHistory => {
            clear_registry_paths(FOLDER_VIEW_HISTORY)
        }
        PlatformPrivacySystemTraceKind::PrinterHistory => clear_registry_paths(PRINTER_HISTORY),
        PlatformPrivacySystemTraceKind::RecentApplications => Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "recent application history is unavailable on Windows",
        )),
    }
}

/// Reads only the native companion revision required by Core's destructive preflight.
/// Recent-document shortcuts and Explorer's RecentDocs registry tree are one product item, so
/// both sources must still match the scan before either side can be cleared.
pub(super) fn system_revision(
    trace: PlatformPrivacySystemTraceKind,
) -> PlatformResult<Option<String>> {
    match trace {
        PlatformPrivacySystemTraceKind::RecentDocumentHistory => {
            let root = RegKey::predef(HKEY_CURRENT_USER);
            registry_paths_snapshot(&root, RECENT_DOCUMENT_HISTORY)
                .map(|(_, revision)| Some(format!("registry:{revision}")))
                .map_err(|_| {
                    PlatformError::operation_failed("read recent document registry revision failed")
                })
        }
        _ => Ok(None),
    }
}

pub(super) fn clear_application_trace(
    trace: PlatformPrivacyApplicationNativeTraceKind,
) -> PlatformResult<bool> {
    if let Some(result) = packaged_app_history::clear(trace) {
        return result;
    }
    if let Some(result) = media_history::clear(trace) {
        return result;
    }
    if trace == PlatformPrivacyApplicationNativeTraceKind::WindowsVisualStudioCodeEditorHistory {
        let roaming = dirs::data_dir().ok_or_else(|| {
            PlatformError::invalid_path("roaming application data directory is unavailable")
        })?;
        return vscode_history::clear(&roaming.join("Code/User/History"));
    }
    if let Some((application, kind)) = office_history_spec(trace) {
        let registry_cleared = clear_registry_targets(application_registry_targets(trace))?;
        let cache_cleared = office_history::clear(&office_root()?, application, kind)
            .map_err(PlatformError::with_possible_side_effects)?;
        return Ok(registry_cleared && cache_cleared);
    }
    clear_registry_targets(application_registry_targets(trace))
}

pub(super) fn system_details(
    trace: PlatformPrivacySystemTraceKind,
    offset: u64,
    limit: u32,
) -> PlatformResult<Vec<PlatformPrivacyDetailEntry>> {
    let paths = match trace {
        PlatformPrivacySystemTraceKind::RunDialogHistory => RUN_DIALOG_MRU,
        PlatformPrivacySystemTraceKind::FileDialogHistory => FILE_DIALOG_MRU,
        PlatformPrivacySystemTraceKind::ExplorerSearchHistory => EXPLORER_SEARCH_MRU,
        PlatformPrivacySystemTraceKind::ExplorerPathHistory => EXPLORER_TYPED_PATHS,
        // File-backed recent-document details are resolved by Core. ShellBag records contain
        // opaque namespace identifiers, so returning no entries intentionally selects the honest
        // aggregate-only presentation instead of exposing configuration fields as folder names.
        PlatformPrivacySystemTraceKind::RecentDocumentHistory
        | PlatformPrivacySystemTraceKind::FolderViewHistory => return Ok(Vec::new()),
        PlatformPrivacySystemTraceKind::ApplicationUsageHistory => APPLICATION_USAGE_HISTORY,
        PlatformPrivacySystemTraceKind::NetworkConnectionHistory => NETWORK_CONNECTION_HISTORY,
        PlatformPrivacySystemTraceKind::PrinterHistory => PRINTER_HISTORY,
        // Clipboard content is intentionally never read for a preview. File-backed traces are
        // listed by Core from their already-authorized roots.
        _ => return Ok(Vec::new()),
    };
    registry_path_details(paths, offset, limit)
}

pub(super) fn application_details(
    trace: PlatformPrivacyApplicationNativeTraceKind,
    offset: u64,
    limit: u32,
) -> PlatformResult<Vec<PlatformPrivacyDetailEntry>> {
    if let Some(result) = packaged_app_history::details(trace, offset, limit) {
        return result;
    }
    if let Some(result) = media_history::details(trace, offset, limit) {
        return result;
    }
    if trace == PlatformPrivacyApplicationNativeTraceKind::WindowsVisualStudioCodeEditorHistory {
        let roaming = dirs::data_dir().ok_or_else(|| {
            PlatformError::invalid_path("roaming application data directory is unavailable")
        })?;
        return vscode_history::details(&roaming.join("Code/User/History"), offset, limit);
    }
    if let Some((application, kind)) = office_history_spec(trace) {
        let root = office_root()?;
        if office_history::snapshot(&root, application, kind)?.item_count > 0 {
            return office_history::details(&root, application, kind, offset, limit);
        }
    }
    registry_target_details(application_registry_targets(trace), offset, limit)
}

#[derive(Clone, Copy)]
enum RegistryTarget {
    Key(&'static str),
    /// Registry state that belongs to the selected trace and must be removed, but does not
    /// represent another user-visible history item. Remote Desktop, for example, stores host
    /// certificate and username hints below `Servers` in addition to the single MRU entry shown in
    /// its connection picker.
    MetadataKey(&'static str),
    Value {
        path: &'static str,
        name: &'static str,
    },
    /// Microsoft 365 stores account-scoped MRU values below opaque account keys. Only values
    /// named `Item N` are user history; folder identifiers and other application state must stay.
    OfficeMru {
        path: &'static str,
        descendant_key: Option<&'static str>,
        application: &'static str,
    },
    /// Some applications store one logical record per direct child key and attach many metadata
    /// values to that child. Count and display the child once instead of inflating the result.
    DirectChildRecords {
        path: &'static str,
        label_value: &'static str,
    },
}

const REMOTE_DESKTOP_CONNECTIONS: &[RegistryTarget] = &[
    RegistryTarget::MetadataKey(r"Software\Microsoft\Terminal Server Client\Servers"),
    RegistryTarget::Value {
        path: r"Software\Microsoft\Terminal Server Client\Default",
        name: "MRU0",
    },
    RegistryTarget::Value {
        path: r"Software\Microsoft\Terminal Server Client\Default",
        name: "MRU1",
    },
    RegistryTarget::Value {
        path: r"Software\Microsoft\Terminal Server Client\Default",
        name: "MRU2",
    },
    RegistryTarget::Value {
        path: r"Software\Microsoft\Terminal Server Client\Default",
        name: "MRU3",
    },
    RegistryTarget::Value {
        path: r"Software\Microsoft\Terminal Server Client\Default",
        name: "MRU4",
    },
    RegistryTarget::Value {
        path: r"Software\Microsoft\Terminal Server Client\Default",
        name: "MRU5",
    },
    RegistryTarget::Value {
        path: r"Software\Microsoft\Terminal Server Client\Default",
        name: "MRU6",
    },
    RegistryTarget::Value {
        path: r"Software\Microsoft\Terminal Server Client\Default",
        name: "MRU7",
    },
    RegistryTarget::Value {
        path: r"Software\Microsoft\Terminal Server Client\Default",
        name: "MRU8",
    },
    RegistryTarget::Value {
        path: r"Software\Microsoft\Terminal Server Client\Default",
        name: "MRU9",
    },
];
const SEVEN_ZIP_RECENT_PATHS: &[RegistryTarget] = &[
    RegistryTarget::Value {
        path: r"Software\7-Zip\Compression",
        name: "ArcHistory",
    },
    RegistryTarget::Key(r"Software\7-Zip\Compression\ArcHistory"),
    RegistryTarget::Value {
        path: r"Software\7-Zip\Extraction",
        name: "PathHistory",
    },
    RegistryTarget::Key(r"Software\7-Zip\Extraction\PathHistory"),
    RegistryTarget::Value {
        path: r"Software\7-Zip\FM",
        name: "CopyHistory",
    },
    RegistryTarget::Value {
        path: r"Software\7-Zip\FM",
        name: "FolderHistory",
    },
    RegistryTarget::Value {
        path: r"Software\7-Zip\FM",
        name: "PanelPath0",
    },
    RegistryTarget::Value {
        path: r"Software\7-Zip\FM",
        name: "PanelPath1",
    },
];
// Office applications own independent MRU lists even when installed by one Microsoft 365 suite.
// Keeping separate allowlists prevents clearing Word from also deleting Excel or PowerPoint state.
const MICROSOFT_WORD_RECENT_DOCUMENTS: &[RegistryTarget] = &[
    RegistryTarget::Key(r"Software\Microsoft\Office\11.0\Word\File MRU"),
    RegistryTarget::Key(r"Software\Microsoft\Office\12.0\Word\File MRU"),
    RegistryTarget::Key(r"Software\Microsoft\Office\14.0\Word\File MRU"),
    RegistryTarget::Key(r"Software\Microsoft\Office\15.0\Word\File MRU"),
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\Word\File MRU",
        descendant_key: None,
        application: "Word",
    },
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\Word\User MRU",
        descendant_key: Some("File MRU"),
        application: "Word",
    },
];
const MICROSOFT_EXCEL_RECENT_DOCUMENTS: &[RegistryTarget] = &[
    RegistryTarget::Key(r"Software\Microsoft\Office\9.0\Excel\Recent Files"),
    RegistryTarget::Key(r"Software\Microsoft\Office\11.0\Excel\Recent Files"),
    RegistryTarget::Key(r"Software\Microsoft\Office\12.0\Excel\File MRU"),
    RegistryTarget::Key(r"Software\Microsoft\Office\14.0\Excel\File MRU"),
    RegistryTarget::Key(r"Software\Microsoft\Office\15.0\Excel\File MRU"),
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\Excel\File MRU",
        descendant_key: None,
        application: "Excel",
    },
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\Excel\User MRU",
        descendant_key: Some("File MRU"),
        application: "Excel",
    },
];
const MICROSOFT_POWERPOINT_RECENT_DOCUMENTS: &[RegistryTarget] = &[
    RegistryTarget::Key(r"Software\Microsoft\Office\9.0\PowerPoint\Recent File List"),
    RegistryTarget::Key(r"Software\Microsoft\Office\11.0\PowerPoint\Recent File List"),
    RegistryTarget::Key(r"Software\Microsoft\Office\12.0\PowerPoint\File MRU"),
    RegistryTarget::Key(r"Software\Microsoft\Office\14.0\PowerPoint\File MRU"),
    RegistryTarget::Key(r"Software\Microsoft\Office\15.0\PowerPoint\File MRU"),
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\PowerPoint\File MRU",
        descendant_key: None,
        application: "PowerPoint",
    },
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\PowerPoint\User MRU",
        descendant_key: Some("File MRU"),
        application: "PowerPoint",
    },
];
const MICROSOFT_WORD_RECENT_PATHS: &[RegistryTarget] = &[
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\Word\Place MRU",
        descendant_key: None,
        application: "Word",
    },
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\Word\User MRU",
        descendant_key: Some("Place MRU"),
        application: "Word",
    },
];
const MICROSOFT_EXCEL_RECENT_PATHS: &[RegistryTarget] = &[
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\Excel\Place MRU",
        descendant_key: None,
        application: "Excel",
    },
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\Excel\User MRU",
        descendant_key: Some("Place MRU"),
        application: "Excel",
    },
];
const MICROSOFT_POWERPOINT_RECENT_PATHS: &[RegistryTarget] = &[
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\PowerPoint\Place MRU",
        descendant_key: None,
        application: "PowerPoint",
    },
    RegistryTarget::OfficeMru {
        path: r"Software\Microsoft\Office\16.0\PowerPoint\User MRU",
        descendant_key: Some("Place MRU"),
        application: "PowerPoint",
    },
];
const MICROSOFT_WORD_RECENT_SEARCHES: &[RegistryTarget] = &[
    RegistryTarget::Key(r"Software\Microsoft\Office\12.0\Common\Open Find\Microsoft Word"),
    RegistryTarget::Key(r"Software\Microsoft\Office\14.0\Common\Open Find\Microsoft Word"),
    RegistryTarget::Key(r"Software\Microsoft\Office\15.0\Common\Open Find\Microsoft Word"),
    RegistryTarget::Key(r"Software\Microsoft\Office\16.0\Common\Open Find\Microsoft Word"),
];
const MICROSOFT_EXCEL_RECENT_SEARCHES: &[RegistryTarget] = &[
    RegistryTarget::Key(r"Software\Microsoft\Office\12.0\Common\Open Find\Microsoft Excel"),
    RegistryTarget::Key(r"Software\Microsoft\Office\14.0\Common\Open Find\Microsoft Excel"),
    RegistryTarget::Key(r"Software\Microsoft\Office\15.0\Common\Open Find\Microsoft Excel"),
    RegistryTarget::Key(r"Software\Microsoft\Office\16.0\Common\Open Find\Microsoft Excel"),
];
const MICROSOFT_POWERPOINT_RECENT_SEARCHES: &[RegistryTarget] = &[
    RegistryTarget::Key(r"Software\Microsoft\Office\12.0\Common\Open Find\Microsoft PowerPoint"),
    RegistryTarget::Key(r"Software\Microsoft\Office\14.0\Common\Open Find\Microsoft PowerPoint"),
    RegistryTarget::Key(r"Software\Microsoft\Office\15.0\Common\Open Find\Microsoft PowerPoint"),
    RegistryTarget::Key(r"Software\Microsoft\Office\16.0\Common\Open Find\Microsoft PowerPoint"),
];
const WORDPAD_RECENT_DOCUMENTS: &[RegistryTarget] = &[RegistryTarget::Key(
    r"Software\Microsoft\Windows\CurrentVersion\Applets\Wordpad\Recent File List",
)];
const ADOBE_READER_RECENT_DOCUMENTS: &[RegistryTarget] = &[
    RegistryTarget::DirectChildRecords {
        path: r"Software\Adobe\Acrobat Reader\DC\AVGeneral\cRecentFiles",
        label_value: "tFileName",
    },
    RegistryTarget::DirectChildRecords {
        path: r"Software\Adobe\Acrobat Reader\2020\AVGeneral\cRecentFiles",
        label_value: "tFileName",
    },
    RegistryTarget::DirectChildRecords {
        path: r"Software\Adobe\Acrobat Reader\2017\AVGeneral\cRecentFiles",
        label_value: "tFileName",
    },
    RegistryTarget::DirectChildRecords {
        path: r"Software\Adobe\Acrobat Reader\2015\AVGeneral\cRecentFiles",
        label_value: "tFileName",
    },
    RegistryTarget::DirectChildRecords {
        path: r"Software\Adobe\Acrobat Reader\11.0\AVGeneral\cRecentFiles",
        label_value: "tFileName",
    },
];
const WPS_OFFICE_RECENT_DOCUMENTS: &[RegistryTarget] = &[
    // `wpsoffice` is the suite-wide visible list. Product-specific keys duplicate the same
    // records and are cleanup metadata only, otherwise one document would be counted repeatedly.
    RegistryTarget::Key(r"Software\Kingsoft\Office\6.0\plugins\ksomisc\RecentFiles\wpsoffice"),
    RegistryTarget::MetadataKey(r"Software\Kingsoft\Office\6.0\wps\RecentFiles"),
    RegistryTarget::MetadataKey(r"Software\Kingsoft\Office\6.0\et\RecentFiles"),
    RegistryTarget::MetadataKey(r"Software\Kingsoft\Office\6.0\wpp\RecentFiles"),
    RegistryTarget::MetadataKey(r"Software\Kingsoft\Office\6.0\pdf\RecentFiles"),
    RegistryTarget::MetadataKey(r"Software\Kingsoft\Office\6.0\ofd\RecentFiles"),
    RegistryTarget::MetadataKey(r"Software\Kingsoft\Office\6.0\plugins\ksomisc\RecentFiles\wps"),
    RegistryTarget::MetadataKey(r"Software\Kingsoft\Office\6.0\plugins\ksomisc\RecentFiles\et"),
    RegistryTarget::MetadataKey(r"Software\Kingsoft\Office\6.0\plugins\ksomisc\RecentFiles\wpp"),
    RegistryTarget::MetadataKey(r"Software\Kingsoft\Office\6.0\plugins\ksomisc\RecentFiles\pdf"),
];
const WPS_OFFICE_RECENT_FOLDERS: &[RegistryTarget] = &[RegistryTarget::DirectChildRecords {
    path: r"Software\Kingsoft\Office\6.0\Common\FileDialog\RecentFolder",
    label_value: "path",
}];
const TEAMVIEWER_RECENT_CONNECTIONS: &[RegistryTarget] = &[
    RegistryTarget::Value {
        path: r"Software\TeamViewer",
        name: "Last_Machine_Connections",
    },
    RegistryTarget::Value {
        path: r"Software\TeamViewer",
        name: "Last_Machine_Connections_UD",
    },
    RegistryTarget::Value {
        path: r"Software\TeamViewer\Version15",
        name: "Last_Machine_Connections",
    },
    RegistryTarget::Value {
        path: r"Software\TeamViewer\Version14",
        name: "Last_Machine_Connections",
    },
    RegistryTarget::Value {
        path: r"Software\TeamViewer\Version13",
        name: "Last_Machine_Connections",
    },
    RegistryTarget::Value {
        path: r"Software\TeamViewer\Version12",
        name: "Last_Machine_Connections",
    },
    RegistryTarget::Value {
        path: r"Software\TeamViewer\Version11",
        name: "Last_Machine_Connections",
    },
    RegistryTarget::Value {
        path: r"Software\TeamViewer\Version10",
        name: "Last_Machine_Connections",
    },
    RegistryTarget::Value {
        path: r"Software\TeamViewer\Version9",
        name: "Last_Machine_Connections",
    },
];
const TORTOISE_SVN_HISTORY: &[RegistryTarget] =
    &[RegistryTarget::Key(r"Software\TortoiseSVN\History")];
const WIN_RAR_HISTORY: &[RegistryTarget] = &[
    RegistryTarget::Key(r"Software\WinRAR\ArcHistory"),
    RegistryTarget::Key(r"Software\WinRAR\DialogEditHistory\ArcName"),
    RegistryTarget::Key(r"Software\WinRAR\DialogEditHistory\ArcCmtName"),
    RegistryTarget::Key(r"Software\WinRAR\DialogEditHistory\ExtrPath"),
    RegistryTarget::Key(r"Software\WinRAR\DialogEditHistory\FindArcNames"),
    RegistryTarget::Key(r"Software\WinRAR\DialogEditHistory\FindNames"),
    RegistryTarget::Key(r"Software\WinRAR\DialogEditHistory\FindText"),
    RegistryTarget::Key(r"Software\WinRAR\DialogEditHistory\WizArcName"),
    RegistryTarget::Value {
        path: r"Software\WinRAR\General",
        name: "LastFolder",
    },
];
const WIN_ZIP_RECENT_ARCHIVES: &[RegistryTarget] = &[
    RegistryTarget::Key(r"Software\Nico Mak Computing\WinZip\mru\jobs"),
    RegistryTarget::Key(r"Software\Nico Mak Computing\WinZip\mru\archives"),
];

fn application_registry_targets(
    trace: PlatformPrivacyApplicationNativeTraceKind,
) -> &'static [RegistryTarget] {
    match trace {
        PlatformPrivacyApplicationNativeTraceKind::MacPreviewRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::MacPdfExpertRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::MacTextEditRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::MacSkimRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::MacWpsOfficeRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::MacPagesRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::MacNumbersRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::MacKeynoteRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::MacVisualStudioCodeRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::MacVisualStudioCodeEditorHistory
        | PlatformPrivacyApplicationNativeTraceKind::MacXcodeRecentProjects
        | PlatformPrivacyApplicationNativeTraceKind::MacVlcRecentMedia
        | PlatformPrivacyApplicationNativeTraceKind::MacMovistRecentMedia
        | PlatformPrivacyApplicationNativeTraceKind::MacQuickTimeRecentMedia
        | PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftWordRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftExcelRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftPowerPointRecentDocuments
        | PlatformPrivacyApplicationNativeTraceKind::WindowsVisualStudioCodeEditorHistory
        | PlatformPrivacyApplicationNativeTraceKind::WindowsModernMediaPlayerRecentMedia
        | PlatformPrivacyApplicationNativeTraceKind::WindowsVlcRecentMedia
        | PlatformPrivacyApplicationNativeTraceKind::WindowsPotPlayerRecentMedia
        | PlatformPrivacyApplicationNativeTraceKind::WindowsNotepadSessionHistory
        | PlatformPrivacyApplicationNativeTraceKind::PaintRecentDocuments => &[],
        PlatformPrivacyApplicationNativeTraceKind::RemoteDesktopConnections => {
            REMOTE_DESKTOP_CONNECTIONS
        }
        PlatformPrivacyApplicationNativeTraceKind::SevenZipRecentPaths => SEVEN_ZIP_RECENT_PATHS,
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftWordRecentDocuments => {
            MICROSOFT_WORD_RECENT_DOCUMENTS
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftWordRecentPaths => {
            MICROSOFT_WORD_RECENT_PATHS
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftWordRecentSearches => {
            MICROSOFT_WORD_RECENT_SEARCHES
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftExcelRecentDocuments => {
            MICROSOFT_EXCEL_RECENT_DOCUMENTS
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftExcelRecentPaths => {
            MICROSOFT_EXCEL_RECENT_PATHS
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftExcelRecentSearches => {
            MICROSOFT_EXCEL_RECENT_SEARCHES
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftPowerPointRecentDocuments => {
            MICROSOFT_POWERPOINT_RECENT_DOCUMENTS
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftPowerPointRecentPaths => {
            MICROSOFT_POWERPOINT_RECENT_PATHS
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftPowerPointRecentSearches => {
            MICROSOFT_POWERPOINT_RECENT_SEARCHES
        }
        PlatformPrivacyApplicationNativeTraceKind::WpsOfficeRecentDocuments => {
            WPS_OFFICE_RECENT_DOCUMENTS
        }
        PlatformPrivacyApplicationNativeTraceKind::WpsOfficeRecentFolders => {
            WPS_OFFICE_RECENT_FOLDERS
        }
        PlatformPrivacyApplicationNativeTraceKind::WordPadRecentDocuments => {
            WORDPAD_RECENT_DOCUMENTS
        }
        PlatformPrivacyApplicationNativeTraceKind::AdobeReaderRecentDocuments => {
            ADOBE_READER_RECENT_DOCUMENTS
        }
        PlatformPrivacyApplicationNativeTraceKind::TeamViewerRecentConnections => {
            TEAMVIEWER_RECENT_CONNECTIONS
        }
        PlatformPrivacyApplicationNativeTraceKind::TortoiseSvnHistory => TORTOISE_SVN_HISTORY,
        PlatformPrivacyApplicationNativeTraceKind::WinRarHistory => WIN_RAR_HISTORY,
        PlatformPrivacyApplicationNativeTraceKind::WinZipRecentArchives => WIN_ZIP_RECENT_ARCHIVES,
    }
}

fn application_privacy_sources(local: &Path, roaming: &Path) -> Vec<PlatformPrivacyApplication> {
    let mut applications = Vec::new();

    for definition in [
        WindowsElectronApplicationDefinition {
            provider_key: "vscode",
            display_name: "Visual Studio Code",
            data_directory: "Code",
            local_executable: "Programs/Microsoft VS Code/Code.exe",
            program_files_executable: "Microsoft VS Code/Code.exe",
            process_names: &["Code.exe"],
            editor_history: true,
        },
        WindowsElectronApplicationDefinition {
            provider_key: "vscodium",
            display_name: "VSCodium",
            data_directory: "VSCodium",
            local_executable: "Programs/VSCodium/VSCodium.exe",
            program_files_executable: "VSCodium/VSCodium.exe",
            process_names: &["VSCodium.exe"],
            editor_history: true,
        },
        WindowsElectronApplicationDefinition {
            provider_key: "notion",
            display_name: "Notion",
            data_directory: "Notion",
            local_executable: "Programs/Notion/Notion.exe",
            program_files_executable: "Notion/Notion.exe",
            process_names: &["Notion.exe"],
            editor_history: false,
        },
        WindowsElectronApplicationDefinition {
            provider_key: "obsidian",
            display_name: "Obsidian",
            data_directory: "obsidian",
            local_executable: "Programs/Obsidian/Obsidian.exe",
            program_files_executable: "Obsidian/Obsidian.exe",
            process_names: &["Obsidian.exe"],
            editor_history: false,
        },
    ] {
        let data_root = roaming.join(definition.data_directory);
        let mut traces = vec![
            file_application_trace(
                definition.provider_key,
                "Application cache",
                PlatformPrivacyApplicationTraceKind::Cache,
                [
                    data_root.join("Cache"),
                    data_root.join("Code Cache"),
                    data_root.join("GPUCache"),
                    data_root.join("CachedConfigurations"),
                    data_root.join("CachedData"),
                    data_root.join("CachedExtensionVSIXs"),
                    data_root.join("CachedProfilesData"),
                    data_root.join("Crashpad"),
                    data_root.join("DawnGraphiteCache"),
                    data_root.join("DawnWebGPUCache"),
                    data_root.join("DawnCache"),
                    data_root.join("blob_storage"),
                    data_root.join("Network Persistent State"),
                ],
            ),
            file_application_trace(
                definition.provider_key,
                "Application logs",
                PlatformPrivacyApplicationTraceKind::Logs,
                [data_root.join("logs")],
            ),
            file_application_trace(
                definition.provider_key,
                "Application sessions",
                PlatformPrivacyApplicationTraceKind::Sessions,
                [data_root.join("Session Storage")],
            ),
        ];
        if definition.editor_history {
            if definition.provider_key == "vscode" {
                traces.push(native_windows_vscode_history_trace(&data_root));
            } else {
                traces.push(file_application_trace(
                    definition.provider_key,
                    "Editor local history",
                    PlatformPrivacyApplicationTraceKind::EditorLocalHistory,
                    [data_root.join("User/History")],
                ));
            }
        }
        let local_executable = local.join(definition.local_executable);
        let application_path =
            windows_application_path(&local_executable, definition.program_files_executable)
                .or_else(|| {
                    local_executable.parent().and_then(|parent| {
                        local_executable
                            .file_name()
                            .and_then(|name| first_versioned_executable(parent, name))
                    })
                });
        push_installed_application(
            &mut applications,
            PlatformPrivacyApplication {
                provider_key: definition.provider_key.into(),
                display_name: definition.display_name.into(),
                application_path,
                process_names: definition
                    .process_names
                    .iter()
                    .map(|name| (*name).into())
                    .collect(),
                traces,
            },
        );
    }

    let vlc_path = first_existing_file(windows_program_files_paths(&["VideoLAN/VLC/vlc.exe"]));
    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "vlc".into(),
            display_name: "VLC".into(),
            application_path: vlc_path,
            process_names: vec!["vlc.exe".into()],
            traces: vec![native_application_trace(
                "vlc",
                "Playback history",
                PlatformPrivacyApplicationTraceKind::PlaybackHistory,
                PlatformPrivacyApplicationNativeTraceKind::WindowsVlcRecentMedia,
            )],
        },
    );

    let media_player_data =
        local.join("Packages/Microsoft.ZuneMusic_8wekyb3d8bbwe/LocalState/MediaPlayer.db");
    // Media Player is an MSIX application. Its ARM64 executable does not always expose a shell
    // icon through SHDefExtractIcon, while the manifest-declared AppList asset is stable and can
    // be decoded by the shared icon service. Keep the executable as a compatibility fallback for
    // package builds that do not contain the declared asset family.
    let media_player_path =
        windows_appx_declared_icon("Microsoft.ZuneMusic_", "Assets/MediaPlayerAppList.png")
            .or_else(|| {
                windows_appx_executable("Microsoft.ZuneMusic_", "Microsoft.Media.Player.exe")
            });
    if media_player_data.is_file() || media_player_path.is_some() {
        applications.push(PlatformPrivacyApplication {
            provider_key: "windows_media_player".into(),
            display_name: "Media Player".into(),
            application_path: media_player_path,
            process_names: vec!["Microsoft.Media.Player.exe".into(), "Music.UI.exe".into()],
            traces: vec![native_application_trace(
                "windows_media_player",
                "Playback history",
                PlatformPrivacyApplicationTraceKind::PlaybackHistory,
                PlatformPrivacyApplicationNativeTraceKind::WindowsModernMediaPlayerRecentMedia,
            )],
        });
    }

    let potplayer_path = first_existing_file(windows_program_files_paths(&[
        "DAUM/PotPlayer/PotPlayerMini64.exe",
        "DAUM/PotPlayer/PotPlayerMini.exe",
    ]));
    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "potplayer".into(),
            display_name: "PotPlayer".into(),
            application_path: potplayer_path,
            process_names: vec!["PotPlayerMini64.exe".into(), "PotPlayerMini.exe".into()],
            traces: vec![native_application_trace(
                "potplayer",
                "Playback history",
                PlatformPrivacyApplicationTraceKind::PlaybackHistory,
                PlatformPrivacyApplicationNativeTraceKind::WindowsPotPlayerRecentMedia,
            )],
        },
    );

    let wps_data = roaming.join("Kingsoft/office6");
    let wps_path = first_existing_file(windows_program_files_paths(&[
        "Kingsoft/WPS Office/office6/wpsoffice.exe",
        "Kingsoft/WPS Office/office6/wps.exe",
    ]))
    .or_else(|| {
        first_child_relative_file(
            &local.join("Kingsoft/WPS Office"),
            &["office6/wpsoffice.exe", "office6/wps.exe"],
        )
    });
    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "wps_office".into(),
            display_name: "WPS Office".into(),
            application_path: wps_path,
            process_names: [
                "wpsoffice.exe",
                "wps.exe",
                "et.exe",
                "wpp.exe",
                "wpspdf.exe",
                "wpscloudsvr.exe",
                "wpscenter.exe",
                "promecefpluginhost.exe",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            traces: vec![
                native_application_trace(
                    "wps_office",
                    "Recent documents",
                    PlatformPrivacyApplicationTraceKind::RecentDocuments,
                    PlatformPrivacyApplicationNativeTraceKind::WpsOfficeRecentDocuments,
                ),
                native_application_trace(
                    "wps_office",
                    "Recent folders",
                    PlatformPrivacyApplicationTraceKind::RecentPaths,
                    PlatformPrivacyApplicationNativeTraceKind::WpsOfficeRecentFolders,
                ),
                file_application_trace(
                    "wps_office",
                    "Application cache",
                    PlatformPrivacyApplicationTraceKind::Cache,
                    [wps_data.join("cache")],
                ),
                file_application_trace(
                    "wps_office",
                    "Application logs",
                    PlatformPrivacyApplicationTraceKind::Logs,
                    [wps_data.join("log")],
                ),
            ],
        },
    );

    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "notepad".into(),
            display_name: "Notepad".into(),
            application_path: windows_appx_executable(
                "Microsoft.WindowsNotepad_",
                "Notepad/Notepad.exe",
            )
            .or_else(|| windows_system_executable("notepad.exe")),
            process_names: vec!["Notepad.exe".into()],
            traces: vec![native_application_trace(
                "notepad",
                "Open tabs and drafts",
                PlatformPrivacyApplicationTraceKind::EditorLocalHistory,
                PlatformPrivacyApplicationNativeTraceKind::WindowsNotepadSessionHistory,
            )],
        },
    );

    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "paint".into(),
            display_name: "Paint".into(),
            application_path: windows_appx_executable("Microsoft.Paint_", "PaintApp/mspaint.exe")
                .or_else(|| windows_system_executable("mspaint.exe")),
            process_names: vec!["mspaint.exe".into()],
            traces: vec![native_application_trace(
                "paint",
                "Recent documents",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                PlatformPrivacyApplicationNativeTraceKind::PaintRecentDocuments,
            )],
        },
    );

    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "wordpad".into(),
            display_name: "WordPad".into(),
            application_path: windows_system_executable("write.exe"),
            process_names: vec!["wordpad.exe".into()],
            traces: vec![native_application_trace(
                "wordpad",
                "Recent documents",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                PlatformPrivacyApplicationNativeTraceKind::WordPadRecentDocuments,
            )],
        },
    );

    for definition in [
        (
            "word",
            "Microsoft Word",
            [
                "Microsoft Office/root/Office16/WINWORD.EXE",
                "Microsoft Office/Office16/WINWORD.EXE",
            ],
            "WINWORD.EXE",
            PlatformPrivacyApplicationNativeTraceKind::MicrosoftWordRecentDocuments,
            PlatformPrivacyApplicationNativeTraceKind::MicrosoftWordRecentPaths,
            PlatformPrivacyApplicationNativeTraceKind::MicrosoftWordRecentSearches,
        ),
        (
            "excel",
            "Microsoft Excel",
            [
                "Microsoft Office/root/Office16/EXCEL.EXE",
                "Microsoft Office/Office16/EXCEL.EXE",
            ],
            "EXCEL.EXE",
            PlatformPrivacyApplicationNativeTraceKind::MicrosoftExcelRecentDocuments,
            PlatformPrivacyApplicationNativeTraceKind::MicrosoftExcelRecentPaths,
            PlatformPrivacyApplicationNativeTraceKind::MicrosoftExcelRecentSearches,
        ),
        (
            "powerpoint",
            "Microsoft PowerPoint",
            [
                "Microsoft Office/root/Office16/POWERPNT.EXE",
                "Microsoft Office/Office16/POWERPNT.EXE",
            ],
            "POWERPNT.EXE",
            PlatformPrivacyApplicationNativeTraceKind::MicrosoftPowerPointRecentDocuments,
            PlatformPrivacyApplicationNativeTraceKind::MicrosoftPowerPointRecentPaths,
            PlatformPrivacyApplicationNativeTraceKind::MicrosoftPowerPointRecentSearches,
        ),
    ] {
        push_installed_application(
            &mut applications,
            PlatformPrivacyApplication {
                provider_key: definition.0.into(),
                display_name: definition.1.into(),
                application_path: first_existing_file(windows_program_files_paths(&definition.2)),
                process_names: vec![definition.3.into()],
                traces: vec![
                    native_application_trace(
                        definition.0,
                        "Recent documents",
                        PlatformPrivacyApplicationTraceKind::RecentDocuments,
                        definition.4,
                    ),
                    native_application_trace(
                        definition.0,
                        "Recent locations",
                        PlatformPrivacyApplicationTraceKind::RecentPaths,
                        definition.5,
                    ),
                    native_application_trace(
                        definition.0,
                        "Recent searches",
                        PlatformPrivacyApplicationTraceKind::RecentSearches,
                        definition.6,
                    ),
                ],
            },
        );
    }

    let adobe_path = first_existing_file(windows_program_files_paths(&[
        "Adobe/Acrobat DC/Acrobat/Acrobat.exe",
        "Adobe/Acrobat Reader DC/Reader/AcroRd32.exe",
    ]));
    let mut adobe_cache_roots = version_child_paths(&local.join("Adobe/Acrobat"), "Cache");
    if let Some(app_data) = local.parent() {
        adobe_cache_roots.extend(version_child_paths(
            &app_data.join("LocalLow/Adobe/Acrobat"),
            "Search",
        ));
    }
    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "adobe_reader".into(),
            display_name: "Adobe Acrobat Reader".into(),
            application_path: adobe_path,
            process_names: vec!["Acrobat.exe".into(), "AcroRd32.exe".into()],
            traces: vec![
                native_application_trace(
                    "adobe_reader",
                    "Recent documents",
                    PlatformPrivacyApplicationTraceKind::RecentDocuments,
                    PlatformPrivacyApplicationNativeTraceKind::AdobeReaderRecentDocuments,
                ),
                file_application_trace(
                    "adobe_reader",
                    "Application cache",
                    PlatformPrivacyApplicationTraceKind::Cache,
                    adobe_cache_roots,
                ),
            ],
        },
    );

    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "openoffice".into(),
            display_name: "Apache OpenOffice".into(),
            application_path: first_existing_file(windows_program_files_paths(&[
                "OpenOffice 4/program/soffice.exe",
                "OpenOffice.org 3/program/soffice.exe",
            ])),
            process_names: vec!["soffice.exe".into(), "soffice.bin".into()],
            traces: vec![file_application_trace(
                "openoffice",
                "Recent documents",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                [
                    roaming.join(
                        "OpenOffice.org/4/user/registry/data/org/openoffice/Office/Histories.xcu",
                    ),
                    roaming.join(
                        "OpenOffice.org/4/user/registry/cache/org.openoffice.Office.Histories.dat",
                    ),
                    roaming.join(
                        "OpenOffice.org/4/user/registry/cache/org.openoffice.Office.Common.dat",
                    ),
                    roaming.join(
                        "OpenOffice.org/3/user/registry/data/org/openoffice/Office/Histories.xcu",
                    ),
                    roaming.join(
                        "OpenOffice.org/3/user/registry/cache/org.openoffice.Office.Histories.dat",
                    ),
                    roaming.join(
                        "OpenOffice.org/3/user/registry/cache/org.openoffice.Office.Common.dat",
                    ),
                ],
            )],
        },
    );

    if let Some(home) = dirs::home_dir() {
        push_installed_application(
            &mut applications,
            PlatformPrivacyApplication {
                provider_key: "vim".into(),
                display_name: "Vim".into(),
                application_path: first_existing_file(windows_program_files_paths(&[
                    "Vim/vim91/gvim.exe",
                    "Vim/vim90/gvim.exe",
                ])),
                process_names: vec!["vim.exe".into(), "gvim.exe".into()],
                traces: vec![file_application_trace(
                    "vim",
                    "Editor local history",
                    PlatformPrivacyApplicationTraceKind::EditorLocalHistory,
                    [home.join("_viminfo")],
                )],
            },
        );
    }

    applications
}

struct WindowsElectronApplicationDefinition {
    provider_key: &'static str,
    display_name: &'static str,
    data_directory: &'static str,
    local_executable: &'static str,
    program_files_executable: &'static str,
    process_names: &'static [&'static str],
    editor_history: bool,
}

fn file_application_trace(
    application_key: &str,
    display_name: &str,
    kind: PlatformPrivacyApplicationTraceKind,
    candidates: impl IntoIterator<Item = PathBuf>,
) -> PlatformPrivacyApplicationTrace {
    PlatformPrivacyApplicationTrace {
        provider_key: format!("{application_key}:{}", application_trace_key(kind)),
        display_name: display_name.into(),
        kind,
        roots: candidates
            .into_iter()
            .filter(|path| existing_path_without_reparse_points(path))
            .collect(),
        native_kind: None,
        all_time_only: true,
        availability: PlatformPrivacyApplicationTraceAvailability::Available,
        item_count: 0,
        revision: String::new(),
    }
}

/// Resolves one fixed child path below every installed product-version directory.
/// Adobe stores regenerable cache and search indexes under versioned folders, so enumerating the
/// immediate children keeps the allowlist narrow without hard-coding versions that quickly age.
fn version_child_paths(version_root: &Path, child: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(version_root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(child))
        .collect()
}

/// Resolves a fixed executable suffix below direct product-version directories.
/// Reading one directory level keeps privacy scans bounded while supporting versioned per-user
/// installations without searching unrelated files.
fn first_child_relative_file(root: &Path, relative_paths: &[&str]) -> Option<PathBuf> {
    fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| {
            existing_path_without_reparse_points(&entry.path()) && entry.path().is_dir()
        })
        .flat_map(|entry| {
            relative_paths
                .iter()
                .map(move |relative| entry.path().join(relative))
        })
        .find(|path| path.is_file())
}

fn native_application_trace(
    application_key: &str,
    display_name: &str,
    kind: PlatformPrivacyApplicationTraceKind,
    native_kind: PlatformPrivacyApplicationNativeTraceKind,
) -> PlatformPrivacyApplicationTrace {
    let (availability, item_count, revision) =
        if let Some(snapshot) = packaged_app_history::snapshot(native_kind) {
            match snapshot {
                Ok(snapshot) => (
                    PlatformPrivacyApplicationTraceAvailability::Available,
                    snapshot.item_count,
                    snapshot.revision,
                ),
                Err(error) => (
                    if error.code() == PlatformErrorCode::AccessDenied {
                        PlatformPrivacyApplicationTraceAvailability::PermissionRequired
                    } else {
                        PlatformPrivacyApplicationTraceAvailability::Unavailable
                    },
                    0,
                    "unavailable".into(),
                ),
            }
        } else if let Some(snapshot) = media_history::snapshot(native_kind) {
            match snapshot {
                Ok(snapshot) => (
                    PlatformPrivacyApplicationTraceAvailability::Available,
                    snapshot.item_count,
                    snapshot.revision,
                ),
                Err(error) => (
                    if error.code() == PlatformErrorCode::AccessDenied {
                        PlatformPrivacyApplicationTraceAvailability::PermissionRequired
                    } else {
                        PlatformPrivacyApplicationTraceAvailability::Unavailable
                    },
                    0,
                    "unavailable".into(),
                ),
            }
        } else if let Some((application, history_kind)) = office_history_spec(native_kind) {
            match office_history_snapshot(native_kind, application, history_kind) {
                Ok((count, revision)) => (
                    PlatformPrivacyApplicationTraceAvailability::Available,
                    count,
                    revision,
                ),
                Err(error) => (
                    if error.code() == PlatformErrorCode::AccessDenied {
                        PlatformPrivacyApplicationTraceAvailability::PermissionRequired
                    } else {
                        PlatformPrivacyApplicationTraceAvailability::Unavailable
                    },
                    0,
                    "unavailable".into(),
                ),
            }
        } else {
            match registry_targets_snapshot(application_registry_targets(native_kind)) {
                Ok((count, revision)) => (
                    PlatformPrivacyApplicationTraceAvailability::Available,
                    count,
                    revision,
                ),
                Err(error) => (
                    if error.kind() == std::io::ErrorKind::PermissionDenied {
                        PlatformPrivacyApplicationTraceAvailability::PermissionRequired
                    } else {
                        PlatformPrivacyApplicationTraceAvailability::Unavailable
                    },
                    0,
                    "unavailable".into(),
                ),
            }
        };
    PlatformPrivacyApplicationTrace {
        provider_key: format!("{application_key}:{}", application_trace_key(kind)),
        display_name: display_name.into(),
        kind,
        roots: Vec::new(),
        native_kind: Some(native_kind),
        all_time_only: true,
        availability,
        item_count,
        revision,
    }
}

fn office_root() -> PlatformResult<PathBuf> {
    dirs::data_local_dir()
        .map(|root| root.join("Microsoft/Office/16.0"))
        .ok_or_else(|| PlatformError::invalid_path("Office local data directory is unavailable"))
}

fn office_history_spec(
    trace: PlatformPrivacyApplicationNativeTraceKind,
) -> Option<(office_history::Application, office_history::Kind)> {
    use office_history::{Application, Kind};

    match trace {
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftWordRecentDocuments => {
            Some((Application::Word, Kind::Documents))
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftWordRecentPaths => {
            Some((Application::Word, Kind::Places))
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftExcelRecentDocuments => {
            Some((Application::Excel, Kind::Documents))
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftExcelRecentPaths => {
            Some((Application::Excel, Kind::Places))
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftPowerPointRecentDocuments => {
            Some((Application::PowerPoint, Kind::Documents))
        }
        PlatformPrivacyApplicationNativeTraceKind::MicrosoftPowerPointRecentPaths => {
            Some((Application::PowerPoint, Kind::Places))
        }
        _ => None,
    }
}

/// New Microsoft 365 builds render their start-page list from `MruServiceCache`, while older or
/// offline builds still use the registry. Prefer the service cache when populated so one logical
/// document is not counted twice, but hash both sources so preflight detects either one changing.
fn office_history_snapshot(
    trace: PlatformPrivacyApplicationNativeTraceKind,
    application: office_history::Application,
    kind: office_history::Kind,
) -> PlatformResult<(u64, String)> {
    let cache = office_history::snapshot(&office_root()?, application, kind)?;
    let (registry_count, registry_revision) =
        registry_targets_snapshot(application_registry_targets(trace))
            .map_err(|error| PlatformError::io("scan Office registry MRU", &error))?;
    let mut revision = blake3::Hasher::new();
    revision.update(b"mangodisk-windows-office-combined-mru-v1\0");
    revision.update(cache.revision.as_bytes());
    revision.update(registry_revision.as_bytes());
    Ok((
        if cache.item_count > 0 {
            cache.item_count
        } else {
            registry_count
        },
        format!("office:{}", revision.finalize().to_hex()),
    ))
}

fn native_windows_vscode_history_trace(data_root: &Path) -> PlatformPrivacyApplicationTrace {
    let snapshot = vscode_history::snapshot(&data_root.join("User/History"));
    let (availability, item_count, revision) = match snapshot {
        Ok(snapshot) => (
            PlatformPrivacyApplicationTraceAvailability::Available,
            snapshot.item_count,
            snapshot.revision,
        ),
        Err(error) => {
            log::warn!("windows_vscode_history_scan_failed code={:?}", error.code());
            (
                if error.code() == PlatformErrorCode::AccessDenied {
                    PlatformPrivacyApplicationTraceAvailability::PermissionRequired
                } else {
                    PlatformPrivacyApplicationTraceAvailability::Unavailable
                },
                0,
                "unavailable".into(),
            )
        }
    };
    PlatformPrivacyApplicationTrace {
        provider_key: "vscode:editor_history".into(),
        display_name: "Editor local history".into(),
        kind: PlatformPrivacyApplicationTraceKind::EditorLocalHistory,
        roots: Vec::new(),
        native_kind: Some(
            PlatformPrivacyApplicationNativeTraceKind::WindowsVisualStudioCodeEditorHistory,
        ),
        all_time_only: true,
        availability,
        item_count,
        revision,
    }
}

fn application_trace_key(kind: PlatformPrivacyApplicationTraceKind) -> &'static str {
    match kind {
        PlatformPrivacyApplicationTraceKind::Cache => "cache",
        PlatformPrivacyApplicationTraceKind::Logs => "logs",
        PlatformPrivacyApplicationTraceKind::Sessions => "sessions",
        PlatformPrivacyApplicationTraceKind::EditorLocalHistory => "editor_history",
        PlatformPrivacyApplicationTraceKind::RecentDocuments => "recent_documents",
        PlatformPrivacyApplicationTraceKind::RecentProjects => "recent_projects",
        PlatformPrivacyApplicationTraceKind::RecentConnections => "recent_connections",
        PlatformPrivacyApplicationTraceKind::PlaybackHistory => "playback_history",
        PlatformPrivacyApplicationTraceKind::RecentPaths => "recent_paths",
        PlatformPrivacyApplicationTraceKind::RecentSearches => "recent_searches",
    }
}

fn existing_path_without_reparse_points(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        Err(error) => error.kind() == std::io::ErrorKind::PermissionDenied,
    }
}

fn push_installed_application(
    applications: &mut Vec<PlatformPrivacyApplication>,
    application: PlatformPrivacyApplication,
) {
    if application.application_path.is_some()
        || application.traces.iter().any(|trace| {
            !trace.roots.is_empty() || trace.native_kind.is_some_and(|_| trace.item_count > 0)
        })
    {
        applications.push(application);
    }
}

fn windows_system_executable(name: &str) -> Option<PathBuf> {
    std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .map(|root| root.join("System32").join(name))
        .filter(|path| path.is_file())
}

fn clipboard_history_trace() -> PlatformPrivacySystemTrace {
    use windows::ApplicationModel::DataTransfer::{Clipboard, ClipboardHistoryItemsResultStatus};

    let result = Clipboard::GetHistoryItemsAsync().and_then(|operation| operation.join());
    let (available, item_count, revision) = match result {
        Ok(result) if result.Status().ok() == Some(ClipboardHistoryItemsResultStatus::Success) => {
            match result.Items() {
                Ok(items) => {
                    let size = items.Size().unwrap_or(0);
                    let mut hasher = blake3::Hasher::new();
                    let mut complete = true;
                    for index in 0..size {
                        match items.GetAt(index).and_then(|item| item.Id()) {
                            Ok(id) => {
                                hasher.update(id.to_string().as_bytes());
                            }
                            Err(_) => {
                                complete = false;
                                break;
                            }
                        };
                    }
                    if complete {
                        (
                            true,
                            u64::from(size),
                            format!("history:{}", hasher.finalize().to_hex()),
                        )
                    } else {
                        (false, 0, "unavailable".into())
                    }
                }
                Err(_) => (false, 0, "unavailable".into()),
            }
        }
        _ => (false, 0, "unavailable".into()),
    };
    PlatformPrivacySystemTrace {
        provider_key: "clipboard_history".into(),
        display_name: "Clipboard history".into(),
        kind: PlatformPrivacySystemTraceKind::ClipboardHistory,
        roots: Vec::new(),
        all_time_only: false,
        available,
        item_count,
        revision,
    }
}

fn jump_lists_trace(roaming: &Path) -> Option<PlatformPrivacySystemTrace> {
    let roots = [
        roaming.join("Microsoft/Windows/Recent/AutomaticDestinations"),
        roaming.join("Microsoft/Windows/Recent/CustomDestinations"),
    ]
    .into_iter()
    .filter(|path| path.is_dir())
    .collect::<Vec<_>>();
    if roots.is_empty() {
        return None;
    }
    let item_count = roots.iter().map(|root| count_destination_files(root)).sum();
    Some(PlatformPrivacySystemTrace {
        provider_key: "jump_lists".into(),
        display_name: "Application jump lists".into(),
        kind: PlatformPrivacySystemTraceKind::JumpLists,
        roots,
        all_time_only: true,
        available: true,
        item_count,
        revision: String::new(),
    })
}

fn shell_history_trace(roaming: &Path, home: Option<&Path>) -> Option<PlatformPrivacySystemTrace> {
    let mut roots = [
        roaming.join("Microsoft/Windows/PowerShell/PSReadLine/ConsoleHost_history.txt"),
        roaming.join("Microsoft/PowerShell/PSReadLine/ConsoleHost_history.txt"),
    ]
    .into_iter()
    .filter(|path| regular_file_without_reparse_points(path))
    .collect::<Vec<_>>();
    if let Some(home) = home {
        roots.extend(
            [
                ".bash_history",
                ".python_history",
                ".node_repl_history",
                ".psql_history",
                ".mysql_history",
                ".sqlite_history",
            ]
            .into_iter()
            .map(|name| home.join(name))
            .filter(|path| regular_file_without_reparse_points(path)),
        );
    }
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        return None;
    }
    let item_count = roots.iter().map(|path| count_text_records(path)).sum();
    Some(PlatformPrivacySystemTrace {
        provider_key: "shell_history".into(),
        display_name: "Terminal and command history".into(),
        kind: PlatformPrivacySystemTraceKind::ShellHistory,
        roots,
        all_time_only: true,
        available: true,
        item_count,
        revision: String::new(),
    })
}

fn count_destination_files(root: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| regular_file_without_reparse_points(&entry.path()))
        .filter(|entry| {
            entry.path().extension().is_some_and(|extension| {
                extension.eq_ignore_ascii_case("automaticDestinations-ms")
                    || extension.eq_ignore_ascii_case("customDestinations-ms")
            })
        })
        .count() as u64
}

fn regular_file_without_reparse_points(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_file() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    })
}

fn count_text_records(path: &Path) -> u64 {
    let Ok(file) = fs::File::open(path) else {
        return 0;
    };
    BufReader::new(file)
        .split(b'\n')
        .take(100_000)
        .filter(Result::is_ok)
        .count() as u64
}

fn registry_trace(
    provider_key: &str,
    display_name: &str,
    kind: PlatformPrivacySystemTraceKind,
    paths: &[&str],
) -> PlatformPrivacySystemTrace {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let snapshot = registry_paths_snapshot(&root, paths);
    let available = snapshot.is_ok();
    let (item_count, revision) = snapshot.unwrap_or_default();
    PlatformPrivacySystemTrace {
        provider_key: provider_key.into(),
        display_name: display_name.into(),
        kind,
        roots: Vec::new(),
        all_time_only: true,
        available,
        item_count,
        revision: if available {
            format!("registry:{revision}")
        } else {
            "unavailable".into()
        },
    }
}

fn registry_paths_snapshot(root: &RegKey, paths: &[&str]) -> std::io::Result<(u64, String)> {
    let mut hasher = blake3::Hasher::new();
    let mut item_count = 0_u64;
    for path in paths {
        match registry_snapshot(root, path)? {
            Some((count, revision)) => {
                item_count = item_count.saturating_add(count);
                hasher.update(revision.as_bytes());
            }
            None => {
                hasher.update(b"missing");
            }
        }
    }
    Ok((item_count, hasher.finalize().to_hex().to_string()))
}

/// Counts ShellBag hierarchy nodes rather than every view-setting field stored below `Bags`.
/// One numeric BagMRU value represents one remembered shell folder, while values such as Mode,
/// IconSize, Sort, and FFlags are attributes of that folder and must not inflate the privacy count.
fn folder_view_history_trace() -> PlatformPrivacySystemTrace {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let result = folder_view_history_snapshot(&root, FOLDER_VIEW_BAG_MRU, FOLDER_VIEW_BAGS);
    let (available, item_count, revision) = match result {
        Ok((count, revision)) => (true, count, format!("registry:{revision}")),
        Err(_) => (false, 0, "unavailable".into()),
    };
    PlatformPrivacySystemTrace {
        provider_key: "folder_view_history".into(),
        display_name: "Folder view history".into(),
        kind: PlatformPrivacySystemTraceKind::FolderViewHistory,
        roots: Vec::new(),
        all_time_only: true,
        available,
        item_count,
        revision,
    }
}

fn folder_view_history_snapshot(
    root: &RegKey,
    bag_mru_path: &str,
    bags_path: &str,
) -> std::io::Result<(u64, String)> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-windows-folder-view-history-v1\0");
    let mut count = 0_u64;
    match root.open_subkey_with_flags(bag_mru_path, KEY_READ) {
        Ok(key) => hash_folder_view_mru_key(&key, &mut hasher, &mut count, 0)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            hasher.update(b"missing-bag-mru");
        }
        Err(error) => return Err(error),
    }
    match registry_snapshot(root, bags_path)? {
        Some((_, revision)) => hasher.update(revision.as_bytes()),
        None => hasher.update(b"missing-bags"),
    };
    Ok((count, hasher.finalize().to_hex().to_string()))
}

fn hash_folder_view_mru_key(
    key: &RegKey,
    hasher: &mut blake3::Hasher,
    count: &mut u64,
    depth: usize,
) -> std::io::Result<()> {
    if depth > 32 || *count >= 50_000 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "folder view history exceeds the bounded scan limit",
        ));
    }
    for value in key.enum_values() {
        let (name, value) = value?;
        if !name.is_empty() && name.chars().all(|character| character.is_ascii_digit()) {
            *count = count.saturating_add(1);
        }
        hasher.update(name.as_bytes());
        hasher.update(format!("{:?}", value.vtype).as_bytes());
        hasher.update(&value.bytes);
    }
    for child_name in key.enum_keys() {
        let child_name = child_name?;
        hasher.update(child_name.as_bytes());
        let child = key.open_subkey_with_flags(&child_name, KEY_READ)?;
        hash_folder_view_mru_key(&child, hasher, count, depth + 1)?;
    }
    Ok(())
}

fn registry_snapshot(root: &RegKey, path: &str) -> std::io::Result<Option<(u64, String)>> {
    let key = match root.open_subkey_with_flags(path, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut hasher = blake3::Hasher::new();
    let mut count = 0_u64;
    hash_registry_key(&key, &mut hasher, &mut count, 0)?;
    Ok(Some((count, hasher.finalize().to_hex().to_string())))
}

fn office_mru_value_name(name: &str) -> bool {
    name.get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Item "))
        && name[5..]
            .chars()
            .all(|character| character.is_ascii_digit())
}

fn office_mru_key_paths(
    root: &RegKey,
    path: &str,
    descendant_key: Option<&str>,
) -> std::io::Result<Vec<String>> {
    if descendant_key.is_none() {
        return match root.open_subkey_with_flags(path, KEY_READ) {
            Ok(_) => Ok(vec![path.to_owned()]),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error),
        };
    }
    let key = match root.open_subkey_with_flags(path, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut matches = Vec::new();
    collect_office_mru_key_paths(
        &key,
        path,
        descendant_key.expect("checked above"),
        0,
        &mut matches,
    )?;
    Ok(matches)
}

fn collect_office_mru_key_paths(
    key: &RegKey,
    path: &str,
    expected_leaf: &str,
    depth: usize,
    matches: &mut Vec<String>,
) -> std::io::Result<()> {
    if depth > 8 || matches.len() >= 256 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the Office MRU registry shape exceeds the bounded limit",
        ));
    }
    for child_name in key.enum_keys() {
        let child_name = child_name?;
        let child_path = format!(r"{path}\{child_name}");
        let child = key.open_subkey_with_flags(&child_name, KEY_READ)?;
        if child_name.eq_ignore_ascii_case(expected_leaf) {
            matches.push(child_path);
        } else {
            collect_office_mru_key_paths(&child, &child_path, expected_leaf, depth + 1, matches)?;
        }
    }
    Ok(())
}

fn office_mru_snapshot(
    root: &RegKey,
    path: &str,
    descendant_key: Option<&str>,
    application: &str,
) -> std::io::Result<(u64, String)> {
    let mut count = 0_u64;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-windows-office-mru-v1\0");
    hasher.update(application.as_bytes());
    for key_path in office_mru_key_paths(root, path, descendant_key)? {
        let key = root.open_subkey_with_flags(&key_path, KEY_READ)?;
        for value in key.enum_values() {
            let (name, value) = value?;
            if !office_mru_value_name(&name) {
                continue;
            }
            count = count.saturating_add(1);
            hasher.update(key_path.as_bytes());
            hasher.update(name.as_bytes());
            hasher.update(format!("{:?}", value.vtype).as_bytes());
            hasher.update(&value.bytes);
        }
    }
    Ok((count, hasher.finalize().to_hex().to_string()))
}

fn collect_office_mru_details(
    root: &RegKey,
    path: &str,
    descendant_key: Option<&str>,
    page: &mut RegistryDetailPage,
) -> PlatformResult<bool> {
    let key_paths = office_mru_key_paths(root, path, descendant_key)
        .map_err(|_| PlatformError::operation_failed("read Microsoft 365 recent items failed"))?;
    for key_path in key_paths {
        let key = root
            .open_subkey_with_flags(&key_path, KEY_READ)
            .map_err(|_| {
                PlatformError::operation_failed("open Microsoft 365 recent items failed")
            })?;
        for value in key.enum_values() {
            let (name, value) = value.map_err(|_| {
                PlatformError::operation_failed("read Microsoft 365 recent item failed")
            })?;
            if !office_mru_value_name(&name) {
                continue;
            }
            let raw = String::from_reg_value(&value).unwrap_or_default();
            let payload = raw.rsplit_once('*').map_or(raw.as_str(), |(_, path)| path);
            let label = sanitize_registry_detail(if payload.trim().is_empty() {
                &name
            } else {
                payload
            });
            if page.push(label)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn clear_office_mru_values(
    root: &RegKey,
    path: &str,
    descendant_key: Option<&str>,
) -> std::io::Result<()> {
    for key_path in office_mru_key_paths(root, path, descendant_key)? {
        let key =
            root.open_subkey_with_flags(&key_path, KEY_READ | winreg::enums::KEY_SET_VALUE)?;
        let names = key
            .enum_values()
            .filter_map(Result::ok)
            .map(|(name, _)| name)
            .filter(|name| office_mru_value_name(name))
            .collect::<Vec<_>>();
        for name in names {
            key.delete_value(name)?;
        }
    }
    Ok(())
}

fn office_mru_contains_items(
    root: &RegKey,
    path: &str,
    descendant_key: Option<&str>,
) -> std::io::Result<bool> {
    for key_path in office_mru_key_paths(root, path, descendant_key)? {
        let key = root.open_subkey_with_flags(key_path, KEY_READ)?;
        for value in key.enum_values() {
            let (name, _) = value?;
            if office_mru_value_name(&name) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn direct_child_records_snapshot(
    root: &RegKey,
    path: &str,
) -> std::io::Result<Option<(u64, String)>> {
    let key = match root.open_subkey_with_flags(path, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let child_count = key
        .enum_keys()
        .try_fold(0_u64, |count, name| name.map(|_| count.saturating_add(1)))?;
    let revision = registry_snapshot(root, path)?
        .map(|(_, revision)| revision)
        .unwrap_or_default();
    Ok(Some((child_count, revision)))
}

fn collect_direct_child_record_details(
    root: &RegKey,
    path: &str,
    label_value: &str,
    page: &mut RegistryDetailPage,
) -> PlatformResult<bool> {
    let key = match root.open_subkey_with_flags(path, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => {
            return Err(PlatformError::operation_failed(
                "read application recent-item details failed",
            ))
        }
    };
    for child_name in key.enum_keys() {
        let child_name = child_name.map_err(|_| {
            PlatformError::operation_failed("read application recent-item key failed")
        })?;
        let child = key
            .open_subkey_with_flags(&child_name, KEY_READ)
            .map_err(|_| {
                PlatformError::operation_failed("open application recent-item key failed")
            })?;
        let label = child
            .get_raw_value(label_value)
            .ok()
            .map(|value| registry_detail_label(&child_name, &value))
            .unwrap_or_else(|| child_name.clone());
        if page.push(label)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn registry_targets_snapshot(targets: &[RegistryTarget]) -> std::io::Result<(u64, String)> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let mut hasher = blake3::Hasher::new();
    let mut item_count = 0_u64;
    for target in targets {
        match *target {
            RegistryTarget::Key(path) => match registry_snapshot(&root, path)? {
                Some((count, revision)) => {
                    item_count = item_count.saturating_add(count);
                    hasher.update(path.as_bytes());
                    hasher.update(revision.as_bytes());
                }
                None => {
                    hasher.update(b"missing-key");
                }
            },
            RegistryTarget::MetadataKey(path) => match registry_snapshot(&root, path)? {
                Some((_, revision)) => {
                    // Auxiliary state participates in drift detection and cleanup verification,
                    // but its internal fields must not inflate the logical history count.
                    hasher.update(path.as_bytes());
                    hasher.update(revision.as_bytes());
                }
                None => {
                    hasher.update(b"missing-metadata-key");
                }
            },
            RegistryTarget::Value { path, name } => {
                match registry_value_snapshot(&root, path, name)? {
                    Some(revision) => {
                        item_count = item_count.saturating_add(1);
                        hasher.update(path.as_bytes());
                        hasher.update(name.as_bytes());
                        hasher.update(revision.as_bytes());
                    }
                    None => {
                        hasher.update(b"missing-value");
                    }
                }
            }
            RegistryTarget::OfficeMru {
                path,
                descendant_key,
                application,
            } => {
                let (count, revision) =
                    office_mru_snapshot(&root, path, descendant_key, application)?;
                item_count = item_count.saturating_add(count);
                hasher.update(path.as_bytes());
                hasher.update(revision.as_bytes());
            }
            RegistryTarget::DirectChildRecords { path, .. } => {
                match direct_child_records_snapshot(&root, path)? {
                    Some((count, revision)) => {
                        item_count = item_count.saturating_add(count);
                        hasher.update(path.as_bytes());
                        hasher.update(revision.as_bytes());
                    }
                    None => {
                        hasher.update(b"missing-direct-child-records");
                    }
                }
            }
        }
    }
    Ok((
        item_count,
        format!("registry:{}", hasher.finalize().to_hex()),
    ))
}

fn registry_path_details(
    paths: &[&str],
    offset: u64,
    limit: u32,
) -> PlatformResult<Vec<PlatformPrivacyDetailEntry>> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let mut page = RegistryDetailPage::new(offset, limit);
    for path in paths {
        let key = match root.open_subkey_with_flags(path, KEY_READ) {
            Ok(key) => key,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Err(PlatformError::operation_failed(
                    "read privacy registry details failed",
                ))
            }
        };
        if collect_registry_key_details(&key, "", &mut page, 0)? {
            break;
        }
    }
    Ok(page.entries)
}

fn registry_target_details(
    targets: &[RegistryTarget],
    offset: u64,
    limit: u32,
) -> PlatformResult<Vec<PlatformPrivacyDetailEntry>> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let mut page = RegistryDetailPage::new(offset, limit);
    for target in targets {
        match *target {
            RegistryTarget::Key(path) => {
                let key = match root.open_subkey_with_flags(path, KEY_READ) {
                    Ok(key) => key,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => {
                        return Err(PlatformError::operation_failed(
                            "read application privacy registry details failed",
                        ))
                    }
                };
                if collect_registry_key_details(&key, "", &mut page, 0)? {
                    break;
                }
            }
            // Metadata participates in fingerprinting and cleanup but is not a user-visible trace.
            RegistryTarget::MetadataKey(_) => {}
            RegistryTarget::Value { path, name } => {
                let key = match root.open_subkey_with_flags(path, KEY_READ) {
                    Ok(key) => key,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => {
                        return Err(PlatformError::operation_failed(
                            "read application privacy registry details failed",
                        ))
                    }
                };
                match key.get_raw_value(name) {
                    Ok(value) => {
                        if page.push(registry_detail_label(name, &value))? {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => {
                        return Err(PlatformError::operation_failed(
                            "read application privacy registry value failed",
                        ))
                    }
                }
            }
            RegistryTarget::OfficeMru {
                path,
                descendant_key,
                ..
            } => {
                if collect_office_mru_details(&root, path, descendant_key, &mut page)? {
                    break;
                }
            }
            RegistryTarget::DirectChildRecords { path, label_value } => {
                if collect_direct_child_record_details(&root, path, label_value, &mut page)? {
                    break;
                }
            }
        }
    }
    Ok(page.entries)
}

fn collect_registry_key_details(
    key: &RegKey,
    prefix: &str,
    page: &mut RegistryDetailPage,
    depth: usize,
) -> PlatformResult<bool> {
    if depth > 32 {
        return Err(PlatformError::operation_failed(
            "privacy registry details exceed the bounded limit",
        ));
    }
    for value in key.enum_values() {
        let (name, value) = value
            .map_err(|_| PlatformError::operation_failed("read privacy registry value failed"))?;
        if name.eq_ignore_ascii_case("MRUList") || name.eq_ignore_ascii_case("MRUListEx") {
            continue;
        }
        let fallback = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix} / {name}")
        };
        if page.push(registry_detail_label(&fallback, &value))? {
            return Ok(true);
        }
    }
    for child_name in key.enum_keys() {
        let child_name = child_name
            .map_err(|_| PlatformError::operation_failed("read privacy registry key failed"))?;
        let child = key
            .open_subkey_with_flags(&child_name, KEY_READ)
            .map_err(|_| PlatformError::operation_failed("open privacy registry key failed"))?;
        let child_prefix = if prefix.is_empty() {
            child_name
        } else {
            format!("{prefix} / {child_name}")
        };
        if collect_registry_key_details(&child, &child_prefix, page, depth + 1)? {
            return Ok(true);
        }
    }
    Ok(false)
}

struct RegistryDetailPage {
    offset: u64,
    limit: usize,
    seen: u64,
    entries: Vec<PlatformPrivacyDetailEntry>,
}

impl RegistryDetailPage {
    fn new(offset: u64, limit: u32) -> Self {
        Self {
            offset,
            limit: limit as usize,
            seen: 0,
            entries: Vec::with_capacity(limit as usize),
        }
    }

    /// Returns `true` as soon as the requested page is complete, allowing recursive registry
    /// traversal to stop without retaining unrelated private labels in memory.
    fn push(&mut self, label: String) -> PlatformResult<bool> {
        if self.seen >= 50_000 {
            return Err(PlatformError::operation_failed(
                "privacy registry details exceed the bounded limit",
            ));
        }
        if self.seen >= self.offset && self.entries.len() < self.limit {
            self.entries.push(PlatformPrivacyDetailEntry {
                label,
                item_count: 1,
            });
        }
        self.seen = self.seen.saturating_add(1);
        Ok(self.entries.len() >= self.limit)
    }
}

fn registry_detail_label(fallback: &str, value: &RegValue<'_>) -> String {
    let text = String::from_reg_value(value).unwrap_or_default();
    let label = if text.trim().is_empty() {
        fallback
    } else {
        &text
    };
    sanitize_registry_detail(label)
}

fn sanitize_registry_detail(label: &str) -> String {
    label
        .trim()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(2_048)
        .collect()
}

fn registry_value_snapshot(
    root: &RegKey,
    path: &str,
    name: &str,
) -> std::io::Result<Option<String>> {
    let key = match root.open_subkey_with_flags(path, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let value = match key.get_raw_value(name) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(format!("{:?}", value.vtype).as_bytes());
    hasher.update(&value.bytes);
    Ok(Some(hasher.finalize().to_hex().to_string()))
}

fn hash_registry_key(
    key: &RegKey,
    hasher: &mut blake3::Hasher,
    count: &mut u64,
    depth: usize,
) -> std::io::Result<()> {
    if depth > 32 || *count >= 50_000 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "privacy registry source exceeds the bounded scan limit",
        ));
    }
    for value in key.enum_values() {
        let (name, value) = value?;
        // MRUList and MRUListEx describe ordering only; count user trace records instead. Hashing
        // every value still detects content changes without returning registry data to Core.
        if !name.eq_ignore_ascii_case("MRUList") && !name.eq_ignore_ascii_case("MRUListEx") {
            *count = count.saturating_add(1);
        }
        hasher.update(name.as_bytes());
        hasher.update(format!("{:?}", value.vtype).as_bytes());
        hasher.update(&value.bytes);
    }
    for child_name in key.enum_keys() {
        let child_name = child_name?;
        hasher.update(child_name.as_bytes());
        let child = key.open_subkey_with_flags(&child_name, KEY_READ)?;
        hash_registry_key(&child, hasher, count, depth + 1)?;
    }
    Ok(())
}

fn clear_registry_paths(paths: &[&str]) -> PlatformResult<bool> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    for path in paths {
        match root.delete_subkey_all(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(PlatformError::operation_failed(
                    "clear privacy registry history failed",
                ))
            }
        }
    }
    Ok(paths.iter().all(|path| {
        root.open_subkey_with_flags(path, KEY_READ)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    }))
}

fn clear_registry_targets(targets: &[RegistryTarget]) -> PlatformResult<bool> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    for target in targets {
        let result = match *target {
            RegistryTarget::Key(path) | RegistryTarget::MetadataKey(path) => {
                root.delete_subkey_all(path)
            }
            RegistryTarget::Value { path, name } => root
                .open_subkey_with_flags(path, winreg::enums::KEY_SET_VALUE)
                .and_then(|key| key.delete_value(name)),
            RegistryTarget::OfficeMru {
                path,
                descendant_key,
                ..
            } => clear_office_mru_values(&root, path, descendant_key),
            RegistryTarget::DirectChildRecords { path, .. } => root.delete_subkey_all(path),
        };
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(PlatformError::operation_failed(
                    "clear application privacy registry trace failed",
                ))
            }
        }
    }
    registry_targets_are_absent(&root, targets).map_err(|_| {
        PlatformError::operation_failed("verify application privacy registry trace failed")
            .with_possible_side_effects()
    })
}

/// Verifies exact target absence instead of inferring success from a zero logical count.
/// Metadata-only keys intentionally contribute zero items, so count-based verification could
/// otherwise claim success if an application recreated those keys during cleanup.
fn registry_targets_are_absent(root: &RegKey, targets: &[RegistryTarget]) -> std::io::Result<bool> {
    for target in targets {
        match *target {
            RegistryTarget::Key(path) | RegistryTarget::MetadataKey(path) => {
                match root.open_subkey_with_flags(path, KEY_READ) {
                    Ok(_) => return Ok(false),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            RegistryTarget::Value { path, name } => {
                let key = match root.open_subkey_with_flags(path, KEY_READ) {
                    Ok(key) => key,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                match key.get_raw_value(name) {
                    Ok(_) => return Ok(false),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            RegistryTarget::OfficeMru {
                path,
                descendant_key,
                ..
            } => {
                if office_mru_contains_items(root, path, descendant_key)? {
                    return Ok(false);
                }
            }
            RegistryTarget::DirectChildRecords { path, .. } => {
                match root.open_subkey_with_flags(path, KEY_READ) {
                    Ok(_) => return Ok(false),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(true)
}

fn clipboard_format_count() -> u64 {
    // SAFETY: CountClipboardFormats reads only an aggregate number and does not expose clipboard
    // data. A zero result is also the correct conservative answer when Windows reports an error.
    unsafe { CountClipboardFormats().max(0) as u64 }
}

fn clipboard_revision() -> String {
    // SAFETY: The sequence number is aggregate metadata maintained by User32. It changes whenever
    // clipboard ownership or content changes and does not expose any clipboard format or value.
    unsafe { format!("clipboard:{}", GetClipboardSequenceNumber()) }
}

fn clear_clipboard() -> PlatformResult<bool> {
    // SAFETY: The clipboard is opened for this process, emptied, and closed on every path after a
    // successful open. A null owner is the documented choice for a foreground desktop action.
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err(PlatformError::operation_failed("open clipboard failed"));
        }
        let emptied = EmptyClipboard() != 0;
        let closed = CloseClipboard() != 0;
        if !emptied || !closed {
            return Err(PlatformError::operation_failed("clear clipboard failed")
                .with_possible_side_effects());
        }
    }
    Ok(clipboard_format_count() == 0)
}

fn chromium_browser(
    provider_key: &str,
    display_name: &str,
    root: &Path,
    application_path: Option<PathBuf>,
    process_names: Vec<String>,
    cancellation: &PlatformCancellation,
) -> PlatformResult<PlatformPrivacyBrowser> {
    let mut profiles = Vec::new();
    if root.is_dir() {
        // Opera releases have used both a root-level profile and Chromium's Default directory.
        // Only a root that owns a supported source becomes a profile, so the browser-wide Local
        // State file cannot be mistaken for browsing evidence.
        if chromium_profile_has_supported_sources(root) {
            profiles.push(chromium_profile(
                provider_key,
                "Default".into(),
                root.to_path_buf(),
            ));
        }
        for entry in fs::read_dir(root)
            .map_err(|error| PlatformError::io("enumerate browser profiles", &error))?
        {
            ensure_not_cancelled(cancellation)?;
            let entry = entry.map_err(|error| PlatformError::io("read browser profile", &error))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != "Default" && !name.starts_with("Profile ") {
                continue;
            }
            let profile_root = entry.path();
            if profile_root.is_dir() {
                profiles.push(chromium_profile(provider_key, name, profile_root));
            }
        }
    }
    Ok(PlatformPrivacyBrowser {
        provider_key: provider_key.into(),
        display_name: display_name.into(),
        application_path,
        kind: PlatformPrivacyBrowserKind::Chromium,
        process_names,
        profiles,
    })
}

fn chromium_profile_has_supported_sources(root: &Path) -> bool {
    [
        root.join("History"),
        root.join("Cookies"),
        root.join("Network/Cookies"),
        root.join("Login Data"),
        root.join("Web Data"),
        root.join("Sessions"),
        root.join("Local Storage"),
        root.join("IndexedDB"),
        root.join("Service Worker"),
        root.join("WebStorage"),
        root.join("Top Sites"),
        root.join("Shortcuts"),
        root.join("Favicons"),
        root.join("Cache"),
        root.join("Code Cache"),
    ]
    .into_iter()
    .any(|path| path.exists())
}

fn chromium_profile(browser_key: &str, name: String, root: PathBuf) -> PlatformPrivacyProfile {
    let display_name = chromium_display_name(&root, &name);
    PlatformPrivacyProfile {
        provider_key: format!("{browser_key}:{name}"),
        display_name,
        history_database: existing_or_inaccessible_file(root.join("History")),
        // Chromium has moved this database between the profile root and Network across products
        // and releases. Prefer the newer nested location when both exist, while retaining the
        // root fallback used by Chrome variants that still keep Cookies beside History.
        cookie_database: first_existing_or_inaccessible_file([
            root.join("Network/Cookies"),
            root.join("Cookies"),
        ]),
        saved_password_source: existing_or_inaccessible_file(root.join("Login Data")),
        autofill_database: existing_or_inaccessible_file(root.join("Web Data")),
        permission_database: None,
        top_sites_database: existing_or_inaccessible_file(root.join("Top Sites")),
        shortcut_database: existing_or_inaccessible_file(root.join("Shortcuts")),
        favicon_database: existing_or_inaccessible_file(root.join("Favicons")),
        session_directories: existing_directories(&root, &["Sessions"]),
        site_storage_directories: existing_directories(
            &root,
            &["Local Storage", "IndexedDB", "Service Worker", "WebStorage"],
        ),
        cache_directories: existing_directories(
            &root,
            &[
                "Cache",
                "Code Cache",
                "GPUCache",
                "DawnCache",
                "GrShaderCache",
                "GraphiteDawnCache",
                "Media Cache",
            ],
        ),
        root,
    }
}

fn firefox_browser(
    root: &Path,
    application_path: Option<PathBuf>,
    cancellation: &PlatformCancellation,
) -> PlatformResult<PlatformPrivacyBrowser> {
    let mut profiles = Vec::new();
    let display_names = firefox_display_names(root);
    if root.is_dir() {
        for entry in fs::read_dir(root)
            .map_err(|error| PlatformError::io("enumerate Firefox profiles", &error))?
        {
            ensure_not_cancelled(cancellation)?;
            let entry = entry.map_err(|error| PlatformError::io("read Firefox profile", &error))?;
            let profile_root = entry.path();
            if !profile_root.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let saved_password_source =
                existing_or_inaccessible_file(profile_root.join("logins.json"));
            profiles.push(PlatformPrivacyProfile {
                provider_key: format!("firefox:{name}"),
                display_name: display_names.get(&name).cloned().unwrap_or(name),
                history_database: existing_or_inaccessible_file(profile_root.join("places.sqlite")),
                cookie_database: existing_or_inaccessible_file(profile_root.join("cookies.sqlite")),
                saved_password_source,
                autofill_database: existing_or_inaccessible_file(
                    profile_root.join("formhistory.sqlite"),
                ),
                permission_database: existing_or_inaccessible_file(
                    profile_root.join("permissions.sqlite"),
                ),
                top_sites_database: None,
                shortcut_database: None,
                favicon_database: None,
                session_directories: existing_directories(&profile_root, &["sessionstore-backups"]),
                site_storage_directories: existing_directories(&profile_root, &["storage/default"]),
                cache_directories: windows_firefox_cache_directories(&profile_root),
                root: profile_root,
            });
        }
    }
    Ok(PlatformPrivacyBrowser {
        provider_key: "firefox".into(),
        display_name: "Firefox".into(),
        application_path,
        kind: PlatformPrivacyBrowserKind::Firefox,
        process_names: vec!["firefox.exe".into()],
        profiles,
    })
}

fn windows_application_path(
    local_candidate: &Path,
    program_files_relative: &str,
) -> Option<PathBuf> {
    std::iter::once(local_candidate.to_path_buf())
        .chain(
            ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
                .into_iter()
                .filter_map(std::env::var_os)
                .map(PathBuf::from)
                .map(|root| root.join(program_files_relative)),
        )
        .find(|path| path.is_file())
}

fn preferred_directory(paths: impl IntoIterator<Item = PathBuf>) -> PathBuf {
    let mut paths = paths.into_iter();
    let primary = paths
        .next()
        .expect("browser directory candidates must not be empty");
    if primary.is_dir() {
        return primary;
    }
    paths.find(|path| path.is_dir()).unwrap_or(primary)
}

fn first_existing_file(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find(|path| path.is_file())
}

fn first_versioned_executable(root: &Path, executable_name: &std::ffi::OsStr) -> Option<PathBuf> {
    let mut candidates = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("app-"))
        .map(|entry| entry.path().join(executable_name))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.pop()
}

fn windows_program_files_paths(relatives: &[&str]) -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .flat_map(|root| relatives.iter().map(move |relative| root.join(relative)))
        .collect()
}

/// Resolves one packaged application executable from the current-user AppModel repository.
/// Package family prefixes are stable while version and architecture segments change, so reading
/// the registered root avoids hard-coding a Windows Store build number or scanning WindowsApps.
fn windows_appx_executable(package_prefix: &str, executable: &str) -> Option<PathBuf> {
    let root = windows_appx_package_root(package_prefix)?;
    let path = root.join(executable);
    path.is_file().then_some(path)
}

/// Returns a manifest-declared image path even when Windows stores only scale or target-size
/// variants beside it. The shared icon resolver expands this declared path to the best existing
/// variant, matching the way the shell resolves packaged application artwork.
fn windows_appx_declared_icon(package_prefix: &str, declared_icon: &str) -> Option<PathBuf> {
    let path = windows_appx_package_root(package_prefix)?.join(declared_icon);
    path.parent()?.is_dir().then_some(path)
}

fn windows_appx_package_root(package_prefix: &str) -> Option<PathBuf> {
    const PACKAGES_KEY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    let packages = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(PACKAGES_KEY, KEY_READ)
        .ok()?;
    packages
        .enum_keys()
        .filter_map(Result::ok)
        .filter(|name| name.starts_with(package_prefix))
        .filter_map(|name| packages.open_subkey_with_flags(name, KEY_READ).ok())
        .filter_map(|key| key.get_value::<String, _>("PackageRootFolder").ok())
        .map(PathBuf::from)
        .find(|root| root.is_dir())
}

fn count_recent_links(root: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        })
        .filter(|entry| {
            entry.metadata().is_ok_and(|metadata| {
                metadata.is_file() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
            })
        })
        .count() as u64
}

/// Treats the Recent shortcut directory as the canonical user-visible source. Windows mirrors
/// these records in Explorer's RecentDocs registry tree, so exposing both sources would duplicate
/// the same logical history and make a single cleanup appear as two unrelated actions.
fn recent_document_history_trace(recent: &Path) -> PlatformPrivacySystemTrace {
    let native_snapshot =
        registry_paths_snapshot(&RegKey::predef(HKEY_CURRENT_USER), RECENT_DOCUMENT_HISTORY);
    let native_available = native_snapshot.is_ok();
    let native_revision = native_snapshot
        .map(|(_, revision)| format!("registry:{revision}"))
        .unwrap_or_else(|_| "unavailable".into());
    PlatformPrivacySystemTrace {
        provider_key: "recent_document_history".into(),
        display_name: "Recent document history".into(),
        kind: PlatformPrivacySystemTraceKind::RecentDocumentHistory,
        roots: recent
            .is_dir()
            .then_some(recent.to_path_buf())
            .into_iter()
            .collect(),
        all_time_only: true,
        available: recent.is_dir() && native_available,
        item_count: if recent.is_dir() && native_available {
            count_recent_links(recent)
        } else {
            0
        },
        revision: native_revision,
    }
}

fn ensure_not_cancelled(cancellation: &PlatformCancellation) -> PlatformResult<()> {
    if cancellation.is_cancelled() {
        return Err(PlatformError::new(
            PlatformErrorCode::UserCancelled,
            "privacy source discovery cancelled",
        ));
    }
    Ok(())
}

fn existing_or_inaccessible_file(path: PathBuf) -> Option<PathBuf> {
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Some(path),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Some(path),
        _ => None,
    }
}

fn first_existing_or_inaccessible_file(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    paths.into_iter().find_map(existing_or_inaccessible_file)
}

fn existing_directories(root: &Path, relatives: &[&str]) -> Vec<PathBuf> {
    relatives
        .iter()
        .map(|relative| root.join(relative))
        .filter(|path| path.exists())
        .collect()
}

fn windows_firefox_cache_directories(profile_root: &Path) -> Vec<PathBuf> {
    let Some(profile_name) = profile_root.file_name() else {
        return Vec::new();
    };
    let Some(local) = dirs::data_local_dir() else {
        return Vec::new();
    };
    existing_directories(
        &local.join("Mozilla/Firefox/Profiles").join(profile_name),
        &["cache2", "startupCache", "thumbnails"],
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    fn fixture_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mangodisk-windows-privacy-{label}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn additional_chromium_browsers_keep_independent_source_identity() {
        let fixture = fixture_directory("additional-chromium-browsers");
        for (provider_key, display_name) in [
            ("brave", "Brave"),
            ("opera", "Opera"),
            ("360_safe_browser", "360 Safe Browser"),
            ("qq_browser", "QQ Browser"),
        ] {
            let root = fixture.join(provider_key);
            let profile_root = root.join("Default");
            fs::create_dir_all(&profile_root).unwrap();
            fs::write(profile_root.join("History"), b"fixture").unwrap();
            fs::write(profile_root.join("Cookies"), b"fixture").unwrap();
            fs::write(profile_root.join("Top Sites"), b"fixture").unwrap();
            fs::write(profile_root.join("Shortcuts"), b"fixture").unwrap();
            fs::write(profile_root.join("Favicons"), b"fixture").unwrap();
            fs::create_dir(profile_root.join("Cache")).unwrap();

            let browser = chromium_browser(
                provider_key,
                display_name,
                &root,
                None,
                vec![format!("{provider_key}.exe")],
                &PlatformCancellation::new(|| false),
            )
            .unwrap();

            assert_eq!(browser.provider_key, provider_key);
            assert_eq!(browser.display_name, display_name);
            assert_eq!(browser.profiles.len(), 1);
            assert!(browser.profiles[0].history_database.is_some());
            assert!(browser.profiles[0].cookie_database.is_some());
            assert!(browser.profiles[0].top_sites_database.is_some());
            assert!(browser.profiles[0].shortcut_database.is_some());
            assert!(browser.profiles[0].favicon_database.is_some());
            assert!(!browser.profiles[0].cache_directories.is_empty());
        }
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn root_level_opera_profile_is_discovered() {
        let fixture = fixture_directory("opera-root-profile");
        fs::write(fixture.join("History"), b"fixture").unwrap();
        fs::write(fixture.join("Cookies"), b"fixture").unwrap();

        let browser = chromium_browser(
            "opera",
            "Opera",
            &fixture,
            None,
            vec!["opera.exe".into()],
            &PlatformCancellation::new(|| false),
        )
        .unwrap();

        assert_eq!(browser.profiles.len(), 1);
        assert_eq!(browser.profiles[0].provider_key, "opera:Default");
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn jump_list_discovery_groups_automatic_and_custom_destinations() {
        let fixture = fixture_directory("jump-lists");
        let recent = fixture.join("Microsoft/Windows/Recent");
        let automatic = recent.join("AutomaticDestinations");
        let custom = recent.join("CustomDestinations");
        fs::create_dir_all(&automatic).unwrap();
        fs::create_dir_all(&custom).unwrap();
        fs::write(automatic.join("one.automaticDestinations-ms"), b"one").unwrap();
        fs::write(custom.join("two.customDestinations-ms"), b"two").unwrap();
        fs::write(custom.join("ignored.txt"), b"ignored").unwrap();

        let trace = jump_lists_trace(&fixture).expect("jump-list roots should be discovered");

        assert_eq!(trace.kind, PlatformPrivacySystemTraceKind::JumpLists);
        assert_eq!(trace.roots.len(), 2);
        assert_eq!(trace.item_count, 2);
        assert!(trace.all_time_only);
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn powershell_history_discovery_counts_records() {
        let fixture = fixture_directory("powershell-history");
        let history =
            fixture.join("Microsoft/Windows/PowerShell/PSReadLine/ConsoleHost_history.txt");
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(&history, b"first\nsecond\n").unwrap();

        let trace =
            shell_history_trace(&fixture, None).expect("PowerShell history should be discovered");

        assert_eq!(trace.kind, PlatformPrivacySystemTraceKind::ShellHistory);
        assert_eq!(trace.item_count, 2);
        assert!(trace.roots.contains(&history));
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn folder_view_history_counts_bag_nodes_instead_of_view_setting_fields() {
        let base = format!(
            r"Software\MangoDisk\Tests\FolderView-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        );
        let bag_mru_path = format!(r"{base}\BagMRU");
        let bags_path = format!(r"{base}\Bags");
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let _ = root.delete_subkey_all(&base);
        let (bag_mru, _) = root.create_subkey(&bag_mru_path).unwrap();
        bag_mru.set_value("NodeSlot", &42_u32).unwrap();
        bag_mru
            .set_raw_value(
                "0",
                &RegValue {
                    vtype: winreg::enums::REG_BINARY,
                    bytes: vec![1, 2, 3, 4].into(),
                },
            )
            .unwrap();
        let (child, _) = bag_mru.create_subkey("0").unwrap();
        child
            .set_raw_value(
                "1",
                &RegValue {
                    vtype: winreg::enums::REG_BINARY,
                    bytes: vec![5, 6, 7, 8].into(),
                },
            )
            .unwrap();
        child.set_value("NodeSlot", &43_u32).unwrap();
        let (view, _) = root
            .create_subkey(format!(r"{bags_path}\42\Shell"))
            .unwrap();
        for (name, value) in [("Mode", 4_u32), ("IconSize", 48), ("Sort", 1)] {
            view.set_value(name, &value).unwrap();
        }

        let (count, revision) =
            folder_view_history_snapshot(&root, &bag_mru_path, &bags_path).unwrap();

        assert_eq!(count, 2);
        assert!(!revision.is_empty());
        root.delete_subkey_all(base).unwrap();
    }

    #[test]
    fn recent_document_history_uses_one_canonical_shortcut_source() {
        let fixture = fixture_directory("recent-document-history");
        fs::write(fixture.join("document.lnk"), b"shortcut").unwrap();
        fs::write(fixture.join("ignored.url"), b"internet shortcut").unwrap();

        let trace = recent_document_history_trace(&fixture);

        assert_eq!(trace.provider_key, "recent_document_history");
        assert_eq!(
            trace.kind,
            PlatformPrivacySystemTraceKind::RecentDocumentHistory
        );
        assert_eq!(trace.roots, vec![fixture.clone()]);
        assert_eq!(trace.item_count, 1);
        assert!(trace.available);
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    #[ignore = "reads the current user's ShellBag hierarchy without modifying it"]
    fn actual_folder_view_history_uses_logical_node_count() {
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (logical_count, _) =
            folder_view_history_snapshot(&root, FOLDER_VIEW_BAG_MRU, FOLDER_VIEW_BAGS).unwrap();
        let raw_field_count = FOLDER_VIEW_HISTORY
            .iter()
            .map(|path| {
                registry_snapshot(&root, path)
                    .unwrap()
                    .map_or(0, |value| value.0)
            })
            .sum::<u64>();

        assert!(logical_count > 0);
        assert!(logical_count < raw_field_count);
        println!(
            "validated folder view history logical_count={logical_count} raw_field_count={raw_field_count}"
        );
    }

    #[test]
    fn productivity_sources_keep_cache_logs_sessions_and_recent_documents_separate() {
        let fixture = fixture_directory("productivity-sources");
        let local = fixture.join("local");
        let roaming = fixture.join("roaming");
        for relative in ["Notion/Cache", "Notion/logs", "Notion/Session Storage"] {
            fs::create_dir_all(roaming.join(relative)).unwrap();
        }
        let adobe_cache = local.join("Adobe/Acrobat/DC/Cache");
        fs::create_dir_all(&adobe_cache).unwrap();
        let openoffice_recent =
            roaming.join("OpenOffice.org/4/user/registry/data/org/openoffice/Office/Histories.xcu");
        fs::create_dir_all(openoffice_recent.parent().unwrap()).unwrap();
        fs::write(&openoffice_recent, b"fixture").unwrap();

        let applications = application_privacy_sources(&local, &roaming);
        let notion = applications
            .iter()
            .find(|application| application.provider_key == "notion")
            .expect("Notion data roots must be discovered");
        assert!(notion
            .traces
            .iter()
            .any(|trace| trace.kind == PlatformPrivacyApplicationTraceKind::Cache));
        assert!(notion
            .traces
            .iter()
            .any(|trace| trace.kind == PlatformPrivacyApplicationTraceKind::Logs));
        assert!(notion
            .traces
            .iter()
            .any(|trace| trace.kind == PlatformPrivacyApplicationTraceKind::Sessions));

        let adobe = applications
            .iter()
            .find(|application| application.provider_key == "adobe_reader")
            .expect("Adobe cache must expose the Adobe source");
        assert!(adobe.traces.iter().any(|trace| {
            trace.kind == PlatformPrivacyApplicationTraceKind::Cache
                && trace.roots.iter().any(|root| root == &adobe_cache)
        }));
        assert!(adobe.traces.iter().any(|trace| {
            trace.kind == PlatformPrivacyApplicationTraceKind::RecentDocuments
                && trace.native_kind
                    == Some(PlatformPrivacyApplicationNativeTraceKind::AdobeReaderRecentDocuments)
        }));
        let openoffice = applications
            .iter()
            .find(|application| application.provider_key == "openoffice")
            .expect("OpenOffice history must expose the OpenOffice source");
        assert!(openoffice.traces.iter().any(|trace| {
            trace.kind == PlatformPrivacyApplicationTraceKind::RecentDocuments
                && trace.roots.iter().any(|root| root == &openoffice_recent)
        }));
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn application_sources_exclude_unrelated_apps_and_keep_office_cache() {
        let fixture = fixture_directory("application-source-scope");
        let local = fixture.join("local");
        let roaming = fixture.join("roaming");
        let wps_cache = roaming.join("Kingsoft/office6/cache");
        fs::create_dir_all(&wps_cache).unwrap();
        let wps_executable = local.join("Kingsoft/WPS Office/12.1.0.28043/office6/wpsoffice.exe");
        fs::create_dir_all(wps_executable.parent().unwrap()).unwrap();
        fs::write(&wps_executable, b"fixture").unwrap();
        for relative in [
            "Cursor/Cache",
            "Windsurf/Cache",
            "Postman/Cache",
            "Slack/Cache",
            "discord/Cache",
            "Antigravity/Cache",
            "Devin/Cache",
            "Claude/Cache",
            "Microsoft/Teams/Cache",
            "Tencent/WeMeet/Global/Logs",
            "Zoom/logs",
            "FileZilla",
            "TeamViewer",
        ] {
            fs::create_dir_all(roaming.join(relative)).unwrap();
        }
        fs::write(roaming.join("FileZilla/recentservers.xml"), b"fixture").unwrap();
        fs::write(
            roaming.join("TeamViewer/TeamViewer_Logfile.log"),
            b"fixture",
        )
        .unwrap();
        for relative in [
            "Tencent/WeMeet/WeMeetApp.exe",
            "Programs/Zoom/bin/Zoom.exe",
            "Microsoft/OneDrive/OneDrive.exe",
        ] {
            let executable = local.join(relative);
            fs::create_dir_all(executable.parent().unwrap()).unwrap();
            fs::write(executable, b"fixture").unwrap();
        }

        let applications = application_privacy_sources(&local, &roaming);
        let wps = applications
            .iter()
            .find(|application| application.provider_key == "wps_office")
            .expect("WPS Office must expose its dedicated cache");
        assert_eq!(wps.application_path.as_ref(), Some(&wps_executable));
        assert!(wps
            .traces
            .iter()
            .any(|trace| trace.roots.contains(&wps_cache)));
        assert!(wps.traces.iter().any(|trace| {
            trace.kind == PlatformPrivacyApplicationTraceKind::RecentDocuments
                && trace.native_kind
                    == Some(PlatformPrivacyApplicationNativeTraceKind::WpsOfficeRecentDocuments)
        }));
        assert!(wps.traces.iter().any(|trace| {
            trace.kind == PlatformPrivacyApplicationTraceKind::RecentPaths
                && trace.native_kind
                    == Some(PlatformPrivacyApplicationNativeTraceKind::WpsOfficeRecentFolders)
        }));

        for excluded_provider in [
            "cursor",
            "windsurf",
            "postman",
            "slack",
            "discord",
            "antigravity",
            "devin",
            "claude",
            "microsoft_teams",
            "tencent_meeting",
            "thunderbird",
            "filezilla",
            "teamviewer",
            "remote_desktop",
            "seven_zip",
            "tortoise_svn",
            "winrar",
            "winzip",
            "zoom",
            "claude_code",
            "onedrive",
        ] {
            assert!(
                applications
                    .iter()
                    .all(|application| application.provider_key != excluded_provider),
                "{excluded_provider} is outside the supported application activity scope"
            );
        }
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn registry_history_snapshot_is_aggregate_and_cleanup_is_verified() {
        let path = format!(
            r"Software\MangoDisk\Tests\Privacy-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        );
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = root.create_subkey(&path).unwrap();
        key.set_value("a", &"private command").unwrap();
        key.set_value("MRUList", &"a").unwrap();
        let (child, _) = key.create_subkey("Nested").unwrap();
        child.set_value("b", &"private path").unwrap();

        let (count, revision) = registry_snapshot(&root, &path)
            .unwrap()
            .expect("test history key should exist");
        let first_page = registry_path_details(&[path.as_str()], 0, 1).unwrap();
        let second_page = registry_path_details(&[path.as_str()], 1, 2).unwrap();

        assert_eq!(count, 2);
        assert!(!revision.contains("private"));
        assert_eq!(first_page.len(), 1);
        assert_eq!(second_page.len(), 1);
        assert!(first_page
            .iter()
            .chain(&second_page)
            .all(|entry| entry.item_count == 1));
        assert!(clear_registry_paths(&[path.as_str()]).unwrap());
        assert!(registry_snapshot(&root, &path).unwrap().is_none());
    }

    #[test]
    fn application_registry_targets_remove_only_allowlisted_keys_and_values() {
        const ROOT: &str = r"Software\MangoDisk\Tests\ApplicationPrivacyTargets";
        const HISTORY: &str = r"Software\MangoDisk\Tests\ApplicationPrivacyTargets\History";
        const TARGETS: &[RegistryTarget] = &[
            RegistryTarget::Key(HISTORY),
            RegistryTarget::Value {
                path: ROOT,
                name: "RecentPath",
            },
        ];
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let _ = root.delete_subkey_all(ROOT);
        let (key, _) = root.create_subkey(ROOT).unwrap();
        key.set_value("RecentPath", &"private path").unwrap();
        key.set_value("UnrelatedSetting", &42_u32).unwrap();
        let (history, _) = root.create_subkey(HISTORY).unwrap();
        history.set_value("Entry", &"private document").unwrap();

        let (count, revision) = registry_targets_snapshot(TARGETS).unwrap();

        assert_eq!(count, 2);
        assert!(!revision.contains("private"));
        assert!(clear_registry_targets(TARGETS).unwrap());
        let key = root.open_subkey_with_flags(ROOT, KEY_READ).unwrap();
        assert_eq!(key.get_value::<u32, _>("UnrelatedSetting").unwrap(), 42);
        assert!(key.get_raw_value("RecentPath").is_err());
        assert!(root.open_subkey_with_flags(HISTORY, KEY_READ).is_err());
        root.delete_subkey_all(ROOT).unwrap();
    }

    #[test]
    fn metadata_keys_are_cleaned_without_inflating_logical_history_count() {
        const ROOT: &str = r"Software\MangoDisk\Tests\RemoteDesktopPrivacyTargets";
        const DEFAULT: &str = r"Software\MangoDisk\Tests\RemoteDesktopPrivacyTargets\Default";
        const SERVERS: &str = r"Software\MangoDisk\Tests\RemoteDesktopPrivacyTargets\Servers";
        const TARGETS: &[RegistryTarget] = &[
            RegistryTarget::MetadataKey(SERVERS),
            RegistryTarget::Value {
                path: DEFAULT,
                name: "MRU0",
            },
        ];
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let _ = root.delete_subkey_all(ROOT);
        let (default, _) = root.create_subkey(DEFAULT).unwrap();
        default.set_value("MRU0", &"example.invalid").unwrap();
        default.set_value("UnrelatedSetting", &42_u32).unwrap();
        let (server, _) = root
            .create_subkey(format!(r"{SERVERS}\example.invalid"))
            .unwrap();
        server.set_value("CertHash", &123_u32).unwrap();
        server.set_value("UsernameHint", &"private-user").unwrap();

        let (count, revision) = registry_targets_snapshot(TARGETS).unwrap();

        assert_eq!(count, 1, "only MRU0 is one user-visible connection");
        assert!(!revision.contains("example.invalid"));
        assert!(!revision.contains("private-user"));
        assert!(clear_registry_targets(TARGETS).unwrap());
        assert!(registry_targets_are_absent(&root, TARGETS).unwrap());
        let default = root.open_subkey_with_flags(DEFAULT, KEY_READ).unwrap();
        assert_eq!(default.get_value::<u32, _>("UnrelatedSetting").unwrap(), 42);
        assert!(root.open_subkey_with_flags(SERVERS, KEY_READ).is_err());
        root.delete_subkey_all(ROOT).unwrap();
    }

    #[test]
    fn office_mru_counts_only_numbered_items_and_preserves_default_locations() {
        let base = format!(
            r"Software\MangoDisk\Tests\OfficeMru-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        );
        let direct_path = format!(r"{base}\File MRU");
        let user_path = format!(r"{base}\User MRU");
        let account_path = format!(r"{user_path}\AccountFixture\File MRU");
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let _ = root.delete_subkey_all(&base);
        let (direct, _) = root.create_subkey(&direct_path).unwrap();
        direct
            .set_value("FOLDERID_Documents", &"default location")
            .unwrap();
        direct
            .set_value("Item 1", &"[F00000000]*C:\\Fixture\\one.docx")
            .unwrap();
        let (account, _) = root.create_subkey(&account_path).unwrap();
        account
            .set_value("FOLDERID_Desktop", &"default location")
            .unwrap();
        account
            .set_value("Item 1", &"[F00000000]*C:\\Fixture\\two.docx")
            .unwrap();
        account
            .set_value("Item 2", &"[F00000000]*C:\\Fixture\\three.docx")
            .unwrap();

        let direct_snapshot = office_mru_snapshot(&root, &direct_path, None, "Word").unwrap();
        let account_snapshot =
            office_mru_snapshot(&root, &user_path, Some("File MRU"), "Word").unwrap();
        let mut page = RegistryDetailPage::new(0, 10);
        assert!(!collect_office_mru_details(&root, &direct_path, None, &mut page,).unwrap());
        assert!(
            !collect_office_mru_details(&root, &user_path, Some("File MRU"), &mut page,).unwrap()
        );

        assert_eq!(direct_snapshot.0, 1);
        assert_eq!(account_snapshot.0, 2);
        assert_eq!(page.entries.len(), 3);
        assert!(page
            .entries
            .iter()
            .all(|entry| entry.label.starts_with("C:\\Fixture\\")));
        clear_office_mru_values(&root, &direct_path, None).unwrap();
        clear_office_mru_values(&root, &user_path, Some("File MRU")).unwrap();
        assert!(!office_mru_contains_items(&root, &direct_path, None).unwrap());
        assert!(!office_mru_contains_items(&root, &user_path, Some("File MRU")).unwrap());
        assert_eq!(
            root.open_subkey_with_flags(&direct_path, KEY_READ)
                .unwrap()
                .get_value::<String, _>("FOLDERID_Documents")
                .unwrap(),
            "default location"
        );
        root.delete_subkey_all(&base).unwrap();
    }

    #[test]
    fn office_application_targets_are_independent() {
        for (application, groups) in [
            (
                "Word",
                [
                    MICROSOFT_WORD_RECENT_DOCUMENTS,
                    MICROSOFT_WORD_RECENT_PATHS,
                    MICROSOFT_WORD_RECENT_SEARCHES,
                ],
            ),
            (
                "Excel",
                [
                    MICROSOFT_EXCEL_RECENT_DOCUMENTS,
                    MICROSOFT_EXCEL_RECENT_PATHS,
                    MICROSOFT_EXCEL_RECENT_SEARCHES,
                ],
            ),
            (
                "PowerPoint",
                [
                    MICROSOFT_POWERPOINT_RECENT_DOCUMENTS,
                    MICROSOFT_POWERPOINT_RECENT_PATHS,
                    MICROSOFT_POWERPOINT_RECENT_SEARCHES,
                ],
            ),
        ] {
            assert!(groups.into_iter().flatten().all(|target| match target {
                RegistryTarget::Key(path) | RegistryTarget::MetadataKey(path) => {
                    path.contains(application)
                }
                RegistryTarget::OfficeMru {
                    path,
                    application: owner,
                    ..
                } => path.contains(application) && owner == &application,
                RegistryTarget::Value { path, .. }
                | RegistryTarget::DirectChildRecords { path, .. } => path.contains(application),
            }));
        }
    }

    #[test]
    fn direct_child_records_count_one_document_instead_of_its_metadata_fields() {
        let path = format!(
            r"Software\MangoDisk\Tests\DirectChildRecords-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        );
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let _ = root.delete_subkey_all(&path);
        for (child_name, file_name) in [("c1", "one.pdf"), ("c2", "two.pdf")] {
            let (child, _) = root.create_subkey(format!(r"{path}\{child_name}")).unwrap();
            child.set_value("tFileName", &file_name).unwrap();
            child.set_value("uPageCount", &42_u32).unwrap();
            child.set_value("uFileSize", &4096_u64).unwrap();
        }
        let target_path: &'static str = Box::leak(path.clone().into_boxed_str());
        let targets = [RegistryTarget::DirectChildRecords {
            path: target_path,
            label_value: "tFileName",
        }];

        let (count, revision) = registry_targets_snapshot(&targets).unwrap();
        let details = registry_target_details(&targets, 0, 10).unwrap();

        assert_eq!(count, 2);
        assert!(!revision.contains("one.pdf"));
        assert_eq!(
            details
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>(),
            vec!["one.pdf", "two.pdf"]
        );
        assert!(clear_registry_targets(&targets).unwrap());
        assert!(root.open_subkey_with_flags(&path, KEY_READ).is_err());
    }

    #[test]
    fn wps_suite_history_counts_only_the_visible_aggregate_list() {
        let base = format!(
            r"Software\MangoDisk\Tests\WpsRecent-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        );
        let visible_path = format!(r"{base}\Visible");
        let duplicate_path = format!(r"{base}\ProductDuplicate");
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let _ = root.delete_subkey_all(&base);
        let (visible, _) = root.create_subkey(&visible_path).unwrap();
        visible.set_value("1", &"C:\\Fixture\\one.docx").unwrap();
        visible.set_value("2", &"C:\\Fixture\\two.xlsx").unwrap();
        let (duplicate, _) = root.create_subkey(&duplicate_path).unwrap();
        duplicate
            .set_value("path", &"C:\\Fixture\\one.docx")
            .unwrap();
        duplicate.set_value("position", &42_u32).unwrap();
        let visible_target: &'static str = Box::leak(visible_path.into_boxed_str());
        let duplicate_target: &'static str = Box::leak(duplicate_path.into_boxed_str());
        let targets = [
            RegistryTarget::Key(visible_target),
            RegistryTarget::MetadataKey(duplicate_target),
        ];

        let (count, _) = registry_targets_snapshot(&targets).unwrap();
        let details = registry_target_details(&targets, 0, 10).unwrap();

        assert_eq!(count, 2);
        assert_eq!(details.len(), 2);
        assert!(clear_registry_targets(&targets).unwrap());
        assert!(registry_targets_are_absent(&root, &targets).unwrap());
        root.delete_subkey_all(&base).unwrap();
    }

    #[test]
    #[ignore = "reads installed productivity application privacy sources without modifying them"]
    fn actual_productivity_sources_expose_logical_records_and_details() {
        let local = dirs::data_local_dir().expect("local application data must be available");
        let roaming = dirs::data_dir().expect("roaming application data must be available");
        let applications = application_privacy_sources(&local, &roaming);
        for provider_key in ["word", "excel", "powerpoint", "wps_office", "adobe_reader"] {
            let application = applications
                .iter()
                .find(|application| application.provider_key == provider_key)
                .unwrap_or_else(|| panic!("installed source is missing: {provider_key}"));
            let native_traces = application
                .traces
                .iter()
                .filter(|trace| trace.native_kind.is_some())
                .collect::<Vec<_>>();
            assert!(
                !native_traces.is_empty(),
                "installed source has no native trace: {provider_key}"
            );
            for trace in native_traces {
                let details = application_details(
                    trace.native_kind.expect("filtered above"),
                    0,
                    trace.item_count.min(100) as u32,
                )
                .unwrap();
                assert_eq!(details.len() as u64, trace.item_count.min(100));
                assert!(details.iter().all(|entry| !entry.label.trim().is_empty()));
                if matches!(provider_key, "word" | "powerpoint")
                    && trace.kind == PlatformPrivacyApplicationTraceKind::RecentDocuments
                    && !details.is_empty()
                {
                    assert!(details.iter().all(|entry| {
                        entry.label.contains("://") || Path::new(&entry.label).is_absolute()
                    }));
                }
                println!(
                    "validated productivity trace provider={} kind={:?} item_count={}",
                    provider_key, trace.kind, trace.item_count
                );
            }
        }
    }

    #[test]
    #[ignore = "reads the installed VS Code history manifests without modifying them"]
    fn actual_vscode_history_displays_resources_instead_of_hash_directories() {
        let roaming = dirs::data_dir().expect("roaming application data must be available");
        let root = roaming.join("Code/User/History");
        let snapshot = vscode_history::snapshot(&root).unwrap();
        let details = vscode_history::details(&root, 0, 50_000).unwrap();

        assert!(snapshot.item_count > 0);
        assert!(!details.is_empty());
        assert_eq!(
            snapshot.item_count,
            details.iter().map(|entry| entry.item_count).sum::<u64>()
        );
        assert!(details
            .iter()
            .any(|entry| Path::new(&entry.label).is_absolute()));
        assert!(details.iter().all(|entry| {
            !entry.label.trim().is_empty()
                && !entry
                    .label
                    .rsplit(['\\', '/'])
                    .next()
                    .is_some_and(|leaf| leaf.starts_with('-') && leaf.len() <= 16)
        }));
        println!(
            "validated VS Code history resource_count={} snapshot_count={}",
            details.len(),
            snapshot.item_count
        );
    }

    #[test]
    #[ignore = "reads installed media-player histories without modifying them"]
    fn actual_media_players_expose_logical_playback_records() {
        let local = dirs::data_local_dir().expect("local application data must be available");
        let roaming = dirs::data_dir().expect("roaming application data must be available");
        let applications = application_privacy_sources(&local, &roaming);

        for provider_key in ["windows_media_player", "vlc", "potplayer"] {
            let application = applications
                .iter()
                .find(|application| application.provider_key == provider_key)
                .unwrap_or_else(|| panic!("installed media source is missing: {provider_key}"));
            assert!(
                application.application_path.is_some(),
                "installed media source has no icon identity: {provider_key}"
            );
            if provider_key == "windows_media_player" {
                assert_eq!(
                    application
                        .application_path
                        .as_ref()
                        .and_then(|path| path.file_name())
                        .and_then(|name| name.to_str()),
                    Some("MediaPlayerAppList.png")
                );
            }
            let trace = application
                .traces
                .iter()
                .find(|trace| trace.kind == PlatformPrivacyApplicationTraceKind::PlaybackHistory)
                .expect("media source must expose playback history");
            assert!(
                trace.item_count > 0,
                "playback history is empty: {provider_key}"
            );
            let details = application_details(
                trace
                    .native_kind
                    .expect("playback history must be structured"),
                0,
                trace.item_count as u32,
            )
            .unwrap();
            assert_eq!(details.len() as u64, trace.item_count);
            assert!(details.iter().all(|entry| !entry.label.trim().is_empty()));
            println!(
                "validated media trace provider={} item_count={}",
                provider_key, trace.item_count
            );
        }
    }

    #[test]
    #[ignore = "reads installed Notepad and Paint histories without modifying them"]
    fn actual_packaged_editors_expose_logical_history_records() {
        let local = dirs::data_local_dir().expect("local application data must be available");
        let roaming = dirs::data_dir().expect("roaming application data must be available");
        let applications = application_privacy_sources(&local, &roaming);

        for provider_key in ["notepad", "paint"] {
            let application = applications
                .iter()
                .find(|application| application.provider_key == provider_key)
                .unwrap_or_else(|| panic!("installed packaged source is missing: {provider_key}"));
            assert!(
                application.application_path.is_some(),
                "installed packaged source has no executable identity: {provider_key}"
            );
            let trace = application
                .traces
                .first()
                .expect("packaged source must expose one focused history trace");
            assert!(trace.item_count > 0, "history is empty: {provider_key}");
            let details = application_details(
                trace.native_kind.expect("history must be structured"),
                0,
                trace.item_count as u32,
            )
            .unwrap();
            assert_eq!(details.len() as u64, trace.item_count);
            assert!(details.iter().all(|entry| !entry.label.trim().is_empty()));
            println!(
                "validated packaged history provider={} item_count={}",
                provider_key, trace.item_count
            );
        }
    }
}
