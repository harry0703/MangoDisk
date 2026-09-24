// Windows release builds use the GUI subsystem. Development builds retain a
// console so native diagnostics remain visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
fn configure_linux_webview() {
    // The tested VMware SVGA X11 stack needs WebKit's software compositing
    // fallback. Preserve acceleration on other GPUs and any user override.
    if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none()
        && std::env::var_os("XDG_SESSION_TYPE")
            .is_some_and(|value| value.eq_ignore_ascii_case("x11"))
        && has_vmware_svga_adapter(std::path::Path::new("/sys/class/drm"))
    {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }
}

#[cfg(target_os = "linux")]
fn has_vmware_svga_adapter(drm_root: &std::path::Path) -> bool {
    let Ok(cards) = std::fs::read_dir(drm_root) else {
        return false;
    };
    cards.filter_map(Result::ok).any(|card| {
        let name = card.file_name();
        let name = name.to_string_lossy();
        let Some(number) = name.strip_prefix("card") else {
            return false;
        };
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        let device = card.path().join("device");
        std::fs::read_to_string(device.join("vendor")).is_ok_and(|value| value.trim() == "0x15ad")
            && std::fs::read_to_string(device.join("device"))
                .is_ok_and(|value| value.trim() == "0x0405")
    })
}

fn main() {
    #[cfg(target_os = "linux")]
    configure_linux_webview();

    #[cfg(windows)]
    if let Some(exit_code) = mangodisk_lib::run_layout_helper_mode(std::env::args_os()) {
        std::process::exit(exit_code);
    }
    #[cfg(windows)]
    if let Some(exit_code) = mangodisk_platform::run_elevation_helper_mode(std::env::args_os()) {
        std::process::exit(exit_code);
    }
    #[cfg(windows)]
    if let Some(exit_code) =
        mangodisk_platform::run_application_record_helper_mode(std::env::args_os())
    {
        std::process::exit(exit_code);
    }
    if let Some(exit_code) = mangodisk_platform::run_startup_helper_mode(std::env::args_os()) {
        std::process::exit(exit_code);
    }
    #[cfg(windows)]
    if let Some(exit_code) =
        mangodisk_platform::run_system_settings_helper_mode(std::env::args_os())
    {
        std::process::exit(exit_code);
    }
    #[cfg(windows)]
    if let Some(exit_code) =
        mangodisk_platform::run_system_maintenance_helper_mode(std::env::args_os())
    {
        std::process::exit(exit_code);
    }
    #[cfg(windows)]
    if let Some(exit_code) = mangodisk_platform::run_disk_cleanup_helper_mode(std::env::args_os()) {
        std::process::exit(exit_code);
    }
    mangodisk_lib::run();
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::has_vmware_svga_adapter;

    #[test]
    fn webview_fallback_only_matches_vmware_svga_cards() {
        let fixture =
            std::env::temp_dir().join(format!("mangodisk-webview-gpu-{}", std::process::id()));
        let card = fixture.join("card0/device");
        std::fs::create_dir_all(&card).expect("create GPU fixture");
        std::fs::write(card.join("vendor"), "0x15ad\n").expect("write vendor");
        std::fs::write(card.join("device"), "0x0405\n").expect("write device");
        assert!(has_vmware_svga_adapter(&fixture));
        std::fs::write(card.join("vendor"), "0x8086\n").expect("replace vendor");
        assert!(!has_vmware_svga_adapter(&fixture));
        std::fs::remove_dir_all(fixture).expect("remove GPU fixture");
    }
}
