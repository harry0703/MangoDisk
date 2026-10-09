use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use objc2_app_kit::NSPasteboard;
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};

use crate::{
    browser_profile::{chromium_display_name, firefox_display_names},
    vscode_history, PlatformCancellation, PlatformError, PlatformErrorCode,
    PlatformPrivacyApplication, PlatformPrivacyApplicationNativeTraceKind,
    PlatformPrivacyApplicationTrace, PlatformPrivacyApplicationTraceAvailability,
    PlatformPrivacyApplicationTraceKind, PlatformPrivacyBrowser, PlatformPrivacyBrowserKind,
    PlatformPrivacyDetailEntry, PlatformPrivacyDiscovery, PlatformPrivacyProfile,
    PlatformPrivacySystemTrace, PlatformPrivacySystemTraceKind, PlatformResult,
};

mod shared_file_list;
mod wps_recent_documents;

fn vscode_history_root(home: &Path) -> PathBuf {
    home.join("Library/Application Support/Code/User/History")
}

const RECENT_DOCUMENTS_LIST: &str = "com.apple.LSSharedFileList.RecentDocuments";
const RECENT_APPLICATIONS_LIST: &str = "com.apple.LSSharedFileList.RecentApplications";
const RECENT_NETWORK_LISTS: [&str; 2] = [
    "com.apple.LSSharedFileList.RecentHosts",
    "com.apple.LSSharedFileList.RecentServers",
];

pub(super) fn discover(
    cancellation: &PlatformCancellation,
) -> PlatformResult<PlatformPrivacyDiscovery> {
    let home = dirs::home_dir()
        .ok_or_else(|| PlatformError::invalid_path("home directory is unavailable"))?;
    let [brave, brave_origin] = brave_browsers(&home, cancellation)?;
    let browsers = vec![
        chromium_browser(
            "chrome",
            "Google Chrome",
            &home.join("Library/Application Support/Google/Chrome"),
            macos_application_path(&home, "Google Chrome.app"),
            vec!["Google Chrome".into(), "Google Chrome Helper".into()],
            cancellation,
        )?,
        chromium_browser(
            "edge",
            "Microsoft Edge",
            &home.join("Library/Application Support/Microsoft Edge"),
            macos_application_path(&home, "Microsoft Edge.app"),
            vec!["Microsoft Edge".into(), "Microsoft Edge Helper".into()],
            cancellation,
        )?,
        brave,
        brave_origin,
        chromium_browser(
            "opera",
            "Opera",
            &home.join("Library/Application Support/com.operasoftware.Opera"),
            macos_application_path(&home, "Opera.app"),
            vec!["Opera".into(), "Opera Helper".into()],
            cancellation,
        )?,
        chromium_browser(
            "360_safe_browser",
            "360 Safe Browser",
            &home.join("Library/Application Support/360Chrome"),
            macos_application_path(&home, "360Chrome.app"),
            vec!["360Chrome".into(), "360Chrome Helper".into()],
            cancellation,
        )?,
        chromium_browser(
            "qq_browser",
            "QQ Browser",
            &home.join("Library/Application Support/QQBrowser3"),
            macos_application_path(&home, "QQBrowser.app"),
            vec!["QQBrowser".into(), "QQBrowser Helper".into()],
            cancellation,
        )?,
        chromium_browser(
            "yandex",
            "Yandex Browser",
            &home.join("Library/Application Support/Yandex/YandexBrowser"),
            macos_application_path(&home, "Yandex.app")
                .or_else(|| macos_application_path(&home, "Yandex Browser.app")),
            vec!["Yandex".into(), "Yandex Helper".into()],
            cancellation,
        )?,
        chromium_browser(
            "vivaldi",
            "Vivaldi",
            &home.join("Library/Application Support/Vivaldi"),
            macos_application_path(&home, "Vivaldi.app"),
            vec!["Vivaldi".into(), "Vivaldi Helper".into()],
            cancellation,
        )?,
        chromium_browser(
            "chromium",
            "Chromium",
            &home.join("Library/Application Support/Chromium"),
            macos_application_path(&home, "Chromium.app"),
            vec!["Chromium".into(), "Chromium Helper".into()],
            cancellation,
        )?,
        firefox_browser(
            &home.join("Library/Application Support/Firefox/Profiles"),
            macos_application_path(&home, "Firefox.app"),
            cancellation,
        )?,
        safari_browser(&home, cancellation)?,
    ];

    let pasteboard = NSPasteboard::generalPasteboard();
    let (recent_documents_trace, recent_documents_permission_denied) =
        native_macos_system_shared_file_list_trace(
            &home,
            "recent_documents",
            "Recent documents",
            PlatformPrivacySystemTraceKind::RecentDocumentHistory,
            RECENT_DOCUMENTS_LIST,
        );
    let (recent_applications_trace, recent_applications_permission_denied) =
        native_macos_system_shared_file_list_trace(
            &home,
            "recent_applications",
            "Recent applications",
            PlatformPrivacySystemTraceKind::RecentApplications,
            RECENT_APPLICATIONS_LIST,
        );
    // Office keeps its MRU database in a protected group container. On macOS, opening that
    // directory without Full Disk Access can block in the kernel for minutes instead of returning
    // EACCES promptly. A denied system Shared File List is a cheap probe for the same TCC boundary,
    // so skip the Office directory entirely and expose a permission-required capability instead.
    let protected_application_data_available =
        !(recent_documents_permission_denied || recent_applications_permission_denied);
    if !protected_application_data_available {
        log::warn!("macos_protected_application_scan_limited reason=full_disk_access_required");
    }

    let mut traces = vec![
        PlatformPrivacySystemTrace {
            provider_key: "current_clipboard".into(),
            display_name: "Current clipboard".into(),
            kind: PlatformPrivacySystemTraceKind::CurrentClipboard,
            roots: Vec::new(),
            all_time_only: false,
            available: true,
            // AppKit can show a privacy prompt when an application enumerates pasteboard items.
            // Model the general pasteboard as one clearable slot instead of inspecting its content
            // or formats during a background scan.
            item_count: 1,
            revision: format!("pasteboard:{}", pasteboard.changeCount()),
        },
        recent_documents_trace,
        recent_applications_trace,
        native_macos_system_shared_file_list_group_trace(
            &home,
            "recent_network_connections",
            "Recent servers and hosts",
            PlatformPrivacySystemTraceKind::NetworkConnectionHistory,
            &RECENT_NETWORK_LISTS,
        ),
    ];
    if let Some(trace) = shell_history_trace(&home) {
        traces.push(trace);
    }
    Ok(PlatformPrivacyDiscovery {
        browsers,
        applications: application_privacy_sources(&home, protected_application_data_available),
        system_traces: traces,
    })
}

pub(super) fn clear(trace: PlatformPrivacySystemTraceKind) -> PlatformResult<bool> {
    match trace {
        PlatformPrivacySystemTraceKind::CurrentClipboard => {
            // `clearContents` does not read or expose clipboard content. Its returned change count
            // proves the pasteboard accepted the mutation without triggering a content-read prompt.
            let pasteboard = NSPasteboard::generalPasteboard();
            let accepted_change_count = pasteboard.clearContents();
            Ok(pasteboard.changeCount() == accepted_change_count)
        }
        PlatformPrivacySystemTraceKind::ClipboardHistory => Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "clipboard history is unavailable on macOS",
        )),
        PlatformPrivacySystemTraceKind::RecentApplications => {
            let home = dirs::home_dir()
                .ok_or_else(|| PlatformError::invalid_path("home directory is unavailable"))?;
            shared_file_list::clear_system(&home, RECENT_APPLICATIONS_LIST)
        }
        PlatformPrivacySystemTraceKind::RecentDocumentHistory => {
            let home = dirs::home_dir()
                .ok_or_else(|| PlatformError::invalid_path("home directory is unavailable"))?;
            shared_file_list::clear_system(&home, RECENT_DOCUMENTS_LIST)
        }
        PlatformPrivacySystemTraceKind::NetworkConnectionHistory => {
            let home = dirs::home_dir()
                .ok_or_else(|| PlatformError::invalid_path("home directory is unavailable"))?;
            shared_file_list::clear_system_group(&home, &RECENT_NETWORK_LISTS)
        }
        PlatformPrivacySystemTraceKind::RecentItems
        | PlatformPrivacySystemTraceKind::ApplicationUsageHistory
        | PlatformPrivacySystemTraceKind::FolderViewHistory
        | PlatformPrivacySystemTraceKind::PrinterHistory
        | PlatformPrivacySystemTraceKind::ShellHistory
        | PlatformPrivacySystemTraceKind::JumpLists
        | PlatformPrivacySystemTraceKind::RunDialogHistory
        | PlatformPrivacySystemTraceKind::FileDialogHistory
        | PlatformPrivacySystemTraceKind::ExplorerSearchHistory
        | PlatformPrivacySystemTraceKind::ExplorerPathHistory => Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "file-backed privacy traces are cleared through the Core safety boundary",
        )),
    }
}

pub(super) fn clear_application_trace(
    trace: PlatformPrivacyApplicationNativeTraceKind,
) -> PlatformResult<bool> {
    let home = dirs::home_dir()
        .ok_or_else(|| PlatformError::invalid_path("home directory is unavailable"))?;
    if let Some(application) = macos_office_recent_application(trace) {
        return clear_macos_office_recent_documents(&home, application);
    }
    if trace == PlatformPrivacyApplicationNativeTraceKind::MacVisualStudioCodeEditorHistory {
        return vscode_history::clear(&vscode_history_root(&home));
    }
    if trace == PlatformPrivacyApplicationNativeTraceKind::MacWpsOfficeRecentDocuments {
        return wps_recent_documents::clear(&home);
    }
    if let Some(bundle_identifier) = macos_shared_file_list_bundle_identifier(trace) {
        return shared_file_list::clear(&home, bundle_identifier);
    }
    Err(PlatformError::new(
        PlatformErrorCode::Unsupported,
        "the native application privacy trace is unavailable on macOS",
    ))
}

pub(super) fn system_details(
    trace: PlatformPrivacySystemTraceKind,
    offset: u64,
    limit: u32,
) -> PlatformResult<Vec<PlatformPrivacyDetailEntry>> {
    let home = dirs::home_dir()
        .ok_or_else(|| PlatformError::invalid_path("home directory is unavailable"))?;
    match trace {
        PlatformPrivacySystemTraceKind::RecentApplications => {
            shared_file_list::system_details(&home, RECENT_APPLICATIONS_LIST, offset, limit)
        }
        PlatformPrivacySystemTraceKind::RecentDocumentHistory => {
            shared_file_list::system_details(&home, RECENT_DOCUMENTS_LIST, offset, limit)
        }
        PlatformPrivacySystemTraceKind::NetworkConnectionHistory => {
            shared_file_list::system_group_details(&home, &RECENT_NETWORK_LISTS, offset, limit)
        }
        // Clipboard content deliberately remains aggregate-only because inspecting it can trigger
        // a privacy prompt. Other file-backed system traces are listed safely by Core.
        _ => Ok(Vec::new()),
    }
}

