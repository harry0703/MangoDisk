use std::{collections::HashSet, ffi::OsString, fs, io::Read, path::PathBuf, time::UNIX_EPOCH};

const MAX_STARTUP_FILE_BYTES: u64 = 1024 * 1024;

use crate::{
    PlatformCancellation, PlatformError, PlatformResult, PlatformStartupArtifact,
    PlatformStartupChangeRequest, PlatformStartupChangeResult, PlatformStartupConfiguredState,
    PlatformStartupControlCapability, PlatformStartupCoverageReason, PlatformStartupCoverageStatus,
    PlatformStartupIdentityConfidence, PlatformStartupOwner, PlatformStartupRuntimeState,
    PlatformStartupScope, PlatformStartupSourceKind, PlatformStartupSourceResult,
    PlatformStartupSummarySource, PlatformStartupTarget, PlatformStartupTargetKind,
    PlatformStartupTrigger, PlatformStartupTrustState,
};

pub(crate) fn scan_startup(
    cancellation: &PlatformCancellation,
) -> PlatformResult<Vec<PlatformStartupSourceResult>> {
    let started = std::time::Instant::now();
    let mut results = Vec::new();

    // XDG autostart entries
    if cancellation.is_cancelled() {
        return Err(PlatformError::new(
            crate::PlatformErrorCode::UserCancelled,
            "startup scan was cancelled",
        ));
    }
    results.push(scan_xdg_autostart(cancellation)?);

    // systemd user services
    if cancellation.is_cancelled() {
        return Err(PlatformError::new(
            crate::PlatformErrorCode::UserCancelled,
            "startup scan was cancelled",
        ));
    }
    results.push(scan_systemd_user_services(cancellation)?);

    log::info!(
        "linux_startup_scan_ready source_count={} item_count={} partial_source_count={} elapsed_ms={}",
        results.len(),
        results.iter().map(|source| source.items.len()).sum::<usize>(),
        results
            .iter()
            .filter(|source| source.status != PlatformStartupCoverageStatus::Complete)
            .count(),
        started.elapsed().as_millis()
    );

    Ok(results)
}

fn scan_xdg_autostart(
    cancellation: &PlatformCancellation,
) -> PlatformResult<PlatformStartupSourceResult> {
    let mut autostart_dirs = Vec::new();
    if let Some(config_dir) = dirs::config_dir() {
        autostart_dirs.push(config_dir.join("autostart"));
    }
    let system_config_dirs = std::env::var_os("XDG_CONFIG_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("/etc/xdg"));
    autostart_dirs.extend(
        std::env::split_paths(&system_config_dirs)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join("autostart")),
    );
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|desktop| !desktop.is_empty())
        .map(String::from)
        .collect();
    scan_xdg_autostart_dirs(&autostart_dirs, &desktops, cancellation)
}

