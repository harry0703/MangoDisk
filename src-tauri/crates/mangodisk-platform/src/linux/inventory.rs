use crate::{PlatformError, PlatformResult, SystemInventory};

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
    let installed_applications = super::package_managers::discover_all();
    let (developer_tools, developer_tools_complete) =
        crate::inventory::detect_tools(DEVELOPER_TOOL_NAMES);

    Ok(SystemInventory {
        os_version: os_version(),
        installed_applications,
        installed_applications_complete: true,
        developer_tools,
        developer_tools_complete,
        filesystem_kinds: detect_filesystem_kinds(),
        filesystem_complete: true,
        capabilities: detect_capabilities(),
        capabilities_complete: true,
    })
}

pub(crate) fn system_inventory_revision() -> PlatformResult<String> {
    let os = os_version();
    let hostname = std::env::var("HOSTNAME").unwrap_or_default();
    Ok(format!("{os}:{hostname}"))
}

pub(crate) fn running_process_names() -> PlatformResult<Vec<String>> {
    let mut names = Vec::new();
    let proc_dir = std::path::Path::new("/proc");
    if !proc_dir.exists() {
        return Ok(names);
    }

    let entries =
        std::fs::read_dir(proc_dir).map_err(|error| PlatformError::io("read /proc", &error))?;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let comm_path = entry.path().join("comm");
        if let Ok(comm) = std::fs::read_to_string(&comm_path) {
            let comm = comm.trim().to_string();
            if !comm.is_empty() {
                names.push(comm);
            }
        }
    }

    Ok(names)
}

fn detect_filesystem_kinds() -> Vec<String> {
    let mut kinds = Vec::new();
    if let Ok(output) = std::process::Command::new("stat")
        .args(["-f", "-c", "%T", "/"])
        .output()
    {
        let fs_type = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !fs_type.is_empty() {
            kinds.push(fs_type);
        }
    }
    kinds
}

fn detect_capabilities() -> Vec<String> {
    let mut caps = Vec::new();
    for name in ["flatpak", "snap", "docker", "ollama"] {
        if std::process::Command::new("which")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            caps.push(name.to_string());
        }
    }
    if std::path::Path::new("/proc/sys/fs/inotify").exists() {
        caps.push("inotify".to_string());
    }
    caps
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
