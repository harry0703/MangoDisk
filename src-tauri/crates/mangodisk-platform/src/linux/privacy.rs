use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    PlatformCancellation, PlatformError, PlatformPrivacyApplicationNativeTraceKind,
    PlatformPrivacyBrowser, PlatformPrivacyBrowserKind, PlatformPrivacyDiscovery,
    PlatformPrivacyProfile, PlatformPrivacySystemTrace, PlatformPrivacySystemTraceKind,
    PlatformResult,
};

pub(crate) fn discover(
    cancellation: &PlatformCancellation,
) -> PlatformResult<PlatformPrivacyDiscovery> {
    if cancellation.is_cancelled() {
        return Err(PlatformError::operation_failed(
            "privacy discovery was cancelled",
        ));
    }
    let home = dirs::home_dir()
        .ok_or_else(|| PlatformError::operation_failed("unable to determine home directory"))?;
    let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
    let cache = dirs::cache_dir().unwrap_or_else(|| home.join(".cache"));
    let data = dirs::data_local_dir().unwrap_or_else(|| home.join(".local/share"));

    let mut browsers = Vec::new();

    // Chromium-based browsers
    let chromium_browsers: Vec<(&str, &str, &str, Vec<&str>)> = vec![
        (
            "chrome",
            "Google Chrome",
            "google-chrome",
            vec!["google-chrome", "chrome"],
        ),
        (
            "edge",
            "Microsoft Edge",
            "microsoft-edge",
            vec!["microsoft-edge", "microsoft-edge-stable"],
        ),
        (
            "brave",
            "Brave",
            "BraveSoftware/Brave-Browser",
            vec!["brave-browser", "brave"],
        ),
        ("opera", "Opera", "com.operasoftware.Opera", vec!["opera"]),
        ("vivaldi", "Vivaldi", "Vivaldi", vec!["vivaldi"]),
        (
            "chromium",
            "Chromium",
            "chromium",
            vec!["chromium", "chromium-browser"],
        ),
    ];

    for (key, display, root_rel, processes) in chromium_browsers {
        if cancellation.is_cancelled() {
            return Err(PlatformError::operation_failed(
                "privacy discovery was cancelled",
            ));
        }
        let root = config.join(root_rel);
        if root.exists() {
            let profiles = discover_chromium_profiles(key, &root, &cache.join(root_rel));
            browsers.push(PlatformPrivacyBrowser {
                provider_key: key.to_string(),
                display_name: display.to_string(),
                application_path: None,
                kind: PlatformPrivacyBrowserKind::Chromium,
                process_names: processes.into_iter().map(String::from).collect(),
                profiles,
            });
        }
    }

    // Firefox
    if cancellation.is_cancelled() {
        return Err(PlatformError::operation_failed(
            "privacy discovery was cancelled",
        ));
    }
    let firefox_root = home.join(".mozilla/firefox");
    if firefox_root.exists() {
        let profiles = discover_firefox_profiles(&firefox_root, &cache.join("mozilla/firefox"));
        browsers.push(PlatformPrivacyBrowser {
            provider_key: "firefox".to_string(),
            display_name: "Firefox".to_string(),
            application_path: None,
            kind: PlatformPrivacyBrowserKind::Firefox,
            process_names: vec!["firefox".to_string()],
            profiles,
        });
    }

    // System traces
    let mut system_traces = Vec::new();

    // Shell history
    let shell_history_files = [
        ".bash_history",
        ".zsh_history",
        ".local/share/fish/fish_history",
    ];
    let has_shell_history = shell_history_files.iter().any(|f| home.join(f).exists());
    if has_shell_history {
        let roots: Vec<PathBuf> = shell_history_files
            .iter()
            .filter(|f| home.join(f).exists())
            .map(|f| home.join(f))
            .collect();
        system_traces.push(PlatformPrivacySystemTrace {
            provider_key: "shell_history".to_string(),
            display_name: "Terminal and command history".to_string(),
            kind: PlatformPrivacySystemTraceKind::ShellHistory,
            roots,
            all_time_only: false,
            available: true,
            item_count: 0,
            revision: String::new(),
        });
    }

    // Recent documents (XDG recently-used.xbel)
    let recent_xbel = data.join("recently-used.xbel");
    if recent_xbel.exists() {
        system_traces.push(PlatformPrivacySystemTrace {
            provider_key: "recent_documents".to_string(),
            display_name: "Recent documents".to_string(),
            kind: PlatformPrivacySystemTraceKind::RecentDocumentHistory,
            roots: vec![recent_xbel],
            all_time_only: true,
            available: true,
            item_count: 0,
            revision: String::new(),
        });
    }

    Ok(PlatformPrivacyDiscovery {
        browsers,
        applications: Vec::new(),
        system_traces,
    })
}

