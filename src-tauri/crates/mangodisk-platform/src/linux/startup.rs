use std::{fs, path::PathBuf, time::UNIX_EPOCH};

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
    let mut results = Vec::new();

    // XDG autostart entries
    if cancellation.is_cancelled() {
        return Err(PlatformError::operation_failed(
            "startup scan was cancelled",
        ));
    }
    results.push(scan_xdg_autostart()?);

    // systemd user services
    if cancellation.is_cancelled() {
        return Err(PlatformError::operation_failed(
            "startup scan was cancelled",
        ));
    }
    results.push(scan_systemd_user_services()?);

    Ok(results)
}

fn scan_xdg_autostart() -> PlatformResult<PlatformStartupSourceResult> {
    let mut items = Vec::new();
    let start = std::time::Instant::now();

    let autostart_dirs = [
        dirs::config_dir().map(|d| d.join("autostart")),
        Some(PathBuf::from("/etc/xdg/autostart")),
    ];

    for dir in autostart_dirs.into_iter().flatten() {
        if !dir.exists() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            if let Some(item) = parse_desktop_autostart(&path) {
                items.push(item);
            }
        }
    }

    Ok(PlatformStartupSourceResult {
        source_id: "xdg-autostart".to_string(),
        required: false,
        status: PlatformStartupCoverageStatus::Complete,
        reason: None,
        items,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

fn parse_desktop_autostart(path: &PathBuf) -> Option<PlatformStartupArtifact> {
    let content = fs::read_to_string(path).ok()?;
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

    // Skip entries only for specific desktop environments
    if !only_show_in.is_empty() || !not_show_in.is_empty() {
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
        control_capability: PlatformStartupControlCapability::Toggleable,
        trust: PlatformStartupTrustState::Unknown,
        modified_at_ms: modified_at,
        diagnostics: Vec::new(),
    })
}

fn scan_systemd_user_services() -> PlatformResult<PlatformStartupSourceResult> {
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
        if !dir.exists() {
            continue;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
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
        status: PlatformStartupCoverageStatus::Complete,
        reason: None,
        items,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

fn parse_systemd_service(path: &PathBuf, user_scope: bool) -> Option<PlatformStartupArtifact> {
    let content = fs::read_to_string(path).ok()?;
    let mut description = None;
    let mut exec_start = None;
    let mut wanted_by = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Description=") {
            description = Some(value.to_string());
        } else if let Some(value) = trimmed.strip_prefix("ExecStart=") {
            exec_start = Some(value.to_string());
        } else if let Some(value) = trimmed.strip_prefix("WantedBy=") {
            wanted_by = Some(value.to_string());
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

    let enabled = wanted_by
        .as_deref()
        .map(|w| w.contains("multi-user.target") || w.contains("graphical.target"))
        .unwrap_or(false);

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
        configured_state: if enabled {
            PlatformStartupConfiguredState::Enabled
        } else {
            PlatformStartupConfiguredState::Disabled
        },
        runtime_state: PlatformStartupRuntimeState::Unknown,
        control_capability: PlatformStartupControlCapability::Toggleable,
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
