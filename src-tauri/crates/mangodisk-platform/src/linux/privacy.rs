use std::{
    fs,
    io::{BufRead, BufReader, Read},
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
    let started = std::time::Instant::now();
    ensure_not_cancelled(cancellation)?;
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
        ("opera", "Opera", "opera", vec!["opera"]),
        ("vivaldi", "Vivaldi", "vivaldi", vec!["vivaldi"]),
        (
            "chromium",
            "Chromium",
            "chromium",
            vec!["chromium", "chromium-browser"],
        ),
    ];

    for (key, display, root_rel, processes) in chromium_browsers {
        ensure_not_cancelled(cancellation)?;
        push_chromium_browser(
            &mut browsers,
            key,
            display,
            config.join(root_rel),
            cache.join(root_rel),
            &processes,
        );
    }

    let sandboxed_chromium = [
        (
            "chrome-flatpak",
            "Google Chrome (Flatpak)",
            home.join(".var/app/com.google.Chrome/config/google-chrome"),
            home.join(".var/app/com.google.Chrome/cache/google-chrome"),
            &["google-chrome", "chrome"][..],
        ),
        (
            "brave-flatpak",
            "Brave (Flatpak)",
            home.join(".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser"),
            home.join(".var/app/com.brave.Browser/cache/BraveSoftware/Brave-Browser"),
            &["brave-browser", "brave"][..],
        ),
        (
            "chromium-flatpak",
            "Chromium (Flatpak)",
            home.join(".var/app/org.chromium.Chromium/config/chromium"),
            home.join(".var/app/org.chromium.Chromium/cache/chromium"),
            &["chromium", "chromium-browser"][..],
        ),
        (
            "chromium-snap",
            "Chromium (Snap)",
            home.join("snap/chromium/common/chromium"),
            home.join("snap/chromium/common/.cache/chromium"),
            &["chromium", "chromium-browser"][..],
        ),
    ];
    for (key, display, root, cache_root, processes) in sandboxed_chromium {
        ensure_not_cancelled(cancellation)?;
        push_chromium_browser(&mut browsers, key, display, root, cache_root, processes);
    }

    // Firefox
    ensure_not_cancelled(cancellation)?;
    let firefox_locations = [
        (
            "firefox",
            "Firefox",
            home.join(".mozilla/firefox"),
            cache.join("mozilla/firefox"),
        ),
        (
            "firefox-snap",
            "Firefox (Snap)",
            home.join("snap/firefox/common/.mozilla/firefox"),
            home.join("snap/firefox/common/.cache/mozilla/firefox"),
        ),
        (
            "firefox-flatpak",
            "Firefox (Flatpak)",
            home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
            home.join(".var/app/org.mozilla.firefox/cache/mozilla/firefox"),
        ),
    ];
    for (key, display, root, cache_root) in firefox_locations {
        ensure_not_cancelled(cancellation)?;
        push_firefox_browser(&mut browsers, key, display, root, cache_root);
    }

    // System traces
    let mut system_traces = Vec::new();

    // Shell history
    let shell_history_files = [
        ".bash_history",
        ".zsh_history",
        ".local/share/fish/fish_history",
    ];
    let roots: Vec<PathBuf> = shell_history_files
        .iter()
        .map(|path| home.join(path))
        .filter(|path| regular_file_without_links(path))
        .collect();
    if !roots.is_empty() {
        let item_count = roots.iter().map(|path| count_text_records(path)).sum();
        system_traces.push(PlatformPrivacySystemTrace {
            provider_key: "shell_history".to_string(),
            display_name: "Terminal and command history".to_string(),
            kind: PlatformPrivacySystemTraceKind::ShellHistory,
            roots,
            all_time_only: true,
            available: true,
            item_count,
            revision: String::new(),
        });
    }

    // Recent documents (XDG recently-used.xbel)
    let recent_xbel = data.join("recently-used.xbel");
    if regular_file_without_links(&recent_xbel) {
        let item_count = count_xbel_bookmarks(&recent_xbel);
        system_traces.push(PlatformPrivacySystemTrace {
            provider_key: "recent_documents".to_string(),
            display_name: "Recent documents".to_string(),
            kind: PlatformPrivacySystemTraceKind::RecentDocumentHistory,
            roots: vec![recent_xbel],
            all_time_only: true,
            available: true,
            item_count,
            revision: String::new(),
        });
    }

    log::info!(
        "linux_privacy_discovery_ready browser_count={} profile_count={} system_trace_count={} elapsed_ms={}",
        browsers.len(),
        browsers
            .iter()
            .map(|browser| browser.profiles.len())
            .sum::<usize>(),
        system_traces.len(),
        started.elapsed().as_millis()
    );
    Ok(PlatformPrivacyDiscovery {
        browsers,
        applications: Vec::new(),
        system_traces,
    })
}

fn ensure_not_cancelled(cancellation: &PlatformCancellation) -> PlatformResult<()> {
    if cancellation.is_cancelled() {
        return Err(PlatformError::new(
            crate::PlatformErrorCode::UserCancelled,
            "privacy discovery was cancelled",
        ));
    }
    Ok(())
}