pub(super) fn application_details(
    trace: PlatformPrivacyApplicationNativeTraceKind,
    offset: u64,
    limit: u32,
) -> PlatformResult<Vec<PlatformPrivacyDetailEntry>> {
    let home = dirs::home_dir()
        .ok_or_else(|| PlatformError::invalid_path("home directory is unavailable"))?;
    if let Some(application) = macos_office_recent_application(trace) {
        return macos_office_recent_details(&home, application, offset, limit);
    }
    if trace == PlatformPrivacyApplicationNativeTraceKind::MacVisualStudioCodeEditorHistory {
        return vscode_history::details(&vscode_history_root(&home), offset, limit);
    }
    if trace == PlatformPrivacyApplicationNativeTraceKind::MacWpsOfficeRecentDocuments {
        return wps_recent_documents::details(&home, offset, limit);
    }
    if let Some(bundle_identifier) = macos_shared_file_list_bundle_identifier(trace) {
        return shared_file_list::details(&home, bundle_identifier, offset, limit);
    }
    Ok(Vec::new())
}

fn application_privacy_sources(
    home: &Path,
    protected_application_data_available: bool,
) -> Vec<PlatformPrivacyApplication> {
    let application_support = home.join("Library/Application Support");
    let caches = home.join("Library/Caches");
    let logs = home.join("Library/Logs");
    let recent_documents = application_support
        .join("com.apple.sharedfilelist/com.apple.LSSharedFileList.ApplicationRecentDocuments");
    let mut applications = Vec::new();

    // Electron applications share a predictable storage layout, but each product receives an
    // independent source identity and process allowlist. Account databases and Local Storage are
    // intentionally excluded because deleting them can remove durable user content or sign-ins.
    for definition in [
        MacApplicationDefinition {
            provider_key: "vscode",
            display_name: "Visual Studio Code",
            bundle_name: "Visual Studio Code.app",
            process_names: &["Code", "Code Helper"],
            support_directory: "Code",
            bundle_identifier: "com.microsoft.vscode",
            editor_history: true,
        },
        MacApplicationDefinition {
            provider_key: "vscodium",
            display_name: "VSCodium",
            bundle_name: "VSCodium.app",
            process_names: &["VSCodium", "VSCodium Helper"],
            support_directory: "VSCodium",
            bundle_identifier: "com.vscodium",
            editor_history: true,
        },
        MacApplicationDefinition {
            provider_key: "notion",
            display_name: "Notion",
            bundle_name: "Notion.app",
            process_names: &["Notion", "Notion Helper"],
            support_directory: "Notion",
            bundle_identifier: "notion.id",
            editor_history: false,
        },
        MacApplicationDefinition {
            provider_key: "obsidian",
            display_name: "Obsidian",
            bundle_name: "Obsidian.app",
            process_names: &["Obsidian", "Obsidian Helper"],
            support_directory: "obsidian",
            bundle_identifier: "md.obsidian",
            editor_history: false,
        },
    ] {
        let support_root = application_support.join(definition.support_directory);
        let mut traces = vec![
            file_application_trace(
                definition.provider_key,
                "Application cache",
                PlatformPrivacyApplicationTraceKind::Cache,
                [
                    support_root.join("Cache"),
                    support_root.join("Code Cache"),
                    support_root.join("GPUCache"),
                    support_root.join("CachedConfigurations"),
                    support_root.join("CachedData"),
                    support_root.join("CachedExtensionVSIXs"),
                    support_root.join("CachedProfilesData"),
                    support_root.join("Crashpad"),
                    support_root.join("DawnGraphiteCache"),
                    support_root.join("DawnWebGPUCache"),
                    support_root.join("blob_storage"),
                    support_root.join("DawnCache"),
                    support_root.join("Network Persistent State"),
                    caches.join(definition.bundle_identifier),
                ],
            ),
            file_application_trace(
                definition.provider_key,
                "Application logs",
                PlatformPrivacyApplicationTraceKind::Logs,
                [
                    support_root.join("logs"),
                    logs.join(definition.display_name),
                ],
            ),
            file_application_trace(
                definition.provider_key,
                "Application sessions",
                PlatformPrivacyApplicationTraceKind::Sessions,
                [support_root.join("Session Storage")],
            ),
            file_application_trace(
                definition.provider_key,
                "Recent documents",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                recent_document_roots(&recent_documents, definition.bundle_identifier),
            ),
        ];
        if definition.provider_key == "vscode" {
            traces
                .retain(|trace| trace.kind != PlatformPrivacyApplicationTraceKind::RecentDocuments);
            traces.push(native_macos_shared_file_list_application_trace(
                home,
                definition.provider_key,
                "Recent documents",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                definition.bundle_identifier,
                PlatformPrivacyApplicationNativeTraceKind::MacVisualStudioCodeRecentDocuments,
                protected_application_data_available,
            ));
        }
        if definition.editor_history {
            traces.push(if definition.provider_key == "vscode" {
                native_macos_vscode_history_trace(home)
            } else {
                file_application_trace(
                    definition.provider_key,
                    "Editor local history",
                    PlatformPrivacyApplicationTraceKind::EditorLocalHistory,
                    [support_root.join("User/History")],
                )
            });
        }
        push_installed_application(
            &mut applications,
            PlatformPrivacyApplication {
                provider_key: definition.provider_key.into(),
                display_name: definition.display_name.into(),
                application_path: macos_application_path(home, definition.bundle_name),
                process_names: definition
                    .process_names
                    .iter()
                    .map(|name| (*name).into())
                    .collect(),
                traces,
            },
        );
    }

    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "vlc".into(),
            display_name: "VLC".into(),
            application_path: macos_application_path(home, "VLC.app"),
            process_names: vec!["VLC".into()],
            traces: vec![
                native_macos_shared_file_list_application_trace(
                    home,
                    "vlc",
                    "Playback history",
                    PlatformPrivacyApplicationTraceKind::PlaybackHistory,
                    "org.videolan.vlc",
                    PlatformPrivacyApplicationNativeTraceKind::MacVlcRecentMedia,
                    protected_application_data_available,
                ),
                file_application_trace(
                    "vlc",
                    "Application cache",
                    PlatformPrivacyApplicationTraceKind::Cache,
                    [caches.join("org.videolan.vlc")],
                ),
            ],
        },
    );

    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "movist".into(),
            display_name: "Movist Pro".into(),
            application_path: macos_application_path(home, "Movist Pro.app")
                .or_else(|| macos_application_path(home, "Movist.app")),
            process_names: vec!["Movist Pro".into(), "Movist".into()],
            traces: vec![native_macos_shared_file_list_application_trace(
                home,
                "movist",
                "Playback history",
                PlatformPrivacyApplicationTraceKind::PlaybackHistory,
                "com.movist.movistpro",
                PlatformPrivacyApplicationNativeTraceKind::MacMovistRecentMedia,
                protected_application_data_available,
            )],
        },
    );

    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "quicktime_player".into(),
            display_name: "QuickTime Player".into(),
            application_path: macos_application_path(home, "QuickTime Player.app"),
            process_names: vec!["QuickTime Player".into()],
            traces: vec![
                native_macos_shared_file_list_application_trace(
                    home,
                    "quicktime_player",
                    "Playback history",
                    PlatformPrivacyApplicationTraceKind::PlaybackHistory,
                    "com.apple.quicktimeplayerx",
                    PlatformPrivacyApplicationNativeTraceKind::MacQuickTimeRecentMedia,
                    protected_application_data_available,
                ),
                file_application_trace(
                    "quicktime_player",
                    "Application cache",
                    PlatformPrivacyApplicationTraceKind::Cache,
                    [
                        caches.join("com.apple.QuickTimePlayerX"),
                        home.join(
                            "Library/Containers/com.apple.QuickTimePlayerX/Data/Library/Caches",
                        ),
                    ],
                ),
            ],
        },
    );

    let wps_container =
        home.join("Library/Containers/com.kingsoft.wpsoffice.mac.global/Data/Library");
    push_installed_application(
        &mut applications,
        PlatformPrivacyApplication {
            provider_key: "wps_office".into(),
            display_name: "WPS Office".into(),
            application_path: macos_application_path(home, "WPS Office.app")
                .or_else(|| macos_application_path(home, "wpsoffice.app")),
            process_names: vec![
                "wpsoffice".into(),
                "WPS Office".into(),
                "wpscloudsvr".into(),
                "promecefpluginhost".into(),
            ],
            traces: vec![
                file_application_trace(
                    "wps_office",
                    "Application cache",
                    PlatformPrivacyApplicationTraceKind::Cache,
                    [
                        caches.join("com.kingsoft.wpsoffice.mac"),
                        wps_container.join("Caches/com.kingsoft.wpsoffice.mac.global"),
                    ],
                ),
                native_macos_wps_recent_documents_trace(
                    home,
                    "wps_office",
                    "Recent documents",
                    PlatformPrivacyApplicationTraceKind::RecentDocuments,
                    protected_application_data_available,
                ),
            ],
        },
    );

    for definition in [
        (
            "libreoffice",
            "LibreOffice",
            "LibreOffice.app",
            "soffice",
            "org.libreoffice.script",
        ),
        (
            "adobe_acrobat",
            "Adobe Acrobat",
            "Adobe Acrobat.app",
            "AdobeAcrobat",
            "com.adobe.Acrobat.Pro",
        ),
    ] {
        push_installed_application(
            &mut applications,
            PlatformPrivacyApplication {
                provider_key: definition.0.into(),
                display_name: definition.1.into(),
                application_path: macos_application_path(home, definition.2),
                process_names: vec![definition.3.into()],
                traces: vec![
                    file_application_trace(
                        definition.0,
                        "Recent documents",
                        PlatformPrivacyApplicationTraceKind::RecentDocuments,
                        recent_document_roots(&recent_documents, definition.4),
                    ),
                    file_application_trace(
                        definition.0,
                        "Application cache",
                        PlatformPrivacyApplicationTraceKind::Cache,
                        [caches.join(definition.4)],
                    ),
                ],
            },
        );
    }

    // Shared recent-item lists are NSKeyedArchiver containers. Treating the container as an
    // ordinary file reports every application as one trace regardless of the records it contains.
    // Native variants keep the fixed bundle allowlist in the platform layer while
    // exposing only aggregate counts and user-requested document names to Core.
    for definition in [
        (
            "pages",
            "Pages",
            "Pages.app",
            "Pages",
            "com.apple.iwork.pages",
            "Recent documents",
            PlatformPrivacyApplicationTraceKind::RecentDocuments,
            PlatformPrivacyApplicationNativeTraceKind::MacPagesRecentDocuments,
        ),
        (
            "numbers",
            "Numbers",
            "Numbers.app",
            "Numbers",
            "com.apple.iwork.numbers",
            "Recent documents",
            PlatformPrivacyApplicationTraceKind::RecentDocuments,
            PlatformPrivacyApplicationNativeTraceKind::MacNumbersRecentDocuments,
        ),
        (
            "keynote",
            "Keynote",
            "Keynote.app",
            "Keynote",
            "com.apple.iwork.keynote",
            "Recent documents",
            PlatformPrivacyApplicationTraceKind::RecentDocuments,
            PlatformPrivacyApplicationNativeTraceKind::MacKeynoteRecentDocuments,
        ),
        (
            "pdf_expert",
            "PDF Expert",
            "PDF Expert.app",
            "PDF Expert",
            "com.readdle.pdfexpert-mac",
            "Recent documents",
            PlatformPrivacyApplicationTraceKind::RecentDocuments,
            PlatformPrivacyApplicationNativeTraceKind::MacPdfExpertRecentDocuments,
        ),
        (
            "xcode",
            "Xcode",
            "Xcode.app",
            "Xcode",
            "com.apple.dt.xcode",
            "Recent projects",
            PlatformPrivacyApplicationTraceKind::RecentProjects,
            PlatformPrivacyApplicationNativeTraceKind::MacXcodeRecentProjects,
        ),
    ] {
        push_installed_application(
            &mut applications,
            PlatformPrivacyApplication {
                provider_key: definition.0.into(),
                display_name: definition.1.into(),
                application_path: macos_application_path(home, definition.2),
                process_names: vec![definition.3.into()],
                traces: vec![
                    native_macos_shared_file_list_application_trace(
                        home,
                        definition.0,
                        definition.5,
                        definition.6,
                        definition.4,
                        definition.7,
                        protected_application_data_available,
                    ),
                    file_application_trace(
                        definition.0,
                        "Application cache",
                        PlatformPrivacyApplicationTraceKind::Cache,
                        [
                            caches.join(definition.4),
                            home.join("Library/Containers")
                                .join(definition.4)
                                .join("Data/Library/Caches"),
                        ],
                    ),
                ],
            },
        );
    }

    for definition in [
        (
            "word",
            "Microsoft Word",
            "Microsoft Word.app",
            "Microsoft Word",
            "com.microsoft.word",
            "Word",
            PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftWordRecentDocuments,
        ),
        (
            "excel",
            "Microsoft Excel",
            "Microsoft Excel.app",
            "Microsoft Excel",
            "com.microsoft.excel",
            "Excel",
            PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftExcelRecentDocuments,
        ),
        (
            "powerpoint",
            "Microsoft PowerPoint",
            "Microsoft PowerPoint.app",
            "Microsoft PowerPoint",
            "com.microsoft.powerpoint",
            "PowerPoint",
            PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftPowerPointRecentDocuments,
        ),
    ] {
        push_installed_application(
            &mut applications,
            PlatformPrivacyApplication {
                provider_key: definition.0.into(),
                display_name: definition.1.into(),
                application_path: macos_application_path(home, definition.2),
                process_names: vec![definition.3.into()],
                traces: vec![
                    native_macos_office_application_trace(
                        home,
                        definition.0,
                        definition.5,
                        definition.6,
                        protected_application_data_available,
                    ),
                    file_application_trace(
                        definition.0,
                        "Application cache",
                        PlatformPrivacyApplicationTraceKind::Cache,
                        [
                            caches.join(definition.4),
                            home.join("Library/Containers")
                                .join(definition.4)
                                .join("Data/Library/Caches"),
                        ],
                    ),
                ],
            },
        );
    }

    for definition in [
        (
            "onenote",
            "Microsoft OneNote",
            "Microsoft OneNote.app",
            "Microsoft OneNote",
            "com.microsoft.onenote.mac",
            false,
        ),
        (
            "preview",
            "Preview",
            "Preview.app",
            "Preview",
            "com.apple.preview",
            false,
        ),
        (
            "textedit",
            "TextEdit",
            "TextEdit.app",
            "TextEdit",
            "com.apple.textedit",
            false,
        ),
        (
            "skim",
            "Skim",
            "Skim.app",
            "Skim",
            "net.sourceforge.skim-app.skim",
            false,
        ),
    ] {
        let recent_kind = if definition.5 {
            PlatformPrivacyApplicationTraceKind::RecentProjects
        } else {
            PlatformPrivacyApplicationTraceKind::RecentDocuments
        };
        let native_kind = match definition.0 {
            "preview" => Some(PlatformPrivacyApplicationNativeTraceKind::MacPreviewRecentDocuments),
            "textedit" => {
                Some(PlatformPrivacyApplicationNativeTraceKind::MacTextEditRecentDocuments)
            }
            "skim" => Some(PlatformPrivacyApplicationNativeTraceKind::MacSkimRecentDocuments),
            _ => None,
        };
        let recent_trace = if let Some(native_kind) = native_kind {
            native_macos_shared_file_list_application_trace(
                home,
                definition.0,
                "Recent documents",
                recent_kind,
                definition.4,
                native_kind,
                protected_application_data_available,
            )
        } else {
            file_application_trace(
                definition.0,
                if definition.5 {
                    "Recent projects"
                } else {
                    "Recent documents"
                },
                recent_kind,
                recent_document_roots(&recent_documents, definition.4),
            )
        };
        push_installed_application(
            &mut applications,
            PlatformPrivacyApplication {
                provider_key: definition.0.into(),
                display_name: definition.1.into(),
                application_path: macos_application_path(home, definition.2),
                process_names: vec![definition.3.into()],
                traces: vec![
                    recent_trace,
                    file_application_trace(
                        definition.0,
                        "Application cache",
                        PlatformPrivacyApplicationTraceKind::Cache,
                        [
                            caches.join(definition.4),
                            home.join("Library/Containers")
                                .join(definition.4)
                                .join("Data/Library/Caches"),
                        ],
                    ),
                ],
            },
        );
    }

    if !protected_application_data_available {
        let (trace_count, root_count) = omit_protected_application_roots(home, &mut applications);
        log::info!(
            "macos_protected_application_roots_omitted trace_count={trace_count} root_count={root_count}"
        );
    }

    applications
}

