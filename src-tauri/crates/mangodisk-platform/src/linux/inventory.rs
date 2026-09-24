use std::{os::unix::fs::MetadataExt, path::PathBuf};

use crate::{PlatformCancellation, PlatformError, PlatformResult, SystemInventory};

const DEVELOPER_TOOL_NAMES: &[&str] = &[
    "cargo",
    "conda",
    "docker",
    "go",
    "java",
    "node",
    "npm",
    "pnpm",
    "python3",
    "pip3",
    "rustc",
    "rustup",
    "cmake",
    "gcc",
    "g++",
    "make",
    "git",
    "helm",
    "kubectl",
    "terraform",
    "ansible",
];

pub(crate) fn system_inventory() -> PlatformResult<SystemInventory> {
    system_inventory_with_cancellation(&PlatformCancellation::new(|| false))
}

pub(crate) fn system_inventory_with_cancellation(
    cancellation: &PlatformCancellation,
) -> PlatformResult<SystemInventory> {
    let started = std::time::Instant::now();
    let installed_applications = super::package_managers::discover_all(cancellation);
    if cancellation.is_cancelled() {
        return Err(PlatformError::new(
            crate::PlatformErrorCode::UserCancelled,
            "application inventory capture was cancelled",
        ));
    }
    let (developer_tools, developer_tools_complete) =
        crate::inventory::detect_tools(DEVELOPER_TOOL_NAMES);
    let (filesystem_kinds, filesystem_complete) = detect_filesystem_kinds();
    let (capabilities, capabilities_complete) = detect_capabilities();

    let inventory = SystemInventory {
        os_version: os_version(),
        installed_applications,
        // Package databases do not cover portable AppImages, locally installed
        // desktop entries, or arbitrary binaries. Positive package matches are
        // still useful, but absence must never be treated as proof that an
        // application is not installed.
        installed_applications_complete: false,
        developer_tools,
        developer_tools_complete,
        filesystem_kinds,
        filesystem_complete,
        capabilities,
        capabilities_complete,
    };
    log::info!(
        "linux_system_inventory_ready applications={} developer_tools={} filesystems={} capabilities={} applications_complete={} developer_tools_complete={} filesystem_complete={} capabilities_complete={} elapsed_ms={}",
        inventory.installed_applications.len(),
        inventory.developer_tools.len(),
        inventory.filesystem_kinds.len(),
        inventory.capabilities.len(),
        inventory.installed_applications_complete,
        inventory.developer_tools_complete,
        inventory.filesystem_complete,
        inventory.capabilities_complete,
        started.elapsed().as_millis()
    );
    Ok(inventory)
}

pub(crate) fn system_inventory_revision() -> PlatformResult<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(os_version().as_bytes());
    let mut sources = vec![
        PathBuf::from("/var/lib/dpkg/status"),
        PathBuf::from("/var/lib/snapd/state.json"),
        PathBuf::from("/var/lib/flatpak/repo/summary"),
        PathBuf::from("/var/lib/pacman/local"),
    ];
    if let Some(data) = dirs::data_local_dir() {
        sources.push(data.join("flatpak/repo/summary"));
    }
    for source in sources {
        hasher.update(source.as_os_str().as_encoded_bytes());
        match std::fs::metadata(&source) {
            Ok(metadata) => {
                hasher.update(&metadata.dev().to_le_bytes());
                hasher.update(&metadata.ino().to_le_bytes());
                hasher.update(&metadata.len().to_le_bytes());
                hasher.update(&metadata.mtime().to_le_bytes());
                hasher.update(&metadata.mtime_nsec().to_le_bytes());
                hasher.update(&metadata.ctime().to_le_bytes());
                hasher.update(&metadata.ctime_nsec().to_le_bytes());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(b"missing");
            }
            Err(error) => return Err(PlatformError::io("read package inventory revision", &error)),
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub(crate) fn running_process_names(
    cancellation: &PlatformCancellation,
) -> PlatformResult<Vec<String>> {
    let mut names = std::collections::BTreeSet::new();
    let proc_dir = std::path::Path::new("/proc");
    if !proc_dir.exists() {
        return Ok(Vec::new());
    }

    let entries =
        std::fs::read_dir(proc_dir).map_err(|error| PlatformError::io("read /proc", &error))?;

    let effective_uid = unsafe { libc::geteuid() };
    for entry in entries {
        if cancellation.is_cancelled() {
            return Err(PlatformError::new(
                crate::PlatformErrorCode::UserCancelled,
                "process snapshot capture was cancelled",
            ));
        }
        let Ok(entry) = entry else {
            continue;
        };
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if entry
            .metadata()
            .map_or(true, |metadata| metadata.uid() != effective_uid)
        {
            continue;
        }
        let comm_path = entry.path().join("comm");
        if let Ok(comm) = std::fs::read_to_string(&comm_path) {
            let comm = comm.trim().to_string();
            if !comm.is_empty() {
                names.insert(comm);
            }
        }
    }

    Ok(names.into_iter().collect())
}

fn detect_filesystem_kinds() -> (Vec<String>, bool) {
    match super::volumes::system_filesystem_kind() {
        Ok(kind) => (vec![kind], true),
        Err(error) => {
            log::warn!("linux_filesystem_inventory_failed error={error}");
            (Vec::new(), false)
        }
    }
}

fn detect_capabilities() -> (Vec<String>, bool) {
    let (tools, complete) =
        crate::inventory::detect_tools(&["flatpak", "snap", "docker", "ollama"]);
    let mut caps = tools.into_iter().map(|tool| tool.name).collect::<Vec<_>>();
    if std::path::Path::new("/proc/sys/fs/inotify").exists() {
        caps.push("inotify".to_string());
    }
    (caps, complete)
}

fn os_version() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|content| {
            for line in content.lines() {
                if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
                    return Some(value.trim_matches('"').to_string());
                }
            }
            None
        })
        .unwrap_or_else(|| "Linux".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_revision_is_stable_and_not_a_hostname_placeholder() {
        let first = system_inventory_revision().unwrap();
        let second = system_inventory_revision().unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn system_filesystem_is_reported_without_spawning_stat() {
        let (filesystems, complete) = detect_filesystem_kinds();
        assert!(complete);
        assert_eq!(filesystems.len(), 1);
        assert!(!filesystems[0].is_empty());
    }

    #[test]
    fn cancelled_inventory_stops_before_package_enumeration() {
        let cancellation = PlatformCancellation::new(|| true);
        let error = system_inventory_with_cancellation(&cancellation).unwrap_err();
        assert_eq!(error.code(), crate::PlatformErrorCode::UserCancelled);
    }

    #[test]
    fn cancelled_process_snapshot_uses_the_stable_code() {
        let cancellation = PlatformCancellation::new(|| true);
        let error = running_process_names(&cancellation).unwrap_err();
        assert_eq!(error.code(), crate::PlatformErrorCode::UserCancelled);
    }
}