pub(crate) fn clear(trace: PlatformPrivacySystemTraceKind) -> PlatformResult<bool> {
    match trace {
        PlatformPrivacySystemTraceKind::ShellHistory => {
            let home = dirs::home_dir().ok_or_else(|| {
                PlatformError::operation_failed("unable to determine home directory")
            })?;
            let files = [".bash_history", ".zsh_history"];
            for file in &files {
                let path = home.join(file);
                if path.exists() {
                    fs::remove_file(&path)
                        .map_err(|error| PlatformError::io("remove shell history", &error))?;
                }
            }
            let fish_history = home.join(".local/share/fish/fish_history");
            if fish_history.exists() {
                fs::remove_file(&fish_history)
                    .map_err(|error| PlatformError::io("remove fish history", &error))?;
            }
            Ok(true)
        }
        PlatformPrivacySystemTraceKind::RecentDocumentHistory => {
            let home = dirs::home_dir().ok_or_else(|| {
                PlatformError::operation_failed("unable to determine home directory")
            })?;
            let path = home.join(".local/share/recently-used.xbel");
            if path.exists() {
                fs::remove_file(&path)
                    .map_err(|error| PlatformError::io("remove recent documents", &error))?;
                Ok(true)
            } else {
                Ok(false)
            }
        }
        _ => Err(PlatformError::new(
            crate::PlatformErrorCode::Unsupported,
            "file-backed privacy traces are cleared through the Core safety boundary",
        )),
    }
}

pub(crate) fn clear_application_trace(
    _trace: PlatformPrivacyApplicationNativeTraceKind,
) -> PlatformResult<bool> {
    Err(PlatformError::new(
        crate::PlatformErrorCode::Unsupported,
        "the native application privacy trace is unavailable on Linux",
    ))
}

fn discover_chromium_profiles(
    browser_key: &str,
    root: &Path,
    cache_root: &Path,
) -> Vec<PlatformPrivacyProfile> {
    let mut profiles = Vec::new();

    // Check if root itself is a profile (Opera compatibility)
    if chromium_profile_has_supported_sources(root) {
        profiles.push(chromium_profile(browser_key, "Default", root, cache_root));
        return profiles;
    }

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == "Default" || name_str.starts_with("Profile ") {
                profiles.push(chromium_profile(
                    browser_key,
                    &name_str,
                    &entry.path(),
                    cache_root,
                ));
            }
        }
    }

    profiles
}

fn chromium_profile_has_supported_sources(root: &Path) -> bool {
    let sources = [
        "History",
        "Cookies",
        "Network/Cookies",
        "Login Data",
        "Web Data",
        "Sessions",
        "Local Storage",
        "IndexedDB",
        "Service Worker",
        "Cache",
        "Code Cache",
    ];
    sources.iter().any(|s| root.join(s).exists())
}