struct MacApplicationDefinition {
    provider_key: &'static str,
    display_name: &'static str,
    bundle_name: &'static str,
    process_names: &'static [&'static str],
    support_directory: &'static str,
    bundle_identifier: &'static str,
    editor_history: bool,
}

fn recent_document_roots(root: &Path, bundle_identifier: &str) -> Vec<PathBuf> {
    ["sfl3", "sfl2"]
        .into_iter()
        .map(|extension| root.join(format!("{bundle_identifier}.{extension}")))
        .collect()
}

fn macos_shared_file_list_bundle_identifier(
    trace: PlatformPrivacyApplicationNativeTraceKind,
) -> Option<&'static str> {
    match trace {
        PlatformPrivacyApplicationNativeTraceKind::MacPreviewRecentDocuments => {
            Some("com.apple.preview")
        }
        PlatformPrivacyApplicationNativeTraceKind::MacPdfExpertRecentDocuments => {
            Some("com.readdle.pdfexpert-mac")
        }
        PlatformPrivacyApplicationNativeTraceKind::MacTextEditRecentDocuments => {
            Some("com.apple.textedit")
        }
        PlatformPrivacyApplicationNativeTraceKind::MacSkimRecentDocuments => {
            Some("net.sourceforge.skim-app.skim")
        }
        PlatformPrivacyApplicationNativeTraceKind::MacPagesRecentDocuments => {
            Some("com.apple.iwork.pages")
        }
        PlatformPrivacyApplicationNativeTraceKind::MacNumbersRecentDocuments => {
            Some("com.apple.iwork.numbers")
        }
        PlatformPrivacyApplicationNativeTraceKind::MacKeynoteRecentDocuments => {
            Some("com.apple.iwork.keynote")
        }
        PlatformPrivacyApplicationNativeTraceKind::MacVisualStudioCodeRecentDocuments => {
            // Shared File List storage normalizes VS Code's bundle identifier to lowercase.
            // Using the on-disk name also keeps exported archives and case-sensitive fixtures
            // consistent with the real macOS source.
            Some("com.microsoft.vscode")
        }
        PlatformPrivacyApplicationNativeTraceKind::MacXcodeRecentProjects => {
            Some("com.apple.dt.xcode")
        }
        PlatformPrivacyApplicationNativeTraceKind::MacVlcRecentMedia => Some("org.videolan.vlc"),
        PlatformPrivacyApplicationNativeTraceKind::MacMovistRecentMedia => {
            Some("com.movist.movistpro")
        }
        PlatformPrivacyApplicationNativeTraceKind::MacQuickTimeRecentMedia => {
            // Application recent-document list names are normalized to lowercase on disk even
            // though QuickTime Player's declared bundle identifier contains uppercase letters.
            Some("com.apple.quicktimeplayerx")
        }
        _ => None,
    }
}

