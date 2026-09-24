use std::time::Duration;

use crate::{
    run_controlled_command_with_log_policy, ApplicationInventorySource, ApplicationSourceIdentity,
    ApplicationUninstallDiagnostic, ControlledCommandError, ControlledCommandLimits,
    ControlledCommandLogPolicy, ControlledEnvironmentPolicy, ControlledExecutable,
    InstalledApplication, PlatformCancellation,
};

const INVENTORY_LIMITS: ControlledCommandLimits = ControlledCommandLimits {
    timeout: Duration::from_secs(30),
    stdout_bytes: 8 * 1024 * 1024,
    stderr_bytes: 64 * 1024,
};

/// Aggregates installed applications across the available Linux package managers.
pub(crate) fn discover_all(cancellation: &PlatformCancellation) -> Vec<InstalledApplication> {
    let started = std::time::Instant::now();
    let mut applications = Vec::new();
    applications.extend(discover_apt(cancellation));
    if cancellation.is_cancelled() {
        return applications;
    }
    applications.extend(discover_snap(cancellation));
    if cancellation.is_cancelled() {
        return applications;
    }
    applications.extend(discover_flatpak(cancellation));
    if cancellation.is_cancelled() {
        return applications;
    }
    applications.extend(discover_pacman(cancellation));
    applications.sort_by_cached_key(|application| {
        (
            application.name.to_lowercase(),
            application.catalog_identifier.clone(),
        )
    });
    log::info!(
        "linux_package_inventory_ready application_count={} elapsed_ms={}",
        applications.len(),
        started.elapsed().as_millis()
    );
    applications
}

fn resolve_on_path(name: &str) -> Option<ControlledExecutable> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find_map(|candidate| {
            candidate
                .is_file()
                .then(|| ControlledExecutable::capture(&candidate).ok())
                .flatten()
        })
}

fn command_output(
    command_id: &'static str,
    executable_name: &str,
    arguments: &[&str],
    cancellation: &PlatformCancellation,
) -> Option<Vec<u8>> {
    let executable = resolve_on_path(executable_name)?;
    match run_controlled_command_with_log_policy(
        command_id,
        &executable,
        arguments,
        ControlledEnvironmentPolicy::Inherit,
        INVENTORY_LIMITS,
        ControlledCommandLogPolicy::ExceptionalOnly,
        &|| cancellation.is_cancelled(),
    ) {
        Ok(output) if output.status.success() => Some(output.stdout),
        Ok(output) => {
            log::warn!(
                "linux_package_inventory_source_failed source={} exit_code={:?} stderr_bytes={} elapsed_ms={}",
                executable_name,
                output.status.code(),
                output.stderr_bytes,
                output.elapsed_ms
            );
            None
        }
        Err(ControlledCommandError::Cancelled) => None,
        Err(error) => {
            log::warn!(
                "linux_package_inventory_source_failed source={} reason={}",
                executable_name,
                error.as_str()
            );
            None
        }
    }
}

/// Builds a Linux `InstalledApplication` from discovered package facts.
fn application(
    source: ApplicationInventorySource,
    package_identifier: &str,
    display_name: &str,
    version: Option<String>,
    publisher: Option<String>,
    estimated_bytes: u64,
) -> InstalledApplication {
    let package_identifier = package_identifier.to_string();
    let identifier = format!(
        "linux-{}:{package_identifier}",
        inventory_source_code(source)
    );
    InstalledApplication {
        catalog_identifier: identifier.clone(),
        primary_identifier: identifier,
        identifiers: vec![package_identifier.clone()],
        source_identities: vec![ApplicationSourceIdentity {
            source,
            identifier: package_identifier,
        }],
        name: display_name.to_string(),
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

fn discover_apt(cancellation: &PlatformCancellation) -> Vec<InstalledApplication> {
    let Some(output) = command_output(
        "linux-package-inventory-apt",
        "dpkg-query",
        &[
            "-W",
            "-f",
            "${Package}\t${Version}\t${Maintainer}\t${Installed-Size}\t${db:Status-Status}\n",
        ],
        cancellation,
    ) else {
        return Vec::new();
    };

    let stdout = String::from_utf8_lossy(&output);
    stdout.lines().filter_map(parse_apt_line).collect()
}

fn parse_apt_line(line: &str) -> Option<InstalledApplication> {
    let parts: Vec<&str> = line.split('\t').collect();
    // dpkg-query also reports removed packages that retain configuration files.
    if parts.len() != 5 || parts[0].is_empty() || parts[4].trim() != "installed" {
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
        parts[0],
        version,
        publisher,
        size_kb.saturating_mul(1024),
    ))
}

// ── snap ────────────────────────────────────────────────────────────────────

fn discover_snap(cancellation: &PlatformCancellation) -> Vec<InstalledApplication> {
    let Some(output) = command_output(
        "linux-package-inventory-snap",
        "snap",
        &["list"],
        cancellation,
    ) else {
        return Vec::new();
    };

    let stdout = String::from_utf8_lossy(&output);
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
        parts[0],
        version,
        publisher,
        0,
    ))
}

// ── flatpak ─────────────────────────────────────────────────────────────────

fn discover_flatpak(cancellation: &PlatformCancellation) -> Vec<InstalledApplication> {
    let Some(output) = command_output(
        "linux-package-inventory-flatpak",
        "flatpak",
        &["list", "--app", "--columns=application,version,origin,name"],
        cancellation,
    ) else {
        return Vec::new();
    };

    let stdout = String::from_utf8_lossy(&output);
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
        &app_id,
        &display_name,
        version,
        origin,
        0,
    ))
}

// ── pacman (Arch Linux) ────────────────────────────────────────────────────

fn discover_pacman(cancellation: &PlatformCancellation) -> Vec<InstalledApplication> {
    let executable = if resolve_on_path("pacman").is_some() {
        "pacman"
    } else if resolve_on_path("yay").is_some() {
        "yay"
    } else {
        return Vec::new();
    };
    let Some(output) = command_output(
        "linux-package-inventory-pacman",
        executable,
        &["-Q"],
        cancellation,
    ) else {
        return Vec::new();
    };

    let stdout = String::from_utf8_lossy(&output);
    parse_pacman_output(&stdout)
}

fn parse_pacman_output(output: &str) -> Vec<InstalledApplication> {
    output
        .lines()
        .filter_map(|line| {
            let (name, version) = line.split_once(' ')?;
            (!name.is_empty() && !version.is_empty()).then(|| {
                application(
                    ApplicationInventorySource::LinuxPacman,
                    name,
                    name,
                    Some(version.to_string()),
                    None,
                    0,
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_line_extracts_package() {
        let application =
            parse_apt_line("firefox\t128.0-1\tMozilla Maintainers\t198450\tinstalled\n").unwrap();
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
    fn apt_line_skips_packages_that_are_not_installed() {
        assert!(
            parse_apt_line("firefox\t128.0-1\tMozilla Maintainers\t0\tconfig-files\n").is_none()
        );
        assert!(
            parse_apt_line("firefox\t128.0-1\tMozilla Maintainers\t0\thalf-configured\n").is_none()
        );
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
        assert_eq!(application.identifiers, ["org.mozilla.firefox"]);
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
        let output = "firefox 128.0-1\nvim 9.1.0002-1\n";
        let entries = parse_pacman_output(output);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "firefox");
        assert_eq!(entries[0].version.as_deref(), Some("128.0-1"));
        assert_eq!(entries[0].estimated_bytes, 0);
        assert_eq!(entries[1].name, "vim");
        assert_eq!(
            entries[1].source_identities[0].source,
            ApplicationInventorySource::LinuxPacman
        );
    }
}
