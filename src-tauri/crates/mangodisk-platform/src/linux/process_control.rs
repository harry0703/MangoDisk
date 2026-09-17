use crate::{
    ApplicationProcessCloseMode, ApplicationProcessCloseResult, ApplicationProcessTarget,
    PlatformResult,
};

pub(crate) fn close(
    target: &ApplicationProcessTarget,
    mode: ApplicationProcessCloseMode,
) -> PlatformResult<ApplicationProcessCloseResult> {
    let signal = match mode {
        ApplicationProcessCloseMode::Graceful => libc::SIGTERM,
        ApplicationProcessCloseMode::Force => libc::SIGKILL,
    };

    let mut matched = 0_u64;
    let mut remaining = Vec::new();

    // Try to find and kill processes by name from /proc
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let comm_path = entry.path().join("comm");
            if let Ok(comm) = std::fs::read_to_string(&comm_path) {
                let comm = comm.trim().to_string();
                if target.executable_names.contains(&comm) {
                    let pid: i32 = match name_str.parse() {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    // Skip our own process
                    if pid == std::process::id() as i32 {
                        continue;
                    }
                    let result = unsafe { libc::kill(pid, signal) };
                    if result == 0 {
                        matched += 1;
                    } else {
                        remaining.push(comm);
                    }
                }
            }
        }
    }

    Ok(ApplicationProcessCloseResult {
        matched_process_count: matched,
        requested_process_count: target.executable_names.len() as u64,
        remaining_processes: remaining,
    })
}