fn chromium_profile(
    browser_key: &str,
    profile_name: &str,
    profile_root: &Path,
    cache_root: &Path,
) -> PlatformPrivacyProfile {
    let history_database = profile_root.join("History");
    let cookie_database = if profile_root.join("Network/Cookies").exists() {
        Some(profile_root.join("Network/Cookies"))
    } else {
        let p = profile_root.join("Cookies");
        p.exists().then_some(p)
    };

    let cache_dirs = vec![
        profile_root.join("Cache"),
        profile_root.join("Code Cache"),
        profile_root.join("GPUCache"),
        profile_root.join("DawnCache"),
        profile_root.join("GrShaderCache"),
        cache_root.join(profile_name).join("Cache"),
        cache_root.join(profile_name).join("Code Cache"),
    ];

    let site_storage_dirs = ["Local Storage", "IndexedDB", "Service Worker"]
        .iter()
        .map(|d| profile_root.join(d))
        .filter(|p| p.exists())
        .collect();

    PlatformPrivacyProfile {
        provider_key: format!("{browser_key}:{profile_name}"),
        display_name: profile_name.to_string(),
        root: profile_root.to_path_buf(),
        history_database: history_database.exists().then_some(history_database),
        cookie_database,
        saved_password_source: {
            let p = profile_root.join("Login Data");
            p.exists().then_some(p)
        },
        autofill_database: {
            let p = profile_root.join("Web Data");
            p.exists().then_some(p)
        },
        permission_database: None,
        top_sites_database: {
            let p = profile_root.join("Top Sites");
            p.exists().then_some(p)
        },
        shortcut_database: {
            let p = profile_root.join("Shortcuts");
            p.exists().then_some(p)
        },
        favicon_database: {
            let p = profile_root.join("Favicons");
            p.exists().then_some(p)
        },
        session_directories: {
            let p = profile_root.join("Sessions");
            p.exists().then_some(p).into_iter().collect()
        },
        site_storage_directories: site_storage_dirs,
        cache_directories: cache_dirs.into_iter().filter(|p| p.exists()).collect(),
    }
}

fn discover_firefox_profiles(
    profiles_root: &Path,
    cache_root: &Path,
) -> Vec<PlatformPrivacyProfile> {
    let mut profiles = Vec::new();
    let display_names = firefox_display_names(profiles_root);

    if let Ok(entries) = fs::read_dir(profiles_root) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let display_name = display_names
                .get(&dir_name)
                .cloned()
                .unwrap_or_else(|| dir_name.clone());

            let profile_root = entry.path();
            let history_database = profile_root.join("places.sqlite");
            let cookie_database = profile_root.join("cookies.sqlite");

            let cache_dirs = vec![
                cache_root.join(&dir_name).join("cache2"),
                cache_root.join(&dir_name).join("startupCache"),
                cache_root.join(&dir_name).join("thumbnails"),
            ];

            profiles.push(PlatformPrivacyProfile {
                provider_key: format!("firefox:{dir_name}"),
                display_name,
                root: profile_root.clone(),
                history_database: history_database.exists().then_some(history_database),
                cookie_database: cookie_database.exists().then_some(cookie_database),
                saved_password_source: {
                    let p = profile_root.join("logins.json");
                    p.exists().then_some(p)
                },
                autofill_database: {
                    let p = profile_root.join("formhistory.sqlite");
                    p.exists().then_some(p)
                },
                permission_database: {
                    let p = profile_root.join("permissions.sqlite");
                    p.exists().then_some(p)
                },
                top_sites_database: None,
                shortcut_database: None,
                favicon_database: None,
                session_directories: {
                    let p = profile_root.join("sessionstore-backups");
                    p.exists().then_some(p).into_iter().collect()
                },
                site_storage_directories: {
                    let p = profile_root.join("storage/default");
                    p.exists().then_some(p).into_iter().collect()
                },
                cache_directories: cache_dirs.into_iter().filter(|p| p.exists()).collect(),
            });
        }
    }

    profiles
}

fn firefox_display_names(profiles_root: &Path) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    let ini_path = profiles_root.join("profiles.ini");
    let Ok(content) = fs::read_to_string(&ini_path) else {
        return map;
    };
    let mut current_name: Option<String> = None;
    let mut current_path: Option<String> = None;
    for line in content.lines() {
        if let Some(name) = line.strip_prefix("Name=") {
            current_name = Some(name.to_string());
        } else if let Some(path) = line.strip_prefix("Path=") {
            current_path = Some(path.to_string());
        } else if line.starts_with('[') {
            if let (Some(name), Some(path)) = (current_name.take(), current_path.take()) {
                let dir_name = path.split('/').next().unwrap_or(&path).to_string();
                map.insert(dir_name, name);
            }
        }
    }
    if let (Some(name), Some(path)) = (current_name, current_path) {
        let dir_name = path.split('/').next().unwrap_or(&path).to_string();
        map.insert(dir_name, name);
    }
    map
}
