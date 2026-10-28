//! Fullscreen classification is independent of the monitor strip's placement.
use super::layout::Bounds;
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Disabled,
    GeometryUnavailable,
    AutoHidden,
    ShellMoving,
    Fullscreen,
    UnsupportedLayout,
    NoSpace,
    ShellUnavailable,
    PaintFailed,
    PositionFailed,
}

/// Browsers can retain WS_MAXIMIZE during frameless fullscreen, while WPS can
/// retain resize styles without being maximized. Accept the latter only when
/// its outer bounds exactly match the monitor: ordinary maximized frames can
/// extend beyond the screen when the taskbar auto-hides. Neither style nor
/// maximization alone identifies fullscreen, and covering only the strip is not
/// enough. Desktop hosts are excluded by the native caller before this check.
pub fn is_fullscreen(window: Bounds, monitor: Bounds, has_frame: bool, maximized: bool) -> bool {
    monitor.width() > 0
        && monitor.height() > 0
        && monitor.fits_in(window)
        && (!has_frame || (!maximized && window == monitor))
}

/// Explorer can focus WorkerW instead of the Progman handle returned by
/// GetShellWindow. Both desktop hosts are frameless and cover the monitor,
/// but neither represents application fullscreen. Check the shell process too:
/// a similarly named window from another application must not bypass detection.
pub fn is_shell_desktop(class: &str, process_id: u32, shell_process_id: u32) -> bool {
    process_id != 0 && process_id == shell_process_id && matches!(class, "WorkerW" | "Progman")
}

/// Windows 11 taskbar thumbnails use a full-monitor XAML host even though only
/// the preview cards are drawn. Require ownership by the current taskbar process;
/// another application's identically named fullscreen window is not a shell UI.
pub fn is_shell_preview(class: &str, process_id: u32, shell_process_id: u32) -> bool {
    class == "XamlExplorerHostIslandWindow" && process_id != 0 && process_id == shell_process_id
}

/// Start/search animations briefly report a full-monitor CoreWindow. Only
/// the system-installed shell hosts are exempt; arbitrary UWP fullscreen windows
/// share this class and must continue to hide the monitor strip.
pub fn is_shell_flyout(class: &str, image_name: &str, system_app: bool) -> bool {
    system_app
        && class == "Windows.UI.Core.CoreWindow"
        && ["StartMenuExperienceHost.exe", "SearchHost.exe"]
            .iter()
            .any(|name| image_name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_animation_exemption_does_not_cover_other_uwp_apps() {
        assert!(is_shell_flyout(
            "Windows.UI.Core.CoreWindow",
            "StartMenuExperienceHost.exe",
            true
        ));
        assert!(is_shell_flyout(
            "Windows.UI.Core.CoreWindow",
            "SearchHost.exe",
            true
        ));
        assert!(!is_shell_flyout(
            "Windows.UI.Core.CoreWindow",
            "SearchHost.exe",
            false
        ));
        assert!(!is_shell_flyout(
            "Windows.UI.Core.CoreWindow",
            "VideoPlayer.exe",
            true
        ));
        assert!(!is_shell_flyout(
            "Windows.UI.Core.CoreWindow",
            "StartMenuExperienceHost.exe",
            false
        ));
        assert!(!is_shell_flyout(
            "Chrome_WidgetWin_1",
            "StartMenuExperienceHost.exe",
            true
        ));
    }

    #[test]
    fn both_explorer_desktop_hosts_are_excluded_from_fullscreen() {
        assert!(is_shell_desktop("WorkerW", 42, 42));
        assert!(is_shell_desktop("Progman", 42, 42));
    }

    #[test]
    fn other_windows_and_unknown_processes_are_not_desktop_hosts() {
        assert!(!is_shell_desktop("Chrome_WidgetWin_1", 42, 42));
        assert!(!is_shell_desktop("CabinetWClass", 42, 42));
        assert!(!is_shell_desktop("WorkerW", 7, 42));
        assert!(!is_shell_desktop("Progman", 0, 0));
    }

    #[test]
    fn full_monitor_taskbar_preview_requires_current_explorer_ownership() {
        assert!(is_fullscreen(MONITOR, MONITOR, false, false));
        assert!(is_shell_preview("XamlExplorerHostIslandWindow", 42, 42));
        assert!(!is_shell_preview("XamlExplorerHostIslandWindow", 7, 42));
        assert!(!is_shell_preview("XamlExplorerHostIslandWindow", 0, 0));
        assert!(!is_shell_preview("XamlExplorerHostIslandWindow", 42, 0));
        assert!(!is_shell_preview("Chrome_WidgetWin_1", 42, 42));
        assert!(!is_shell_preview("CabinetWClass", 42, 42));
    }

    const MONITOR: Bounds = Bounds {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };

    #[test]
    fn fullscreen_does_not_depend_on_previous_maximized_state() {
        assert!(is_fullscreen(MONITOR, MONITOR, false, true));
        assert!(is_fullscreen(MONITOR, MONITOR, false, false));
        // Restoring the browser frame must restore the strip even if Explorer's
        // work-area geometry has not yet caught up with the fullscreen transition.
        assert!(!is_fullscreen(MONITOR, MONITOR, true, true));
    }

    #[test]
    fn fullscreen_can_retain_resize_style_without_being_maximized() {
        // WPS keeps WS_THICKFRAME while sizing its non-maximized fullscreen
        // window exactly to the monitor. Style presence alone must not reject it.
        assert!(is_fullscreen(MONITOR, MONITOR, true, false));
        assert!(!is_fullscreen(MONITOR, MONITOR, true, true));
        assert!(is_fullscreen(MONITOR, MONITOR, false, true));
        assert!(!is_fullscreen(
            Bounds {
                right: 1919,
                ..MONITOR
            },
            MONITOR,
            true,
            false
        ));
        assert!(!is_fullscreen(
            Bounds {
                left: -8,
                right: 1928,
                ..MONITOR
            },
            MONITOR,
            true,
            false
        ));
    }

    #[test]
    fn maximized_auto_hide_window_is_not_fullscreen() {
        let window = Bounds {
            left: -8,
            top: -8,
            right: 1928,
            bottom: 1088,
        };
        assert!(!is_fullscreen(window, MONITOR, true, true));
    }

    #[test]
    fn covering_only_the_taskbar_does_not_hide_the_strip() {
        let window = Bounds {
            top: 500,
            ..MONITOR
        };
        assert!(!is_fullscreen(window, MONITOR, false, false));
    }

    #[test]
    fn fullscreen_on_another_monitor_does_not_hide_the_strip() {
        let secondary = Bounds {
            left: -1920,
            right: 0,
            ..MONITOR
        };
        assert!(!is_fullscreen(secondary, MONITOR, false, false));
        assert!(is_fullscreen(secondary, secondary, false, false));
        assert!(!is_fullscreen(MONITOR, Bounds::default(), false, false));
    }
}
