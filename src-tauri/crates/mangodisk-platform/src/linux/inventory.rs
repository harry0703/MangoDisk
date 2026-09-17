use crate::{PlatformError, PlatformResult, SystemInventory};

pub(crate) fn system_inventory() -> PlatformResult<SystemInventory> {
    let inventory = SystemInventory {
        os_version: os_version(),
        installed_applications_complete: true,
        developer_tools_complete: true,
        filesystem_complete: true,
        capabilities_complete: true,
        ..Default::default()
    };
    Ok(inventory)
}

pub(crate) fn system_inventory_revision() -> PlatformResult<String> {
    // Use OS version + hostname as a lightweight revision identifier.
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