fn native_macos_vscode_history_trace(home: &Path) -> PlatformPrivacyApplicationTrace {
    let snapshot = vscode_history::snapshot(&vscode_history_root(home));
    let (availability, item_count, revision) = match snapshot {
        Ok(snapshot) => (
            PlatformPrivacyApplicationTraceAvailability::Available,
            snapshot.item_count,
            snapshot.revision,
        ),
        Err(error) => {
            log::warn!("macos_vscode_history_scan_failed code={:?}", error.code());
            (
                application_trace_availability_for_error(&error),
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
            PlatformPrivacyApplicationNativeTraceKind::MacVisualStudioCodeEditorHistory,
        ),
        all_time_only: true,
        availability,
        item_count,
        revision,
    }
}

fn native_macos_system_shared_file_list_trace(
    home: &Path,
    provider_key: &str,
    display_name: &str,
    kind: PlatformPrivacySystemTraceKind,
    list_identifier: &str,
) -> (PlatformPrivacySystemTrace, bool) {
    match shared_file_list::system_snapshot(home, list_identifier) {
        Ok(snapshot) => (
            PlatformPrivacySystemTrace {
                provider_key: provider_key.into(),
                display_name: display_name.into(),
                kind,
                roots: Vec::new(),
                all_time_only: true,
                available: true,
                item_count: snapshot.item_count,
                revision: snapshot.revision,
            },
            false,
        ),
        Err(error) => {
            log::warn!(
                "macos_system_shared_file_list_scan_failed source={} code={:?}",
                provider_key,
                error.code()
            );
            let permission_denied = error.code() == PlatformErrorCode::AccessDenied;
            (
                PlatformPrivacySystemTrace {
                    provider_key: provider_key.into(),
                    display_name: display_name.into(),
                    kind,
                    roots: Vec::new(),
                    all_time_only: true,
                    available: false,
                    item_count: 0,
                    revision: "unavailable".into(),
                },
                permission_denied,
            )
        }
    }
}

fn native_macos_system_shared_file_list_group_trace(
    home: &Path,
    provider_key: &str,
    display_name: &str,
    kind: PlatformPrivacySystemTraceKind,
    list_identifiers: &[&str],
) -> PlatformPrivacySystemTrace {
    match shared_file_list::system_group_snapshot(home, list_identifiers) {
        Ok(snapshot) => PlatformPrivacySystemTrace {
            provider_key: provider_key.into(),
            display_name: display_name.into(),
            kind,
            roots: Vec::new(),
            all_time_only: true,
            available: true,
            item_count: snapshot.item_count,
            revision: snapshot.revision,
        },
        Err(error) => {
            log::warn!(
                "macos_system_shared_file_list_group_scan_failed source={} code={:?}",
                provider_key,
                error.code()
            );
            PlatformPrivacySystemTrace {
                provider_key: provider_key.into(),
                display_name: display_name.into(),
                kind,
                roots: Vec::new(),
                all_time_only: true,
                available: false,
                item_count: 0,
                revision: "unavailable".into(),
            }
        }
    }
}

fn native_macos_shared_file_list_application_trace(
    home: &Path,
    application_key: &str,
    display_name: &str,
    kind: PlatformPrivacyApplicationTraceKind,
    bundle_identifier: &str,
    native_kind: PlatformPrivacyApplicationNativeTraceKind,
    protected_application_data_available: bool,
) -> PlatformPrivacyApplicationTrace {
    if !protected_application_data_available {
        return permission_required_application_trace(
            application_key,
            display_name,
            kind,
            native_kind,
        );
    }
    let snapshot = shared_file_list::snapshot(home, bundle_identifier);
    let (availability, item_count, revision) = match snapshot {
        Ok(snapshot) => (
            PlatformPrivacyApplicationTraceAvailability::Available,
            snapshot.item_count,
            snapshot.revision,
        ),
        Err(error) => {
            log::warn!(
                "macos_shared_file_list_scan_failed application={} code={:?}",
                application_key,
                error.code()
            );
            (
                application_trace_availability_for_error(&error),
                0,
                "unavailable".into(),
            )
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

fn native_macos_wps_recent_documents_trace(
    home: &Path,
    application_key: &str,
    display_name: &str,
    kind: PlatformPrivacyApplicationTraceKind,
    protected_application_data_available: bool,
) -> PlatformPrivacyApplicationTrace {
    let native_kind = PlatformPrivacyApplicationNativeTraceKind::MacWpsOfficeRecentDocuments;
    if !protected_application_data_available {
        return permission_required_application_trace(
            application_key,
            display_name,
            kind,
            native_kind,
        );
    }
    let snapshot = wps_recent_documents::snapshot(home);
    let (availability, item_count, revision) = match snapshot {
        Ok(snapshot) => (
            PlatformPrivacyApplicationTraceAvailability::Available,
            snapshot.item_count,
            snapshot.revision,
        ),
        Err(error) => {
            log::warn!(
                "macos_wps_recent_documents_scan_failed application={} code={:?}",
                application_key,
                error.code()
            );
            (
                application_trace_availability_for_error(&error),
                0,
                "unavailable".into(),
            )
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

#[derive(Debug)]
struct MacOsOfficeRecentEntry {
    node_id: i64,
    record_id: String,
    file_name: String,
    timestamp: String,
    document_url: String,
}

fn macos_office_recent_application(
    trace: PlatformPrivacyApplicationNativeTraceKind,
) -> Option<&'static str> {
    match trace {
        PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftWordRecentDocuments => Some("Word"),
        PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftExcelRecentDocuments => {
            Some("Excel")
        }
        PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftPowerPointRecentDocuments => {
            Some("PowerPoint")
        }
        _ => None,
    }
}

fn native_macos_office_application_trace(
    home: &Path,
    application_key: &str,
    office_application: &str,
    native_kind: PlatformPrivacyApplicationNativeTraceKind,
    protected_application_data_available: bool,
) -> PlatformPrivacyApplicationTrace {
    if !protected_application_data_available {
        return permission_required_application_trace(
            application_key,
            "Recent documents",
            PlatformPrivacyApplicationTraceKind::RecentDocuments,
            native_kind,
        );
    }
    let snapshot = macos_office_recent_snapshot(home, office_application);
    let (availability, item_count, revision) = match snapshot {
        Ok((count, revision)) => (
            PlatformPrivacyApplicationTraceAvailability::Available,
            count,
            revision,
        ),
        Err(error) => {
            log::warn!(
                "macos_office_recent_scan_failed application={} code={:?}",
                office_application,
                error.code()
            );
            (
                application_trace_availability_for_error(&error),
                0,
                "unavailable".into(),
            )
        }
    };
    PlatformPrivacyApplicationTrace {
        provider_key: format!(
            "{application_key}:{}",
            application_trace_key(PlatformPrivacyApplicationTraceKind::RecentDocuments)
        ),
        display_name: "Recent documents".into(),
        kind: PlatformPrivacyApplicationTraceKind::RecentDocuments,
        roots: Vec::new(),
        native_kind: Some(native_kind),
        all_time_only: true,
        availability,
        item_count,
        revision,
    }
}

fn permission_required_application_trace(
    application_key: &str,
    display_name: &str,
    kind: PlatformPrivacyApplicationTraceKind,
    native_kind: PlatformPrivacyApplicationNativeTraceKind,
) -> PlatformPrivacyApplicationTrace {
    PlatformPrivacyApplicationTrace {
        provider_key: format!("{application_key}:{}", application_trace_key(kind)),
        display_name: display_name.into(),
        kind,
        roots: Vec::new(),
        native_kind: Some(native_kind),
        all_time_only: true,
        availability: PlatformPrivacyApplicationTraceAvailability::PermissionRequired,
        item_count: 0,
        revision: "permission-required".into(),
    }
}

fn application_trace_availability_for_error(
    error: &PlatformError,
) -> PlatformPrivacyApplicationTraceAvailability {
    if error.code() == PlatformErrorCode::AccessDenied {
        PlatformPrivacyApplicationTraceAvailability::PermissionRequired
    } else {
        PlatformPrivacyApplicationTraceAvailability::Unavailable
    }
}

fn macos_office_registration_databases(home: &Path) -> PlatformResult<Vec<PathBuf>> {
    let root = home.join("Library/Group Containers/UBF8T346G9.Office/MicrosoftRegistrationDB");
    let metadata = match fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(PlatformError::io(
                "inspect the Microsoft Office registration database directory",
                &error,
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PlatformError::invalid_path(
            "the Microsoft Office registration database directory is not a safe directory",
        ));
    }

    let entries = fs::read_dir(&root).map_err(|error| {
        PlatformError::io("enumerate Microsoft Office registration databases", &error)
    })?;
    let mut databases = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            PlatformError::io(
                "read a Microsoft Office registration database entry",
                &error,
            )
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("MicrosoftRegistrationDB_") || !name.ends_with(".reg") {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            PlatformError::io("inspect a Microsoft Office registration database", &error)
        })?;
        if metadata.is_file() && !metadata.file_type().is_symlink() {
            databases.push(path);
        }
    }
    databases.sort();
    Ok(databases)
}

fn office_recent_patterns(application: &str) -> (String, String) {
    let direct = format!(
        r"Software\Microsoft\Office\*\Common\MruUserData\*\{application}\Local\Documents\*"
    );
    let nested = format!(r"{direct}\*");
    (direct, nested)
}

fn office_database_error(operation: &'static str, error: &rusqlite::Error) -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::OperationFailed,
        format!("{operation}: {error}"),
    )
}

fn read_macos_office_recent_entries(
    connection: &Connection,
    application: &str,
) -> PlatformResult<Vec<MacOsOfficeRecentEntry>> {
    const QUERY: &str = r#"
        WITH RECURSIVE registry_path(node_id, name, path) AS (
            SELECT node_id, name, name
            FROM HKEY_CURRENT_USER
            WHERE parent_id = -1
            UNION ALL
            SELECT child.node_id, child.name, registry_path.path || '\' || child.name
            FROM HKEY_CURRENT_USER AS child
            JOIN registry_path ON child.parent_id = registry_path.node_id
        )
        SELECT entry.node_id,
               entry.name,
               COALESCE(CAST((
                   SELECT value FROM HKEY_CURRENT_USER_values
                   WHERE node_id = entry.node_id AND name = 'FileName'
                   LIMIT 1
               ) AS TEXT), ''),
               COALESCE(CAST((
                   SELECT value FROM HKEY_CURRENT_USER_values
                   WHERE node_id = entry.node_id AND name = 'Timestamp'
                   LIMIT 1
               ) AS TEXT), ''),
               COALESCE(CAST((
                   SELECT value FROM HKEY_CURRENT_USER_values
                   WHERE node_id = entry.node_id AND name = 'DocumentUrl'
                   LIMIT 1
               ) AS TEXT), '')
        FROM registry_path AS entry
        WHERE entry.path GLOB ?1
          AND entry.path NOT GLOB ?2
          AND NOT EXISTS (
              SELECT 1
              FROM HKEY_CURRENT_USER_values AS pinned
              WHERE pinned.node_id = entry.node_id
                AND pinned.name = 'IsPinned'
                AND CAST(pinned.value AS TEXT) <> '0'
          )
        ORDER BY 4 DESC, entry.node_id DESC
    "#;
    let (direct_pattern, nested_pattern) = office_recent_patterns(application);
    let mut statement = connection.prepare(QUERY).map_err(|error| {
        office_database_error("prepare the Office recent-document query", &error)
    })?;
    let rows = statement
        .query_map(params![direct_pattern, nested_pattern], |row| {
            Ok(MacOsOfficeRecentEntry {
                node_id: row.get(0)?,
                record_id: row.get(1)?,
                file_name: row.get(2)?,
                timestamp: row.get(3)?,
                document_url: row.get(4)?,
            })
        })
        .map_err(|error| office_database_error("query Office recent documents", &error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| office_database_error("decode Office recent documents", &error))
}

fn open_macos_office_database(path: &Path, read_only: bool) -> PlatformResult<Connection> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    Connection::open_with_flags(path, flags)
        .map_err(|error| office_database_error("open the Office registration database", &error))
}

fn macos_office_recent_snapshot(home: &Path, application: &str) -> PlatformResult<(u64, String)> {
    let databases = macos_office_registration_databases(home)?;
    let mut count = 0_u64;
    let mut revision = blake3::Hasher::new();
    revision.update(b"mangodisk-macos-office-mru-v1\0");
    revision.update(application.as_bytes());
    for database in &databases {
        let connection = open_macos_office_database(database, true)?;
        for entry in read_macos_office_recent_entries(&connection, application)? {
            count = count.saturating_add(1);
            for value in [
                entry.record_id.as_str(),
                entry.file_name.as_str(),
                entry.timestamp.as_str(),
                entry.document_url.as_str(),
            ] {
                revision.update(value.as_bytes());
                revision.update(b"\0");
            }
        }
    }
    log::debug!(
        "macos_office_recent_scanned application={} database_count={} item_count={}",
        application,
        databases.len(),
        count
    );
    Ok((count, revision.finalize().to_hex().to_string()))
}

fn macos_office_recent_details(
    home: &Path,
    application: &str,
    offset: u64,
    limit: u32,
) -> PlatformResult<Vec<PlatformPrivacyDetailEntry>> {
    let mut entries = Vec::new();
    for database in macos_office_registration_databases(home)? {
        let connection = open_macos_office_database(&database, true)?;
        entries.extend(read_macos_office_recent_entries(&connection, application)?);
    }
    entries.sort_by(|left, right| {
        right
            .timestamp
            .cmp(&left.timestamp)
            .then_with(|| right.node_id.cmp(&left.node_id))
    });
    Ok(entries
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|entry| {
            let document_path = vscode_history::resource_label(&entry.document_url);
            PlatformPrivacyDetailEntry {
                label: if document_path.is_empty() {
                    entry.file_name
                } else {
                    document_path
                },
                item_count: 1,
            }
        })
        .collect())
}

fn clear_macos_office_recent_documents(home: &Path, application: &str) -> PlatformResult<bool> {
    let databases = macos_office_registration_databases(home)?;
    let mut removed = 0_u64;
    for database in &databases {
        let mut connection = open_macos_office_database(database, false)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| {
                office_database_error("begin the Office recent-document transaction", &error)
            })?;
        let entries = read_macos_office_recent_entries(&transaction, application)?;

        // MRU document nodes are expected to be leaves. Refusing an unknown nested shape prevents
        // a future Office schema change from broadening deletion beyond the reviewed record.
        for entry in &entries {
            let child_count: i64 = transaction
                .query_row(
                    "SELECT COUNT(*) FROM HKEY_CURRENT_USER WHERE parent_id = ?1",
                    [entry.node_id],
                    |row| row.get(0),
                )
                .map_err(|error| {
                    office_database_error("verify the Office recent-document record", &error)
                })?;
            if child_count != 0 {
                return Err(PlatformError::new(
                    PlatformErrorCode::InvalidData,
                    "an Office recent-document record has an unsupported nested shape",
                ));
            }
        }

        for entry in &entries {
            transaction
                .execute(
                    "DELETE FROM HKEY_CURRENT_USER_values WHERE node_id = ?1",
                    [entry.node_id],
                )
                .map_err(|error| {
                    office_database_error("delete Office recent-document values", &error)
                        .with_possible_side_effects()
                })?;
            let deleted = transaction
                .execute(
                    "DELETE FROM HKEY_CURRENT_USER WHERE node_id = ?1",
                    [entry.node_id],
                )
                .map_err(|error| {
                    office_database_error("delete the Office recent-document record", &error)
                        .with_possible_side_effects()
                })?;
            removed = removed.saturating_add(deleted as u64);
        }
        transaction.commit().map_err(|error| {
            office_database_error("commit the Office recent-document transaction", &error)
                .with_possible_side_effects()
        })?;
    }

    let (remaining, _) = macos_office_recent_snapshot(home, application)?;
    log::info!(
        "macos_office_recent_cleared application={} database_count={} removed_count={} remaining_count={}",
        application,
        databases.len(),
        removed,
        remaining
    );
    Ok(remaining == 0)
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
            .filter(|path| existing_path_without_links(path))
            .collect(),
        native_kind: None,
        all_time_only: true,
        availability: PlatformPrivacyApplicationTraceAvailability::Available,
        item_count: 0,
        revision: String::new(),
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

#[cfg(test)]
fn fixed_file_system_trace(
    provider_key: &str,
    display_name: &str,
    kind: PlatformPrivacySystemTraceKind,
    candidates: impl IntoIterator<Item = PathBuf>,
) -> PlatformPrivacySystemTrace {
    let roots = candidates
        .into_iter()
        .filter(|path| existing_path_without_links(path))
        .collect::<Vec<_>>();
    PlatformPrivacySystemTrace {
        provider_key: provider_key.into(),
        display_name: display_name.into(),
        kind,
        item_count: roots.len() as u64,
        roots,
        all_time_only: true,
        available: true,
        revision: "fixed-files".into(),
    }
}

fn existing_path_without_links(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(metadata) => !metadata.file_type().is_symlink(),
        Err(error) => error.kind() == std::io::ErrorKind::PermissionDenied,
    }
}

fn omit_protected_application_roots(
    home: &Path,
    applications: &mut [PlatformPrivacyApplication],
) -> (usize, usize) {
    let protected_roots = [
        home.join("Library/Containers"),
        home.join("Library/Group Containers"),
        home.join("Library/Application Support/com.apple.sharedfilelist"),
    ];
    let mut affected_trace_count = 0_usize;
    let mut omitted_root_count = 0_usize;

    for trace in applications
        .iter_mut()
        .flat_map(|application| application.traces.iter_mut())
    {
        let previous_root_count = trace.roots.len();
        trace.roots.retain(|root| {
            !protected_roots
                .iter()
                .any(|protected_root| root.starts_with(protected_root))
        });
        let removed = previous_root_count.saturating_sub(trace.roots.len());
        if removed == 0 {
            continue;
        }
        affected_trace_count = affected_trace_count.saturating_add(1);
        omitted_root_count = omitted_root_count.saturating_add(removed);
        if trace.roots.is_empty() {
            trace.availability = PlatformPrivacyApplicationTraceAvailability::PermissionRequired;
            trace.item_count = 0;
            trace.revision = "permission-required".into();
        }
    }

    (affected_trace_count, omitted_root_count)
}

fn push_installed_application(
    applications: &mut Vec<PlatformPrivacyApplication>,
    application: PlatformPrivacyApplication,
) {
    if application.application_path.is_some()
        || application
            .traces
            .iter()
            .any(|trace| !trace.roots.is_empty())
    {
        applications.push(application);
    }
}

fn shell_history_trace(home: &Path) -> Option<PlatformPrivacySystemTrace> {
    // These files are user-owned command or console histories with stable locations. Discovery
    // records only aggregate line counts; neither commands nor raw paths cross the platform API.
    let roots = [
        ".zsh_history",
        ".bash_history",
        ".sh_history",
        ".python_history",
        ".node_repl_history",
        ".psql_history",
        ".mysql_history",
        ".sqlite_history",
    ]
    .into_iter()
    .map(|name| home.join(name))
    .filter(|path| regular_file_without_links(path))
    .collect::<Vec<_>>();
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

fn regular_file_without_links(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

fn count_text_records(path: &Path) -> u64 {
    let Ok(file) = fs::File::open(path) else {
        return 0;
    };
    // Bound discovery work even when a history file is unexpectedly large. The displayed count
    // remains a conservative aggregate while execution still clears the complete selected file.
    BufReader::new(file)
        .split(b'\n')
        .take(100_000)
        .filter(Result::is_ok)
        .count() as u64
}

fn brave_browsers(
    home: &Path,
    cancellation: &PlatformCancellation,
) -> PlatformResult<[PlatformPrivacyBrowser; 2]> {
    Ok([
        chromium_browser(
            "brave",
            "Brave",
            &home.join("Library/Application Support/BraveSoftware/Brave-Browser"),
            macos_application_path(home, "Brave Browser.app"),
            vec!["Brave Browser".into(), "Brave Browser Helper".into()],
            cancellation,
        )?,
        chromium_browser(
            "brave-origin",
            "Brave Origin",
            &home.join("Library/Application Support/BraveSoftware/Brave-Origin"),
            macos_application_path(home, "Brave Origin.app"),
            vec!["Brave Origin".into(), "Brave Origin Helper".into()],
            cancellation,
        )?,
    ])
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
        // Detecting a root profile only when it owns a supported data source keeps the adapter
        // compatible with either layout without treating Local State as user browsing evidence.
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
            if cancellation.is_cancelled() {
                return Err(PlatformError::new(
                    PlatformErrorCode::UserCancelled,
                    "privacy source discovery cancelled",
                ));
            }
            let entry = entry.map_err(|error| PlatformError::io("read browser profile", &error))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != "Default" && !name.starts_with("Profile ") {
                continue;
            }
            let profile_root = entry.path();
            if !profile_root.is_dir() {
                continue;
            }
            profiles.push(chromium_profile(provider_key, name, profile_root));
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
        // root fallback used by current Chrome profiles on some installations.
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
        cache_directories: chromium_cache_directories(&root),
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
            if cancellation.is_cancelled() {
                return Err(PlatformError::new(
                    PlatformErrorCode::UserCancelled,
                    "privacy source discovery cancelled",
                ));
            }
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
                cache_directories: firefox_cache_directories(&profile_root),
                root: profile_root,
            });
        }
    }
    Ok(PlatformPrivacyBrowser {
        provider_key: "firefox".into(),
        display_name: "Firefox".into(),
        application_path,
        kind: PlatformPrivacyBrowserKind::Firefox,
        process_names: vec!["firefox".into()],
        profiles,
    })
}

