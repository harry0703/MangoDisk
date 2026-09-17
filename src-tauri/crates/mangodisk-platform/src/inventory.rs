#[cfg(windows)]
use std::ffi::OsString;
use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};

use crate::{ControlledExecutable, DetectedTool};

/// Tool probes inspect controlled names without executing third-party programs. The first real
/// file on PATH provides capability evidence, avoiding one `which` or `where` subprocess for every
/// npm, Cargo, or Docker rule.
///
/// The returned boolean reports whether the inventory is complete. A missing PATH must not
/// invalidate application facts already collected; Core treats tool-dependent rules as unknown
/// and scans them conservatively.
pub(crate) fn detect_tools(names: &[&str]) -> (Vec<DetectedTool>, bool) {
    let Some(path_value) = env::var_os("PATH") else {
        return (Vec::new(), false);
    };
    let mut tools = Vec::new();
    let mut seen = HashSet::new();
    let mut rejected = HashSet::new();
    for directory in env::split_paths(&path_value) {
        for name in names {
            let normalized_name = (*name).to_ascii_lowercase();
            if seen.contains(&normalized_name) {
                continue;
            }
            for candidate in executable_candidates(&directory, name) {
                if candidate.is_file() {
                    match ControlledExecutable::capture(&candidate) {
                        Ok(executable) => {
                            tools.push(DetectedTool {
                                name: (*name).to_string(),
                                executable,
                            });
                            seen.insert(normalized_name.clone());
                            rejected.remove(&normalized_name);
                            break;
                        }
                        Err(error) => {
                            // A file can be replaced between PATH enumeration and identity capture.
                            // Continue with later candidates; if all fail, the caller marks the
                            // tool inventory incomplete.
                            log::warn!(
                                "tool_inventory_candidate_rejected tool_name={} reason={}",
                                name,
                                error.as_str()
                            );
                            rejected.insert(normalized_name.clone());
                        }
                    }
                }
            }
        }
    }
    (tools, rejected.is_empty())
}

fn executable_candidates(directory: &Path, name: &str) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![directory.join(name)]
    }
    #[cfg(target_os = "linux")]
    {
        vec![directory.join(name)]
    }
    #[cfg(windows)]
    {
        let path = Path::new(name);
        if path.extension().is_some() {
            return vec![directory.join(path)];
        }
        let extensions =
            env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"));
        extensions
            .to_string_lossy()
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(|extension| directory.join(format!("{name}{extension}")))
            .collect()
    }
}

pub(crate) fn normalize_fact(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
