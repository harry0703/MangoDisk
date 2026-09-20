#[cfg(windows)]
mod elevation;
#[cfg(windows)]
pub use elevation::run_elevation_helper_mode;
pub mod application_quit;
#[cfg(not(target_os = "linux"))]
mod browser_profile;
mod command;
mod contracts;
mod current;
pub mod diagnostics;
#[cfg(windows)]
mod disk_cleanup_helper;
mod file_icon;
mod inventory;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "linux"))]
mod startup_helper;
#[cfg(target_os = "linux")]
mod startup_helper {
    use std::ffi::OsString;

    const HELPER_FAILURE_EXIT_CODE: i32 = 70;

    /// Linux startup items belong to the user scope and do not need an elevated
    /// helper. Keep the protocol guard so mixed arguments never start Tauri.
    pub fn run_startup_helper_mode<I>(arguments: I) -> Option<i32>
    where
        I: IntoIterator<Item = OsString>,
    {
        let arguments = arguments.into_iter().collect::<Vec<_>>();
        if arguments
            .get(1)
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("--mangodisk-startup-helper-"))
        {
            return Some(HELPER_FAILURE_EXIT_CODE);
        }
        None
    }
}
#[cfg(windows)]
mod system_maintenance_helper;
pub mod system_resources;
#[cfg(windows)]
mod system_settings_helper;
#[cfg(not(target_os = "linux"))]
mod vscode_history;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use command::configure_background_process;
pub use command::{
    run_controlled_command, run_controlled_command_with_log_policy, ControlledCommandError,
    ControlledCommandLimits, ControlledCommandLogPolicy, ControlledCommandOutput,
    ControlledEnvironmentPolicy, ControlledExecutable,
};
pub use contracts::*;
pub use current::{application_directories, current_platform, CurrentPlatform};
#[cfg(windows)]
pub use disk_cleanup_helper::run_disk_cleanup_helper_mode;
pub use file_icon::{
    NativeFileIconAsset, NativeFileIconAssignment, NativeFileIconItemKind,
    NativeFileIconLoadResult, NativeFileIconMode, NativeFileIconRequest, NativeFileIconService,
};
pub use inventory::detect_git_executable;
#[cfg(target_os = "macos")]
pub use macos::{
    macos_privileged_application_removal_supported, remove_application_bundle_with_privileges,
};
pub use startup_helper::run_startup_helper_mode;
#[cfg(windows)]
pub use system_maintenance_helper::run_system_maintenance_helper_mode;
#[cfg(windows)]
pub use system_settings_helper::run_system_settings_helper_mode;
#[cfg(windows)]
pub use windows::{
    estimate_windows_previous_installations_with_privileges, execute_windows_disk_cleanup,
    execute_windows_previous_installations_with_privileges, fresh_windows_disk_cleanup_estimates,
    run_application_record_helper_mode, windows_disk_cleanup_estimates,
};

#[cfg(test)]
mod startup_baseline_tests {
    use std::collections::BTreeSet;

    use super::{current_platform, PlatformCancellation, StartupPlatform};

    #[test]
    #[ignore = "requires the host startup configuration"]
    fn actual_startup_source_baseline_has_unique_source_ids() {
        let cancellation = PlatformCancellation::new(|| false);
        let results = current_platform()
            .scan_startup_sources(&cancellation)
            .expect("the host startup scan should return a catalog");
        let mut source_ids = BTreeSet::new();
        for source in results {
            println!(
                "source_id={} status={:?} item_count={} elapsed_ms={}",
                source.source_id,
                source.status,
                source.items.len(),
                source.elapsed_ms
            );
            assert!(
                source_ids.insert(source.source_id),
                "startup source identifiers must be unique"
            );
        }
        assert!(
            !source_ids.is_empty(),
            "at least one source must be reported"
        );
    }
}

// Exercise the pure Windows command grammar on development hosts without simulating native APIs.
#[cfg(all(test, not(windows)))]
#[path = "windows/native_uninstall/command.rs"]
mod windows_uninstall_command_tests;

#[cfg(all(test, not(windows)))]
#[path = "windows/shortcut_overlay/icon.rs"]
mod windows_shortcut_icon_tests;