fn safari_browser(
    home: &Path,
    cancellation: &PlatformCancellation,
) -> PlatformResult<PlatformPrivacyBrowser> {
    if cancellation.is_cancelled() {
        return Err(PlatformError::new(
            PlatformErrorCode::UserCancelled,
            "privacy source discovery cancelled",
        ));
    }
    let root = home.join("Library/Safari");
    let profile = PlatformPrivacyProfile {
        provider_key: "safari:default".into(),
        display_name: "Default".into(),
        history_database: existing_or_inaccessible_file(root.join("History.db")),
        cookie_database: None,
        saved_password_source: None,
        autofill_database: None,
        permission_database: None,
        top_sites_database: None,
        shortcut_database: None,
        favicon_database: None,
        session_directories: existing_directories(&root, &["LastSession.plist"]),
        site_storage_directories: Vec::new(),
        cache_directories: existing_directories(
            &home.join("Library/Caches"),
            &["com.apple.Safari", "com.apple.WebKit.Networking"],
        ),
        root,
    };
    Ok(PlatformPrivacyBrowser {
        provider_key: "safari".into(),
        display_name: "Safari".into(),
        application_path: macos_application_path(home, "Safari.app"),
        kind: PlatformPrivacyBrowserKind::Safari,
        process_names: vec!["Safari".into()],
        profiles: if profile.history_database.is_some() {
            vec![profile]
        } else {
            Vec::new()
        },
    })
}

