use std::process::Command;

use crate::{
    ApplicationInventorySource, ApplicationSourceIdentity, ApplicationUninstallDiagnostic,
    InstalledApplication,
};

/// Aggregates installed applications across the available Linux package managers.
pub(crate) fn discover_all() -> Vec<InstalledApplication> {
    let mut applications = Vec::new();
    applications.extend(discover_apt());
    applications.extend(discover_snap());
    applications.extend(discover_flatpak());
    applications.extend(discover_pacman());
    applications.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    applications
}

fn command_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Builds a Linux `InstalledApplication` from discovered package facts.
fn application(
    source: ApplicationInventorySource,
    package_name: &str,
    version: Option<String>,
    publisher: Option<String>,
    estimated_bytes: u64,
) -> InstalledApplication {
    let package_name = package_name.to_string();
    let identifier = format!("linux-{}:{package_name}", inventory_source_code(source));
    InstalledApplication {
        catalog_identifier: identifier.clone(),
        primary_identifier: identifier,
        identifiers: vec![package_name.clone()],
        source_identities: vec![ApplicationSourceIdentity {
            source,
            identifier: package_name.clone(),
        }],
        name: package_name,
        version,
        publisher,
        estimated_bytes,
        last_used_at_ms: None,
        installed_at_ms: None,
        icon_path: None,
        bundle_path: None,
        executable_paths: Vec::new(),
        uninstall_registration: None,
        uninstall_diagnostic: Some(ApplicationUninstallDiagnostic::UnsupportedCommandHost),
    }
}

const fn inventory_source_code(source: ApplicationInventorySource) -> &'static str {
    match source {
        ApplicationInventorySource::LinuxApt => "apt",
        ApplicationInventorySource::LinuxSnap => "snap",
        ApplicationInventorySource::LinuxFlatpak => "flatpak",
        ApplicationInventorySource::LinuxPacman => "pacman",
        ApplicationInventorySource::MacosBundle
        | ApplicationInventorySource::WindowsRegistry
        | ApplicationInventorySource::WindowsMsi
        | ApplicationInventorySource::WindowsAppx
        | ApplicationInventorySource::Winget
        | ApplicationInventorySource::Steam
        | ApplicationInventorySource::Scoop
        | ApplicationInventorySource::Chocolatey => "other",
    }
}

// ── apt / dpkg ────────────────────────────────────────────────────────────