fn scan_xdg_autostart_dirs(
    autostart_dirs: &[PathBuf],
    desktops: &[String],
    cancellation: &PlatformCancellation,
) -> PlatformResult<PlatformStartupSourceResult> {
    let mut items = Vec::new();
    let start = std::time::Instant::now();
    let mut read_failure = false;
    let mut seen_names = HashSet::new();

    for dir in autostart_dirs {
        ensure_not_cancelled(cancellation)?;
        if !dir.exists() {
            continue;
        }
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                read_failure = true;
                log::warn!(
                    "linux_startup_source_read_failed source=xdg-autostart path={:?} error_kind={:?} os_code={:?}",
                    dir,
                    error.kind(),
                    error.raw_os_error()
                );
                continue;
            }
        };
        for entry in entries {
            ensure_not_cancelled(cancellation)?;
            let Ok(entry) = entry else {
                read_failure = true;
                continue;
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            // A higher-priority entry masks every lower-priority entry with the same filename.
            if !seen_names.insert(entry.file_name()) {
                continue;
            }
            if let Some(item) = parse_desktop_autostart(&path, desktops) {
                items.push(item);
            }
        }
    }

    Ok(PlatformStartupSourceResult {
        source_id: "xdg-autostart".to_string(),
        required: false,
        status: if read_failure {
            PlatformStartupCoverageStatus::Partial
        } else {
            PlatformStartupCoverageStatus::Complete
        },
        reason: read_failure.then_some(PlatformStartupCoverageReason::AccessDenied),
        items,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

fn parse_desktop_autostart(path: &PathBuf, desktops: &[String]) -> Option<PlatformStartupArtifact> {
    let content = read_bounded_text(path)?;
    let mut name = None;
    let mut exec = None;
    let mut hidden = false;
    let mut only_show_in = Vec::new();
    let mut not_show_in = Vec::new();

    for line in content.lines() {
        if let Some(value) = line.strip_prefix("Name=") {
            name = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("Exec=") {
            exec = Some(value.to_string());
        } else if line.strip_prefix("Hidden=true").is_some() {
            hidden = true;
        } else if let Some(value) = line.strip_prefix("OnlyShowIn=") {
            only_show_in = value
                .split(';')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
        } else if let Some(value) = line.strip_prefix("NotShowIn=") {
            not_show_in = value
                .split(';')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
        }
    }

    if hidden || name.is_none() || exec.is_none() {
        return None;
    }

    if (!only_show_in.is_empty()
        && !desktops
            .iter()
            .any(|desktop| only_show_in.contains(desktop)))
        || desktops.iter().any(|desktop| not_show_in.contains(desktop))
    {
        return None;
    }

    let display_name = name.unwrap_or_default();
    let exec_str = exec.unwrap_or_default();
    let args: Vec<String> = exec_str
        .split_whitespace()
        .skip(1)
        .map(String::from)
        .collect();
    let program = exec_str.split_whitespace().next().unwrap_or("").to_string();

    let modified_at = fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);

    Some(PlatformStartupArtifact {
        provider_item_id: format!("xdg-autostart:{}", path.display()),
        source_kind: PlatformStartupSourceKind::AdvancedAutoRun,
        scope: PlatformStartupScope::CurrentUser,
        triggers: vec![PlatformStartupTrigger::UserLogon],
        display_name,
        configuration_path: Some(path.clone()),
        target: PlatformStartupTarget {
            kind: PlatformStartupTargetKind::Executable,
            identity_key: exec_str.clone(),
            path: None,
            executable_name: Some(program),
            arguments: args,
        },
        owner: PlatformStartupOwner {
            identity_key: None,
            name: None,
            publisher: None,
            summary: None,
            summary_source: PlatformStartupSummarySource::SourceLabel,
            version: None,
            icon_path: None,
            confidence: PlatformStartupIdentityConfidence::Unresolved,
        },
        configured_state: PlatformStartupConfiguredState::Enabled,
        runtime_state: PlatformStartupRuntimeState::Unknown,
        control_capability: PlatformStartupControlCapability::ViewOnly,
        trust: PlatformStartupTrustState::Unknown,
        modified_at_ms: modified_at,
        diagnostics: Vec::new(),
    })
}

fn scan_systemd_user_services(
    cancellation: &PlatformCancellation,
) -> PlatformResult<PlatformStartupSourceResult> {
    let mut items = Vec::new();
    let start = std::time::Instant::now();

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            return Ok(PlatformStartupSourceResult::unavailable(
                "systemd-user",
                false,
                PlatformStartupCoverageReason::StateUnavailable,
            ));
        }
    };

    let service_dirs = [
        home.join(".config/systemd/user"),
        PathBuf::from("/usr/lib/systemd/user"),
    ];

    for dir in &service_dirs {
        ensure_not_cancelled(cancellation)?;
        if !dir.exists() {
            continue;
        }
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                log::warn!(
                    "linux_startup_source_read_failed source=systemd-user path={:?} error_kind={:?} os_code={:?}",
                    dir,
                    error.kind(),
                    error.raw_os_error()
                );
                continue;
            }
        };
        for entry in entries {
            ensure_not_cancelled(cancellation)?;
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("service") {
                continue;
            }
            if let Some(item) = parse_systemd_service(&path, true) {
                items.push(item);
            }
        }
    }

    Ok(PlatformStartupSourceResult {
        source_id: "systemd-user".to_string(),
        required: false,
        // Unit files describe what can be enabled, not the effective systemd user-manager state.
        // Keep this source explicitly partial until the adapter queries that state safely.
        status: PlatformStartupCoverageStatus::Partial,
        reason: Some(PlatformStartupCoverageReason::StateUnavailable),
        items,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

fn parse_systemd_service(path: &PathBuf, user_scope: bool) -> Option<PlatformStartupArtifact> {
    let content = read_bounded_text(path)?;
    let mut description = None;
    let mut exec_start = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Description=") {
            description = Some(value.to_string());
        } else if let Some(value) = trimmed.strip_prefix("ExecStart=") {
            exec_start = Some(value.to_string());
        }
    }

    let file_name = path.file_stem()?.to_string_lossy().to_string();
    let display_name = description.unwrap_or_else(|| file_name.clone());
    let exec_str = exec_start.unwrap_or_default();
    let args: Vec<String> = exec_str
        .split_whitespace()
        .skip(1)
        .map(String::from)
        .collect();
    let program = exec_str.split_whitespace().next().unwrap_or("").to_string();

    let scope = if user_scope {
        PlatformStartupScope::CurrentUser
    } else {
        PlatformStartupScope::AllUsers
    };

    let modified_at = fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);

    Some(PlatformStartupArtifact {
        provider_item_id: format!(
            "systemd:{}:{}",
            if user_scope { "user" } else { "system" },
            file_name
        ),
        source_kind: PlatformStartupSourceKind::Service,
        scope,
        triggers: vec![PlatformStartupTrigger::Boot],
        display_name,
        configuration_path: Some(path.clone()),
        target: PlatformStartupTarget {
            kind: PlatformStartupTargetKind::Service,
            identity_key: file_name,
            path: None,
            executable_name: Some(program),
            arguments: args,
        },
        owner: PlatformStartupOwner {
            identity_key: None,
            name: None,
            publisher: None,
            summary: None,
            summary_source: PlatformStartupSummarySource::SourceLabel,
            version: None,
            icon_path: None,
            confidence: PlatformStartupIdentityConfidence::Probable,
        },
        configured_state: PlatformStartupConfiguredState::Unknown,
        runtime_state: PlatformStartupRuntimeState::Unknown,
        control_capability: PlatformStartupControlCapability::ViewOnly,
        trust: PlatformStartupTrustState::Unknown,
        modified_at_ms: modified_at,
        diagnostics: Vec::new(),
    })
}