fn push_chromium_browser(
    browsers: &mut Vec<PlatformPrivacyBrowser>,
    key: &str,
    display_name: &str,
    root: PathBuf,
    cache_root: PathBuf,
    process_names: &[&str],
) {
    if !directory_without_links(&root) {
        return;
    }
    let profiles = discover_chromium_profiles(key, &root, &cache_root);
    if profiles.is_empty() {
        return;
    }
    browsers.push(PlatformPrivacyBrowser {
        provider_key: key.to_string(),
        display_name: display_name.to_string(),
        application_path: None,
        kind: PlatformPrivacyBrowserKind::Chromium,
        process_names: process_names
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
        profiles,
    });
}

fn push_firefox_browser(
    browsers: &mut Vec<PlatformPrivacyBrowser>,
    key: &str,
    display_name: &str,
    root: PathBuf,
    cache_root: PathBuf,
) {
    if !directory_without_links(&root) {
        return;
    }
    let profiles = discover_firefox_profiles(&root, &cache_root);
    if profiles.is_empty() {
        return;
    }
    browsers.push(PlatformPrivacyBrowser {
        provider_key: key.to_string(),
        display_name: display_name.to_string(),
        application_path: None,
        kind: PlatformPrivacyBrowserKind::Firefox,
        process_names: vec!["firefox".to_string()],
        profiles,
    });
}

pub(crate) fn clear(trace: PlatformPrivacySystemTraceKind) -> PlatformResult<bool> {
    Err(PlatformError::new(
        crate::PlatformErrorCode::Unsupported,
        format!(
            "Linux privacy trace {trace:?} is file-backed and must be cleared through the Core safety boundary"
        ),
    ))
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
            let profile_root = entry.path();
            if !firefox_profile_has_supported_sources(&profile_root) {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let display_name = display_names
                .get(&dir_name)
                .cloned()
                .unwrap_or_else(|| dir_name.clone());

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

fn firefox_profile_has_supported_sources(root: &Path) -> bool {
    [
        "places.sqlite",
        "cookies.sqlite",
        "logins.json",
        "formhistory.sqlite",
        "permissions.sqlite",
        "sessionstore-backups",
        "storage/default",
    ]
    .iter()
    .any(|source| root.join(source).exists())
}

fn firefox_display_names(profiles_root: &Path) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    let ini_path = profiles_root.join("profiles.ini");
    let Some(content) = read_bounded_text(&ini_path, 1024 * 1024) else {
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
                let dir_name = path.rsplit('/').next().unwrap_or(&path).to_string();
                map.insert(dir_name, name);
            }
        }
    }
    if let (Some(name), Some(path)) = (current_name, current_path) {
        let dir_name = path.rsplit('/').next().unwrap_or(&path).to_string();
        map.insert(dir_name, name);
    }
    map
}

fn regular_file_without_links(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

fn directory_without_links(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn read_bounded_text(path: &Path, maximum_bytes: u64) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > maximum_bytes {
        return None;
    }
    let mut content = String::new();
    file.take(maximum_bytes.saturating_add(1))
        .read_to_string(&mut content)
        .ok()?;
    (content.len() as u64 <= maximum_bytes).then_some(content)
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

fn count_xbel_bookmarks(path: &Path) -> u64 {
    let Ok(file) = fs::File::open(path) else {
        return 0;
    };
    BufReader::new(file)
        .lines()
        .take(100_000)
        .filter_map(Result::ok)
        .map(|line| line.match_indices("<bookmark ").count() as u64)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "mangodisk-linux-privacy-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("fixture directory should be created");
        directory
    }

    #[test]
    fn firefox_profile_name_uses_the_profile_directory_leaf() {
        let directory = fixture_directory("firefox-profile-name");
        fs::write(
            directory.join("profiles.ini"),
            "[Profile0]\nName=Personal\nPath=Profiles/abc.default-release\n",
        )
        .unwrap();

        let names = firefox_display_names(&directory);

        assert_eq!(names.get("abc.default-release"), Some(&"Personal".into()));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn xbel_counter_counts_bookmarks_without_reading_document_contents() {
        let directory = fixture_directory("xbel-count");
        let path = directory.join("recently-used.xbel");
        fs::write(
            &path,
            "<xbel>\n<bookmark href=\"file:///one\"/>\n<bookmark href=\"file:///two\"/>\n</xbel>\n",
        )
        .unwrap();

        assert_eq!(count_xbel_bookmarks(&path), 2);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn native_clear_rejects_file_backed_linux_traces() {
        let error = clear(PlatformPrivacySystemTraceKind::RecentDocumentHistory).unwrap_err();
        assert_eq!(error.code(), crate::PlatformErrorCode::Unsupported);
    }

    #[test]
    fn sandboxed_firefox_location_is_exposed_with_a_distinct_identity() {
        let directory = fixture_directory("sandboxed-firefox");
        let profile = directory.join("example.default");
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join("places.sqlite"), b"fixture").unwrap();
        let mut browsers = Vec::new();

        push_firefox_browser(
            &mut browsers,
            "firefox-snap",
            "Firefox (Snap)",
            directory.clone(),
            directory.join("cache"),
        );

        assert_eq!(browsers.len(), 1);
        assert_eq!(browsers[0].provider_key, "firefox-snap");
        assert_eq!(browsers[0].profiles.len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