fn macos_application_path(home: &Path, bundle_name: &str) -> Option<PathBuf> {
    [
        home.join("Applications").join(bundle_name),
        PathBuf::from("/Applications").join(bundle_name),
        PathBuf::from("/System/Applications").join(bundle_name),
    ]
    .into_iter()
    .find(|path| path.is_dir())
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

fn chromium_cache_directories(profile_root: &Path) -> Vec<PathBuf> {
    const RELATIVES: &[&str] = &[
        "Cache",
        "Code Cache",
        "GPUCache",
        "DawnCache",
        "DawnGraphiteCache",
        "DawnWebGPUCache",
        "GrShaderCache",
        "GraphiteDawnCache",
        "Media Cache",
    ];
    let mut directories = existing_directories(profile_root, RELATIVES);
    if let Some(home) = dirs::home_dir() {
        let application_support = home.join("Library/Application Support");
        if let Ok(relative) = profile_root.strip_prefix(application_support) {
            directories.extend(existing_directories(
                &home.join("Library/Caches").join(relative),
                RELATIVES,
            ));
        }
    }
    directories.sort();
    directories.dedup();
    directories
}

fn firefox_cache_directories(profile_root: &Path) -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let profile_parent = home.join("Library/Application Support/Firefox/Profiles");
    let Ok(relative) = profile_root.strip_prefix(profile_parent) else {
        return Vec::new();
    };
    existing_directories(
        &home.join("Library/Caches/Firefox/Profiles").join(relative),
        &["cache2", "startupCache", "thumbnails"],
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    fn fixture_home(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mangodisk-macos-privacy-{label}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn fixture_office_registration_database(home: &Path) -> Connection {
        let root = home.join("Library/Group Containers/UBF8T346G9.Office/MicrosoftRegistrationDB");
        fs::create_dir_all(&root).unwrap();
        let connection =
            Connection::open(root.join("MicrosoftRegistrationDB_fixture.reg")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE HKEY_CURRENT_USER (
                     node_id INTEGER PRIMARY KEY,
                     parent_id INTEGER,
                     name TEXT NOT NULL,
                     write_time BLOB
                 );
                 CREATE TABLE HKEY_CURRENT_USER_values (
                     node_id INTEGER NOT NULL,
                     name TEXT NOT NULL,
                     type INTEGER NOT NULL,
                     value BLOB
                 );",
            )
            .unwrap();
        connection
    }

    fn insert_office_recent_record(
        connection: &Connection,
        application: &str,
        record_id: &str,
        file_name: &str,
        timestamp: &str,
        pinned: bool,
    ) {
        let mut parent_id = -1_i64;
        for component in [
            "Software",
            "Microsoft",
            "Office",
            "15.0",
            "Common",
            "MruUserData",
            "UnsignedUser",
            application,
            "Local",
            "Documents",
        ] {
            let existing = connection
                .query_row(
                    "SELECT node_id FROM HKEY_CURRENT_USER WHERE parent_id = ?1 AND name = ?2",
                    params![parent_id, component],
                    |row| row.get::<_, i64>(0),
                )
                .ok();
            parent_id = match existing {
                Some(node_id) => node_id,
                None => {
                    connection
                        .execute(
                            "INSERT INTO HKEY_CURRENT_USER(parent_id, name) VALUES (?1, ?2)",
                            params![parent_id, component],
                        )
                        .unwrap();
                    connection.last_insert_rowid()
                }
            };
        }
        connection
            .execute(
                "INSERT INTO HKEY_CURRENT_USER(parent_id, name) VALUES (?1, ?2)",
                params![parent_id, record_id],
            )
            .unwrap();
        let node_id = connection.last_insert_rowid();
        for (name, value_type, value) in [
            ("FileName", 1_i64, file_name.as_bytes()),
            ("Timestamp", 1_i64, timestamp.as_bytes()),
            (
                "IsPinned",
                4_i64,
                if pinned {
                    b"1".as_slice()
                } else {
                    b"0".as_slice()
                },
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO HKEY_CURRENT_USER_values(node_id, name, type, value)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![node_id, name, value_type, value],
                )
                .unwrap();
        }
        let document_url = format!("file:///Users/test/{file_name}");
        connection
            .execute(
                "INSERT INTO HKEY_CURRENT_USER_values(node_id, name, type, value)
                 VALUES (?1, 'DocumentUrl', 1, ?2)",
                params![node_id, document_url.as_bytes()],
            )
            .unwrap();
    }

    #[test]
    fn microsoft_edge_profiles_use_the_chromium_privacy_contract() {
        let home = fixture_home("edge");
        let edge_root = home.join("Library/Application Support/Microsoft Edge");
        for profile in ["Default", "Profile 1"] {
            let root = edge_root.join(profile);
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("History"), b"fixture").unwrap();
            fs::write(root.join("Cookies"), b"fixture").unwrap();
            fs::write(root.join("Top Sites"), b"fixture").unwrap();
            fs::write(root.join("Shortcuts"), b"fixture").unwrap();
            fs::write(root.join("Favicons"), b"fixture").unwrap();
            fs::create_dir(root.join("Cache")).unwrap();
        }

        let edge = chromium_browser(
            "edge",
            "Microsoft Edge",
            &edge_root,
            None,
            vec!["Microsoft Edge".into(), "Microsoft Edge Helper".into()],
            &PlatformCancellation::new(|| false),
        )
        .unwrap();

        assert_eq!(edge.provider_key, "edge");
        assert_eq!(edge.display_name, "Microsoft Edge");
        assert_eq!(edge.kind, PlatformPrivacyBrowserKind::Chromium);
        assert_eq!(edge.profiles.len(), 2);
        assert!(edge
            .profiles
            .iter()
            .all(|profile| profile.history_database.is_some()
                && profile.cookie_database.is_some()
                && profile.top_sites_database.is_some()
                && profile.shortcut_database.is_some()
                && profile.favicon_database.is_some()
                && !profile.cache_directories.is_empty()));

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn application_access_denial_requests_permission_instead_of_reporting_unsupported() {
        let error = PlatformError::new(PlatformErrorCode::AccessDenied, "fixture");

        assert_eq!(
            application_trace_availability_for_error(&error),
            PlatformPrivacyApplicationTraceAvailability::PermissionRequired
        );
    }

    #[test]
    fn office_recent_trace_skips_protected_database_when_permission_is_unavailable() {
        let home = fixture_home("office-permission-gate");

        let trace = native_macos_office_application_trace(
            &home,
            "word",
            "Word",
            PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftWordRecentDocuments,
            false,
        );

        assert_eq!(
            trace.availability,
            PlatformPrivacyApplicationTraceAvailability::PermissionRequired
        );
        assert_eq!(trace.item_count, 0);
        assert_eq!(trace.revision, "permission-required");
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn application_sources_omit_protected_roots_without_full_disk_access() {
        let home = fixture_home("protected-application-roots");
        fs::create_dir_all(home.join("Applications/Pages.app")).unwrap();
        fs::create_dir_all(
            home.join("Library/Containers/com.apple.iwork.pages/Data/Library/Caches"),
        )
        .unwrap();

        let applications = application_privacy_sources(&home, false);
        let pages = applications
            .iter()
            .find(|application| application.provider_key == "pages")
            .unwrap();

        assert!(pages.traces.iter().all(|trace| {
            trace.roots.is_empty()
                && trace.availability
                    == PlatformPrivacyApplicationTraceAvailability::PermissionRequired
        }));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn installed_editor_exposes_independent_cache_log_session_history_and_recent_document_traces() {
        let home = fixture_home("application-privacy");
        fs::create_dir_all(home.join("Applications/Visual Studio Code.app")).unwrap();
        let support = home.join("Library/Application Support/Code");
        for relative in ["Cache", "logs", "Session Storage", "User/History"] {
            fs::create_dir_all(support.join(relative)).unwrap();
        }
        let history_resource = support.join("User/History/resource-one");
        fs::create_dir_all(&history_resource).unwrap();
        fs::write(
            history_resource.join("entries.json"),
            br#"{"version":1,"resource":"file:///fixture/document.txt","entries":[{"id":"one","timestamp":1},{"id":"two","timestamp":2}]}"#,
        )
        .unwrap();
        shared_file_list::tests::write_fixture(
            &home,
            "com.microsoft.vscode",
            &[("vscode-visible", 0, b"bookmark")],
        );

        let applications = application_privacy_sources(&home, true);
        let vscode = applications
            .iter()
            .find(|application| application.provider_key == "vscode")
            .expect("installed editor must be reported");

        assert!(vscode.application_path.is_some());
        assert_eq!(vscode.traces.len(), 5);
        assert!(vscode.traces.iter().all(|trace| {
            trace.availability == PlatformPrivacyApplicationTraceAvailability::Available
        }));
        let editor_history = vscode
            .traces
            .iter()
            .find(|trace| trace.kind == PlatformPrivacyApplicationTraceKind::EditorLocalHistory)
            .unwrap();
        assert_eq!(editor_history.item_count, 2);
        assert!(editor_history.roots.is_empty());
        assert_eq!(
            editor_history.native_kind,
            Some(PlatformPrivacyApplicationNativeTraceKind::MacVisualStudioCodeEditorHistory)
        );
        let recent = vscode
            .traces
            .iter()
            .find(|trace| trace.kind == PlatformPrivacyApplicationTraceKind::RecentDocuments)
            .unwrap();
        assert_eq!(recent.item_count, 1);
        assert!(recent.roots.is_empty());
        assert_eq!(
            recent.native_kind,
            Some(PlatformPrivacyApplicationNativeTraceKind::MacVisualStudioCodeRecentDocuments)
        );

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn shell_history_discovery_counts_records_without_reading_content_into_the_contract() {
        let home = fixture_home("shell-history");
        fs::write(home.join(".zsh_history"), b"first\nsecond\n").unwrap();
        fs::write(home.join(".python_history"), b"third\n").unwrap();

        let trace = shell_history_trace(&home).expect("history files should produce one source");

        assert_eq!(trace.kind, PlatformPrivacySystemTraceKind::ShellHistory);
        assert_eq!(trace.roots.len(), 2);
        assert_eq!(trace.item_count, 3);
        assert!(trace.all_time_only);
        assert!(trace.revision.is_empty());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn productivity_application_exposes_recent_documents_and_regenerable_cache() {
        let home = fixture_home("productivity-application");
        fs::create_dir_all(home.join("Applications/Microsoft Word.app")).unwrap();
        let office_database = fixture_office_registration_database(&home);
        insert_office_recent_record(
            &office_database,
            "Word",
            "record-one",
            "proposal.docx",
            "2026-09-01T12:00:00Z",
            false,
        );
        drop(office_database);
        let cache = home.join("Library/Caches/com.microsoft.word");
        fs::create_dir_all(&cache).unwrap();

        let applications = application_privacy_sources(&home, true);
        let word = applications
            .iter()
            .find(|application| application.provider_key == "word")
            .expect("installed productivity application must be reported");

        assert!(word.traces.iter().any(|trace| {
            trace.kind == PlatformPrivacyApplicationTraceKind::RecentDocuments
                && trace.native_kind
                    == Some(
                        PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftWordRecentDocuments,
                    )
                && trace.item_count == 1
        }));
        assert!(word.traces.iter().any(|trace| {
            trace.kind == PlatformPrivacyApplicationTraceKind::Cache
                && trace.roots.iter().any(|root| root == &cache)
        }));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn structured_application_recents_count_archive_items_instead_of_container_files() {
        let home = fixture_home("structured-application-recents");
        for bundle_name in [
            "Pages.app",
            "Numbers.app",
            "Keynote.app",
            "Visual Studio Code.app",
            "Xcode.app",
            "VLC.app",
            "Movist Pro.app",
            "QuickTime Player.app",
            "Preview.app",
            "PDF Expert.app",
            "TextEdit.app",
            "Skim.app",
        ] {
            fs::create_dir_all(home.join("Applications").join(bundle_name)).unwrap();
        }
        shared_file_list::tests::write_fixture(
            &home,
            "com.apple.iwork.pages",
            &[
                ("pages-visible-one", 0, b"bookmark-one"),
                ("pages-visible-two", 0, b"bookmark-two"),
                ("pages-hidden", 1, b"bookmark-hidden"),
            ],
        );
        shared_file_list::tests::write_fixture(
            &home,
            "com.apple.iwork.numbers",
            &[("numbers-visible", 0, b"bookmark-three")],
        );
        for (bundle_identifier, record_identifier) in [
            ("com.apple.iwork.keynote", "keynote-visible"),
            ("com.microsoft.vscode", "vscode-visible"),
            ("com.apple.dt.xcode", "xcode-visible"),
            ("org.videolan.vlc", "vlc-visible"),
            ("com.movist.movistpro", "movist-visible"),
            ("com.apple.quicktimeplayerx", "quicktime-visible"),
            ("com.apple.preview", "preview-visible"),
            ("com.readdle.pdfexpert-mac", "pdf-expert-visible"),
            ("com.apple.textedit", "textedit-visible"),
            ("net.sourceforge.skim-app.skim", "skim-visible"),
        ] {
            shared_file_list::tests::write_fixture(
                &home,
                bundle_identifier,
                &[(record_identifier, 0, b"bookmark")],
            );
        }

        let applications = application_privacy_sources(&home, true);
        for (provider_key, trace_kind, expected_count, expected_kind) in [
            (
                "pages",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                2,
                PlatformPrivacyApplicationNativeTraceKind::MacPagesRecentDocuments,
            ),
            (
                "numbers",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                1,
                PlatformPrivacyApplicationNativeTraceKind::MacNumbersRecentDocuments,
            ),
            (
                "keynote",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                1,
                PlatformPrivacyApplicationNativeTraceKind::MacKeynoteRecentDocuments,
            ),
            (
                "vscode",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                1,
                PlatformPrivacyApplicationNativeTraceKind::MacVisualStudioCodeRecentDocuments,
            ),
            (
                "xcode",
                PlatformPrivacyApplicationTraceKind::RecentProjects,
                1,
                PlatformPrivacyApplicationNativeTraceKind::MacXcodeRecentProjects,
            ),
            (
                "preview",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                1,
                PlatformPrivacyApplicationNativeTraceKind::MacPreviewRecentDocuments,
            ),
            (
                "pdf_expert",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                1,
                PlatformPrivacyApplicationNativeTraceKind::MacPdfExpertRecentDocuments,
            ),
            (
                "textedit",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                1,
                PlatformPrivacyApplicationNativeTraceKind::MacTextEditRecentDocuments,
            ),
            (
                "skim",
                PlatformPrivacyApplicationTraceKind::RecentDocuments,
                1,
                PlatformPrivacyApplicationNativeTraceKind::MacSkimRecentDocuments,
            ),
            (
                "vlc",
                PlatformPrivacyApplicationTraceKind::PlaybackHistory,
                1,
                PlatformPrivacyApplicationNativeTraceKind::MacVlcRecentMedia,
            ),
            (
                "movist",
                PlatformPrivacyApplicationTraceKind::PlaybackHistory,
                1,
                PlatformPrivacyApplicationNativeTraceKind::MacMovistRecentMedia,
            ),
            (
                "quicktime_player",
                PlatformPrivacyApplicationTraceKind::PlaybackHistory,
                1,
                PlatformPrivacyApplicationNativeTraceKind::MacQuickTimeRecentMedia,
            ),
        ] {
            let application = applications
                .iter()
                .find(|application| application.provider_key == provider_key)
                .unwrap();
            let recent = application
                .traces
                .iter()
                .find(|trace| trace.kind == trace_kind)
                .unwrap();
            assert_eq!(recent.item_count, expected_count);
            assert_eq!(recent.native_kind, Some(expected_kind));
            assert!(recent.roots.is_empty());
            assert_eq!(
                recent.availability,
                PlatformPrivacyApplicationTraceAvailability::Available
            );
        }
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn office_recent_documents_preserve_pinned_records_and_verify_cleanup() {
        let home = fixture_home("office-recent-records");
        let database = fixture_office_registration_database(&home);
        insert_office_recent_record(
            &database,
            "Word",
            "older-record",
            "older.docx",
            "2026-08-01T12:00:00Z",
            false,
        );
        insert_office_recent_record(
            &database,
            "Word",
            "newer-record",
            "newer.docx",
            "2026-09-01T12:00:00Z",
            false,
        );
        insert_office_recent_record(
            &database,
            "Word",
            "pinned-record",
            "pinned.docx",
            "2026-09-02T12:00:00Z",
            true,
        );
        drop(database);

        let (count, revision) = macos_office_recent_snapshot(&home, "Word").unwrap();
        let details = macos_office_recent_details(&home, "Word", 0, 10).unwrap();

        assert_eq!(count, 2);
        assert_eq!(details.len(), 2);
        assert_eq!(details[0].label, "/Users/test/newer.docx");
        assert_eq!(details[1].label, "/Users/test/older.docx");
        assert!(!revision.contains("newer.docx"));
        assert!(clear_macos_office_recent_documents(&home, "Word").unwrap());
        assert_eq!(macos_office_recent_snapshot(&home, "Word").unwrap().0, 0);

        let connection = open_macos_office_database(
            &macos_office_registration_databases(&home).unwrap()[0],
            true,
        )
        .unwrap();
        let remaining: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM HKEY_CURRENT_USER WHERE name = 'pinned-record'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 1);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn office_suite_sources_keep_recent_documents_scoped_to_each_application() {
        let home = fixture_home("office-suite-recent-records");
        for bundle_name in [
            "Microsoft Word.app",
            "Microsoft Excel.app",
            "Microsoft PowerPoint.app",
        ] {
            fs::create_dir_all(home.join("Applications").join(bundle_name)).unwrap();
        }
        let database = fixture_office_registration_database(&home);
        for (application, count) in [("Word", 2), ("Excel", 3), ("PowerPoint", 4)] {
            for index in 0..count {
                insert_office_recent_record(
                    &database,
                    application,
                    &format!("{application}-record-{index}"),
                    &format!("document-{index}"),
                    &format!("2026-09-01T12:00:{index:02}Z"),
                    false,
                );
            }
        }
        drop(database);

        let applications = application_privacy_sources(&home, true);
        for (provider_key, expected_count, expected_kind) in [
            (
                "word",
                2,
                PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftWordRecentDocuments,
            ),
            (
                "excel",
                3,
                PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftExcelRecentDocuments,
            ),
            (
                "powerpoint",
                4,
                PlatformPrivacyApplicationNativeTraceKind::MacMicrosoftPowerPointRecentDocuments,
            ),
        ] {
            let application = applications
                .iter()
                .find(|application| application.provider_key == provider_key)
                .unwrap();
            let recent = application
                .traces
                .iter()
                .find(|trace| trace.kind == PlatformPrivacyApplicationTraceKind::RecentDocuments)
                .unwrap();
            assert_eq!(recent.item_count, expected_count);
            assert_eq!(recent.native_kind, Some(expected_kind));
        }
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    #[ignore = "copies an explicitly selected real Office registration database"]
    fn actual_office_registration_copy_clears_without_mutating_the_source() {
        let source = std::env::var_os("MANGODISK_TEST_OFFICE_REGISTRATION_DB")
            .map(PathBuf::from)
            .expect("MANGODISK_TEST_OFFICE_REGISTRATION_DB must name an Office database");
        let source_before = blake3::hash(&fs::read(&source).unwrap());
        let home = fixture_home("actual-office-registration-copy");
        let destination_root =
            home.join("Library/Group Containers/UBF8T346G9.Office/MicrosoftRegistrationDB");
        fs::create_dir_all(&destination_root).unwrap();
        fs::copy(
            &source,
            destination_root.join("MicrosoftRegistrationDB_fixture.reg"),
        )
        .unwrap();

        let (before_count, _) = macos_office_recent_snapshot(&home, "Word").unwrap();
        assert!(before_count > 0);
        let details = macos_office_recent_details(&home, "Word", 0, before_count as u32).unwrap();
        assert_eq!(details.len() as u64, before_count);
        assert!(details
            .iter()
            .any(|entry| Path::new(&entry.label).is_absolute()));
        assert!(clear_macos_office_recent_documents(&home, "Word").unwrap());
        assert_eq!(macos_office_recent_snapshot(&home, "Word").unwrap().0, 0);
        assert_eq!(source_before, blake3::hash(&fs::read(&source).unwrap()));

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    #[ignore = "reads the installed Movist recent-media list without modifying it"]
    fn actual_movist_exposes_logical_playback_records() {
        let home = dirs::home_dir().expect("home directory must be available");
        let applications = application_privacy_sources(&home, true);
        let movist = applications
            .iter()
            .find(|application| application.provider_key == "movist")
            .expect("installed Movist source must be reported");
        let trace = movist
            .traces
            .iter()
            .find(|trace| trace.kind == PlatformPrivacyApplicationTraceKind::PlaybackHistory)
            .expect("Movist must expose playback history");
        assert!(trace.item_count > 0);
        let details = application_details(
            trace
                .native_kind
                .expect("Movist history must be structured"),
            0,
            trace.item_count as u32,
        )
        .unwrap();
        assert_eq!(details.len() as u64, trace.item_count);
        assert!(details.iter().all(|entry| !entry.label.trim().is_empty()));
        println!("validated Movist history item_count={}", trace.item_count);
    }

    #[test]
    #[ignore = "copies real Apple application recent-item lists without modifying them"]
    fn actual_apple_application_recents_expose_logical_items_and_clear_copies() {
        let source_home = dirs::home_dir().expect("home directory must be available");
        let fixture = fixture_home("actual-apple-application-recents");
        let source_root = source_home.join(
            "Library/Application Support/com.apple.sharedfilelist/com.apple.LSSharedFileList.ApplicationRecentDocuments",
        );
        let fixture_root = fixture.join(
            "Library/Application Support/com.apple.sharedfilelist/com.apple.LSSharedFileList.ApplicationRecentDocuments",
        );
        fs::create_dir_all(&fixture_root).unwrap();

        for (application, bundle_identifier) in [
            ("Preview", "com.apple.preview"),
            ("TextEdit", "com.apple.textedit"),
            ("QuickTime Player", "com.apple.quicktimeplayerx"),
        ] {
            let mut source_hashes = Vec::new();
            for extension in ["sfl3", "sfl2"] {
                let file_name = format!("{bundle_identifier}.{extension}");
                let source = source_root.join(&file_name);
                let metadata = match fs::symlink_metadata(&source) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => panic!("failed to inspect {application} recent items: {error}"),
                };
                assert!(metadata.is_file() && !metadata.file_type().is_symlink());
                let source_bytes = fs::read(&source).unwrap();
                source_hashes.push((source, blake3::hash(&source_bytes)));
                fs::copy(
                    &source_hashes.last().unwrap().0,
                    fixture_root.join(file_name),
                )
                .unwrap();
            }
            assert!(
                !source_hashes.is_empty(),
                "{application} must have a recent-item list before this manual test"
            );

            let snapshot = shared_file_list::snapshot(&fixture, bundle_identifier).unwrap();
            assert!(
                snapshot.item_count > 0,
                "{application} history must not be empty"
            );
            let details = shared_file_list::details(
                &fixture,
                bundle_identifier,
                0,
                snapshot.item_count as u32,
            )
            .unwrap();
            assert_eq!(details.len() as u64, snapshot.item_count);
            assert!(details.iter().all(|entry| {
                let label = entry.label.trim();
                !label.is_empty()
                    && !label.ends_with(".sfl2")
                    && !label.ends_with(".sfl3")
                    && !label.eq_ignore_ascii_case(bundle_identifier)
            }));

            assert!(shared_file_list::clear(&fixture, bundle_identifier).unwrap());
            assert_eq!(
                shared_file_list::snapshot(&fixture, bundle_identifier)
                    .unwrap()
                    .item_count,
                0
            );
            for (source, source_hash) in source_hashes {
                assert_eq!(source_hash, blake3::hash(&fs::read(source).unwrap()));
            }
            println!(
                "validated {application} recent items item_count={}",
                snapshot.item_count
            );
        }

        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn application_sources_exclude_communication_apps_and_keep_office_cache() {
        let home = fixture_home("application-source-scope");
        for application in [
            "WPS Office.app",
            "Cursor.app",
            "Windsurf.app",
            "Slack.app",
            "Discord.app",
            "Postman.app",
            "Claude.app",
            "Microsoft Teams.app",
            "DingTalk.app",
            "TencentMeeting.app",
            "Thunderbird.app",
            "zoom.us.app",
            "Microsoft Outlook.app",
            "OneDrive.app",
        ] {
            fs::create_dir_all(home.join("Applications").join(application)).unwrap();
        }
        fs::create_dir_all(home.join(".claude/cache")).unwrap();
        let wps_cache = home.join("Library/Caches/com.kingsoft.wpsoffice.mac");
        fs::create_dir_all(&wps_cache).unwrap();

        let applications = application_privacy_sources(&home, true);
        let wps = applications
            .iter()
            .find(|application| application.provider_key == "wps_office")
            .expect("WPS Office must expose its dedicated cache");
        assert!(wps
            .traces
            .iter()
            .any(|trace| trace.roots.contains(&wps_cache)));
        assert!(wps.traces.iter().any(|trace| {
            trace.kind == PlatformPrivacyApplicationTraceKind::RecentDocuments
                && trace.native_kind
                    == Some(PlatformPrivacyApplicationNativeTraceKind::MacWpsOfficeRecentDocuments)
        }));
        for excluded_provider in [
            "cursor",
            "windsurf",
            "slack",
            "discord",
            "postman",
            "claude",
            "microsoft_teams",
            "dingtalk",
            "tencent_meeting",
            "thunderbird",
            "claude_code",
            "zoom",
            "outlook",
            "onedrive",
        ] {
            assert!(
                applications
                    .iter()
                    .all(|application| application.provider_key != excluded_provider),
                "{excluded_provider} is outside the supported application activity scope"
            );
        }
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn fixed_system_history_groups_only_existing_allowlisted_files() {
        let home = fixture_home("fixed-system-history");
        let existing = home.join("RecentApplications.sfl3");
        fs::write(&existing, b"fixture").unwrap();

        let trace = fixed_file_system_trace(
            "recent_applications",
            "Recent applications",
            PlatformPrivacySystemTraceKind::RecentApplications,
            [existing.clone(), home.join("missing.sfl3")],
        );

        assert_eq!(trace.roots, vec![existing]);
        assert_eq!(trace.item_count, 1);
        assert!(trace.available);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn native_system_recent_applications_count_list_entries_not_archive_files() {
        let home = fixture_home("system-recent-applications");
        shared_file_list::tests::write_system_fixture(
            &home,
            RECENT_APPLICATIONS_LIST,
            &[
                ("application-one", 0, b"bookmark-one"),
                ("application-two", 0, b"bookmark-two"),
                ("hidden-application", 1, b"bookmark-hidden"),
            ],
        );

        let (trace, permission_denied) = native_macos_system_shared_file_list_trace(
            &home,
            "recent_applications",
            "Recent applications",
            PlatformPrivacySystemTraceKind::RecentApplications,
            RECENT_APPLICATIONS_LIST,
        );

        assert_eq!(trace.item_count, 2);
        assert!(trace.roots.is_empty());
        assert!(trace.available);
        assert!(!permission_denied);
        assert!(!trace.revision.is_empty());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn native_system_network_history_combines_hosts_and_servers() {
        let home = fixture_home("system-network-history");
        shared_file_list::tests::write_system_fixture(
            &home,
            RECENT_NETWORK_LISTS[0],
            &[("host-one", 0, b"bookmark-host")],
        );
        shared_file_list::tests::write_system_fixture(
            &home,
            RECENT_NETWORK_LISTS[1],
            &[
                ("server-one", 0, b"bookmark-server-one"),
                ("server-two", 0, b"bookmark-server-two"),
            ],
        );

        let trace = native_macos_system_shared_file_list_group_trace(
            &home,
            "recent_network_connections",
            "Recent servers and hosts",
            PlatformPrivacySystemTraceKind::NetworkConnectionHistory,
            &RECENT_NETWORK_LISTS,
        );

        assert_eq!(trace.item_count, 3);
        assert!(trace.roots.is_empty());
        assert!(trace.available);
        assert!(!trace.revision.is_empty());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn brave_origin_privacy_profiles_are_independent_from_regular_brave() {
        let home = fixture_home("brave-origin-privacy");
        let cancellation = PlatformCancellation::new(|| false);
        let missing = brave_browsers(&home, &cancellation).unwrap();
        assert!(missing.iter().all(|browser| browser.profiles.is_empty()));
        for product in ["Brave-Browser", "Brave-Origin"] {
            let root = home
                .join("Library/Application Support")
                .join("BraveSoftware")
                .join(product);
            for name in ["Default", "Profile 1", "Other Profile"] {
                let profile = root.join(name);
                fs::create_dir_all(profile.join("Network")).unwrap();
                for file in [
                    "History",
                    "Network/Cookies",
                    "Login Data",
                    "Web Data",
                    "Bookmarks",
                ] {
                    fs::write(profile.join(file), b"fixture").unwrap();
                }
                for cache in ["Cache", "DawnGraphiteCache", "DawnWebGPUCache"] {
                    fs::create_dir(profile.join(cache)).unwrap();
                }
                fs::create_dir(profile.join("Sessions")).unwrap();
                fs::create_dir(profile.join("Local Storage")).unwrap();
            }
        }
        let browsers = brave_browsers(&home, &cancellation).unwrap();
        assert_eq!(browsers[0].provider_key, "brave");
        assert_eq!(browsers[0].display_name, "Brave");
        assert_eq!(browsers[1].provider_key, "brave-origin");
        assert_eq!(browsers[1].display_name, "Brave Origin");
        assert_eq!(
            browsers[1].process_names,
            vec!["Brave Origin", "Brave Origin Helper"]
        );
        for (browser, product) in browsers.iter().zip(["Brave-Browser", "Brave-Origin"]) {
            assert_eq!(browser.kind, PlatformPrivacyBrowserKind::Chromium);
            assert_eq!(browser.profiles.len(), 2);
            let root = home
                .join("Library/Application Support")
                .join("BraveSoftware")
                .join(product);
            for profile in &browser.profiles {
                assert!(profile.root.starts_with(&root));
                assert!(profile
                    .provider_key
                    .starts_with(&format!("{}:", browser.provider_key)));
                for database in [
                    &profile.history_database,
                    &profile.cookie_database,
                    &profile.saved_password_source,
                    &profile.autofill_database,
                ] {
                    assert!(database.as_ref().unwrap().starts_with(&profile.root));
                }
                assert_eq!(
                    profile.cache_directories,
                    vec![
                        profile.root.join("Cache"),
                        profile.root.join("DawnGraphiteCache"),
                        profile.root.join("DawnWebGPUCache")
                    ]
                );
                assert_eq!(
                    profile.session_directories,
                    vec![profile.root.join("Sessions")]
                );
                assert_eq!(
                    profile.site_storage_directories,
                    vec![profile.root.join("Local Storage")]
                );
                assert_eq!(
                    fs::read(profile.root.join("Bookmarks")).unwrap(),
                    b"fixture"
                );
            }
        }
        assert_eq!(
            brave_browsers(&home, &PlatformCancellation::new(|| true))
                .unwrap_err()
                .code(),
            PlatformErrorCode::UserCancelled
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn additional_chromium_browsers_keep_independent_source_identity() {
        let home = fixture_home("additional-chromium-browsers");
        let definitions = [
            ("brave", "Brave", "BraveSoftware/Brave-Browser"),
            ("opera", "Opera", "com.operasoftware.Opera"),
            ("360_safe_browser", "360 Safe Browser", "360Chrome"),
            ("qq_browser", "QQ Browser", "QQBrowser3"),
        ];

        for (provider_key, display_name, relative_root) in definitions {
            let root = home.join(relative_root);
            let profile_root = root.join("Default");
            fs::create_dir_all(&profile_root).unwrap();
            fs::write(profile_root.join("History"), b"fixture").unwrap();
            fs::write(profile_root.join("Cookies"), b"fixture").unwrap();

            let browser = chromium_browser(
                provider_key,
                display_name,
                &root,
                None,
                vec![format!("{display_name} process")],
                &PlatformCancellation::new(|| false),
            )
            .unwrap();

            assert_eq!(browser.provider_key, provider_key);
            assert_eq!(browser.display_name, display_name);
            assert_eq!(browser.kind, PlatformPrivacyBrowserKind::Chromium);
            assert_eq!(browser.profiles.len(), 1);
            assert!(browser.profiles[0].provider_key.starts_with(provider_key));
            assert!(browser.profiles[0].history_database.is_some());
            assert!(browser.profiles[0].cookie_database.is_some());
        }

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn root_level_opera_profile_is_discovered_without_duplicate_default() {
        let home = fixture_home("opera-root-profile");
        fs::write(home.join("History"), b"fixture").unwrap();
        fs::write(home.join("Cookies"), b"fixture").unwrap();

        let opera = chromium_browser(
            "opera",
            "Opera",
            &home,
            None,
            vec!["Opera".into()],
            &PlatformCancellation::new(|| false),
        )
        .unwrap();

        assert_eq!(opera.profiles.len(), 1);
        assert_eq!(opera.profiles[0].provider_key, "opera:Default");
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    #[ignore = "reads the installed Microsoft Edge profile metadata"]
    fn actual_microsoft_edge_profiles_are_discovered() {
        let home = dirs::home_dir().expect("home directory must be available");
        let edge = chromium_browser(
            "edge",
            "Microsoft Edge",
            &home.join("Library/Application Support/Microsoft Edge"),
            macos_application_path(&home, "Microsoft Edge.app"),
            vec!["Microsoft Edge".into(), "Microsoft Edge Helper".into()],
            &PlatformCancellation::new(|| false),
        )
        .unwrap();

        println!("edge_profile_count={}", edge.profiles.len());
        assert!(edge.application_path.is_some());
        assert!(!edge.profiles.is_empty());
        assert!(edge
            .profiles
            .iter()
            .any(|profile| profile.history_database.is_some()));
    }

    #[test]
    #[ignore = "reads installed Chromium browser profile metadata"]
    fn actual_additional_chromium_browsers_are_discovered() {
        let home = dirs::home_dir().expect("home directory must be available");
        let definitions = [
            (
                "brave",
                "Brave",
                "BraveSoftware/Brave-Browser",
                "Brave Browser.app",
                "Brave Browser",
            ),
            (
                "opera",
                "Opera",
                "com.operasoftware.Opera",
                "Opera.app",
                "Opera",
            ),
            (
                "360_safe_browser",
                "360 Safe Browser",
                "360Chrome",
                "360Chrome.app",
                "360Chrome",
            ),
            (
                "qq_browser",
                "QQ Browser",
                "QQBrowser3",
                "QQBrowser.app",
                "QQBrowser",
            ),
        ];

        for (provider_key, display_name, relative_root, bundle_name, process_name) in definitions {
            let browser = chromium_browser(
                provider_key,
                display_name,
                &home.join("Library/Application Support").join(relative_root),
                macos_application_path(&home, bundle_name),
                vec![process_name.into()],
                &PlatformCancellation::new(|| false),
            )
            .unwrap();

            println!(
                "browser_source={provider_key} profile_count={}",
                browser.profiles.len()
            );
            assert!(browser.application_path.is_some());
            assert!(!browser.profiles.is_empty());
            assert!(browser
                .profiles
                .iter()
                .any(|profile| profile.history_database.is_some()));
        }
    }
}