pub(crate) fn change_startup_item(
    _request: &PlatformStartupChangeRequest,
    _authorization_prompt: Option<&str>,
) -> PlatformResult<PlatformStartupChangeResult> {
    Err(PlatformError::new(
        crate::PlatformErrorCode::Unsupported,
        "startup item management is not yet supported on Linux",
    ))
}

fn ensure_not_cancelled(cancellation: &PlatformCancellation) -> PlatformResult<()> {
    if cancellation.is_cancelled() {
        return Err(PlatformError::new(
            crate::PlatformErrorCode::UserCancelled,
            "startup scan was cancelled",
        ));
    }
    Ok(())
}

fn read_bounded_text(path: &PathBuf) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_STARTUP_FILE_BYTES {
        return None;
    }
    let mut content = String::new();
    file.take(MAX_STARTUP_FILE_BYTES.saturating_add(1))
        .read_to_string(&mut content)
        .ok()?;
    (content.len() as u64 <= MAX_STARTUP_FILE_BYTES).then_some(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_file(name: &str, content: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mangodisk-linux-startup-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        fs::write(&path, content).expect("fixture should be written");
        path
    }

    #[test]
    fn xdg_entries_are_view_only_until_mutation_is_supported() {
        let path = fixture_file(
            "xdg.desktop",
            "[Desktop Entry]\nName=Example\nExec=/usr/bin/example --background\n",
        );
        let item = parse_desktop_autostart(&path, &[]).unwrap();
        assert_eq!(
            item.control_capability,
            PlatformStartupControlCapability::ViewOnly
        );
        assert_eq!(
            item.configured_state,
            PlatformStartupConfiguredState::Enabled
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn systemd_unit_install_metadata_does_not_claim_effective_enablement() {
        let path = fixture_file(
            "example.service",
            "[Unit]\nDescription=Example\n[Service]\nExecStart=/usr/bin/example\n[Install]\nWantedBy=default.target\n",
        );
        let item = parse_systemd_service(&path, true).unwrap();
        assert_eq!(
            item.configured_state,
            PlatformStartupConfiguredState::Unknown
        );
        assert_eq!(
            item.control_capability,
            PlatformStartupControlCapability::ViewOnly
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn cancellation_uses_the_stable_user_cancelled_code() {
        let cancellation = PlatformCancellation::new(|| true);
        let error = ensure_not_cancelled(&cancellation).unwrap_err();
        assert_eq!(error.code(), crate::PlatformErrorCode::UserCancelled);
    }

    #[test]
    fn oversized_startup_files_are_ignored() {
        let path = fixture_file("oversized.desktop", "placeholder");
        fs::write(&path, vec![b'x'; MAX_STARTUP_FILE_BYTES as usize + 1]).unwrap();
        assert!(parse_desktop_autostart(&path, &[]).is_none());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn higher_priority_hidden_entry_masks_system_autostart() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-linux-startup-priority-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let user_dir = root.join("user");
        let system_dir = root.join("system");
        fs::create_dir_all(&user_dir).unwrap();
        fs::create_dir_all(&system_dir).unwrap();
        fs::write(
            user_dir.join("example.desktop"),
            "[Desktop Entry]\nHidden=true\n",
        )
        .unwrap();
        fs::write(
            system_dir.join("example.desktop"),
            "[Desktop Entry]\nName=Example\nExec=/usr/bin/example\n",
        )
        .unwrap();
        fs::write(
            system_dir.join("other.desktop"),
            "[Desktop Entry]\nName=Other\nExec=/usr/bin/other\n",
        )
        .unwrap();

        let result = scan_xdg_autostart_dirs(
            &[user_dir, system_dir],
            &[],
            &PlatformCancellation::new(|| false),
        )
        .unwrap();
        assert_eq!(result.status, PlatformStartupCoverageStatus::Complete);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].display_name, "Other");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desktop_restrictions_match_the_current_desktop() {
        let only_path = fixture_file(
            "only-show-in.desktop",
            "[Desktop Entry]\nName=GNOME only\nExec=/usr/bin/example\nOnlyShowIn=GNOME;\n",
        );
        let not_path = fixture_file(
            "not-show-in.desktop",
            "[Desktop Entry]\nName=Not KDE\nExec=/usr/bin/example\nNotShowIn=KDE;\n",
        );
        let gnome = vec!["ubuntu".to_string(), "GNOME".to_string()];
        let kde = vec!["KDE".to_string()];

        assert!(parse_desktop_autostart(&only_path, &gnome).is_some());
        assert!(parse_desktop_autostart(&only_path, &kde).is_none());
        assert!(parse_desktop_autostart(&not_path, &gnome).is_some());
        assert!(parse_desktop_autostart(&not_path, &kde).is_none());

        fs::remove_file(only_path).unwrap();
        fs::remove_file(not_path).unwrap();
    }
}