fn discover_apt() -> Vec<InstalledApplication> {
    if !command_exists("dpkg-query") {
        return Vec::new();
    }
    let output = match Command::new("dpkg-query")
        .args([
            "-W",
            "-f",
            "${Package}\t${Version}\t${Maintainer}\t${Installed-Size}\n",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().filter_map(parse_apt_line).collect()
}

fn parse_apt_line(line: &str) -> Option<InstalledApplication> {
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.len() < 2 || parts[0].is_empty() {
        return None;
    }
    let version = parts
        .get(1)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let publisher = parts
        .get(2)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let size_kb: u64 = parts
        .get(3)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    Some(application(
        ApplicationInventorySource::LinuxApt,
        parts[0],
        version,
        publisher,
        size_kb * 1024,
    ))
}

// ── snap ────────────────────────────────────────────────────────────────────

fn discover_snap() -> Vec<InstalledApplication> {
    if !command_exists("snap") {
        return Vec::new();
    }
    let output = match Command::new("snap").args(["list"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().skip(1).filter_map(parse_snap_line).collect()
}

fn parse_snap_line(line: &str) -> Option<InstalledApplication> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() || parts[0].is_empty() {
        return None;
    }
    let version = parts.get(1).map(|s| s.to_string());
    let publisher = parts.get(2).map(|s| s.to_string());
    Some(application(
        ApplicationInventorySource::LinuxSnap,
        parts[0],
        version,
        publisher,
        0,
    ))
}

// ── flatpak ─────────────────────────────────────────────────────────────────

fn discover_flatpak() -> Vec<InstalledApplication> {
    if !command_exists("flatpak") {
        return Vec::new();
    }
    let output = match Command::new("flatpak")
        .args(["list", "--app", "--columns=application,version,origin,name"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().filter_map(parse_flatpak_line).collect()
}

fn parse_flatpak_line(line: &str) -> Option<InstalledApplication> {
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.is_empty() || parts[0].is_empty() {
        return None;
    }
    let app_id = parts[0].to_string();
    let version = parts
        .get(1)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let origin = parts
        .get(2)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let display_name = parts
        .get(3)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| app_id.clone());
    Some(application(
        ApplicationInventorySource::LinuxFlatpak,
        &display_name,
        version,
        origin,
        0,
    ))
}

// ── pacman (Arch Linux) ────────────────────────────────────────────────────

fn discover_pacman() -> Vec<InstalledApplication> {
    let (cmd, _) = if command_exists("pacman") {
        ("pacman", ())
    } else if command_exists("yay") {
        ("yay", ())
    } else {
        return Vec::new();
    };

    let output = match Command::new(cmd).arg("-Qi").output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_pacman_output(&stdout)
}

fn parse_pacman_output(output: &str) -> Vec<InstalledApplication> {
    let mut entries = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;
    let mut current_publisher: Option<String> = None;
    let mut current_size: u64 = 0;

    for line in output.lines() {
        if let Some(value) = line.strip_prefix("Name            : ") {
            flush(
                &mut entries,
                &mut current_name,
                &mut current_version,
                &mut current_publisher,
                &mut current_size,
            );
            current_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Version         : ") {
            current_version = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("URL             : ") {
            let value = value.trim();
            if current_publisher.is_none() && !value.is_empty() {
                current_publisher = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix("Installed Size  : ") {
            current_size = parse_pacman_size(value.trim());
        }
    }
    flush(
        &mut entries,
        &mut current_name,
        &mut current_version,
        &mut current_publisher,
        &mut current_size,
    );

    entries
}

#[allow(clippy::too_many_arguments)]
fn flush(
    entries: &mut Vec<InstalledApplication>,
    name: &mut Option<String>,
    version: &mut Option<String>,
    publisher: &mut Option<String>,
    size: &mut u64,
) {
    if let Some(name) = name.take() {
        entries.push(application(
            ApplicationInventorySource::LinuxPacman,
            &name,
            version.take(),
            publisher.take(),
            *size,
        ));
    }
    *size = 0;
}

fn parse_pacman_size(s: &str) -> u64 {
    // Formats observed from `pacman -Qi`: "123.45 MiB", "1234.56 KiB", "4096 B".
    let s = s.trim();
    if let Some(value) = s.strip_suffix(" MiB") {
        value
            .parse::<f64>()
            .map(|v| (v * 1024.0 * 1024.0) as u64)
            .unwrap_or(0)
    } else if let Some(value) = s.strip_suffix(" KiB") {
        value
            .parse::<f64>()
            .map(|v| (v * 1024.0) as u64)
            .unwrap_or(0)
    } else if let Some(value) = s.strip_suffix(" B") {
        value.trim().parse::<u64>().unwrap_or(0)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_line_extracts_package() {
        let application =
            parse_apt_line("firefox\t128.0-1\tMozilla Maintainers\t198450\n").unwrap();
        assert_eq!(application.name, "firefox");
        assert_eq!(application.version.as_deref(), Some("128.0-1"));
        assert_eq!(
            application.publisher.as_deref(),
            Some("Mozilla Maintainers")
        );
        assert_eq!(application.estimated_bytes, 198_450 * 1024);
        assert_eq!(
            application.source_identities[0].source,
            ApplicationInventorySource::LinuxApt
        );
        assert!(application.catalog_identifier.starts_with("linux-apt:"));
    }

    #[test]
    fn snap_line_extracts_package() {
        let application = parse_snap_line("firefox\t142.0\tmozilla\tstable\n").unwrap();
        assert_eq!(application.name, "firefox");
        assert_eq!(application.version.as_deref(), Some("142.0"));
        assert_eq!(application.publisher.as_deref(), Some("mozilla"));
    }

    #[test]
    fn flatpak_line_extracts_application() {
        let application =
            parse_flatpak_line("org.mozilla.firefox\t142.0\tflathub\tFirefox\n").unwrap();
        assert_eq!(application.name, "Firefox");
        assert_eq!(application.version.as_deref(), Some("142.0"));
        assert_eq!(application.publisher.as_deref(), Some("flathub"));
        assert_eq!(
            application.source_identities[0].source,
            ApplicationInventorySource::LinuxFlatpak
        );
    }

    #[test]
    fn flatpak_line_falls_back_to_app_id() {
        let application = parse_flatpak_line("org.example.App\t\t\t\n").unwrap();
        assert_eq!(application.name, "org.example.App");
    }

    #[test]
    fn pacman_output_extracts_packages() {
        let output = r#"Name            : firefox
Version         : 128.0-1
URL             : https://www.mozilla.org/firefox/
Installed Size  : 198.45 MiB

Name            : vim
Version         : 9.1.0002-1
URL             : https://www.vim.org/
Installed Size  : 3.45 MiB
"#;
        let entries = parse_pacman_output(output);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "firefox");
        assert_eq!(entries[0].version.as_deref(), Some("128.0-1"));
        assert!(entries[0].estimated_bytes > 100_000_000);
        assert_eq!(entries[1].name, "vim");
        assert_eq!(
            entries[1].source_identities[0].source,
            ApplicationInventorySource::LinuxPacman
        );
    }

    #[test]
    fn pacman_size_variants() {
        assert_eq!(parse_pacman_size("198.45 MiB"), 208_089_907);
        assert_eq!(parse_pacman_size("3.45 MiB"), 3_617_587);
        assert_eq!(parse_pacman_size("128 KiB"), 131_072);
        assert_eq!(parse_pacman_size("4096 B"), 4096);
        assert_eq!(parse_pacman_size("unknown"), 0);
    }
}
