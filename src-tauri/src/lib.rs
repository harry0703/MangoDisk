#[cfg(target_os = "macos")]
mod application_menu;
mod commands;
mod events;
mod resident;
#[cfg(windows)]
pub use resident::taskbar_display::run_layout_helper_mode;
mod services;
mod webview_runtime;

use log::LevelFilter;
use mangodisk_core::{configure_application_paths, ApplicationPaths};
use services::application_uninstall_catalog::ApplicationUninstallCatalogCache;
use services::feedback::FeedbackDraftStore;
use tauri::{LogicalSize, Manager};
use tauri_plugin_log::RotationStrategy;
use tauri_plugin_window_state::{StateFlags, WindowExt};

const MAIN_WINDOW_LABEL: &str = "main";
const LOG_FILE_MAX_BYTES: u128 = 10 * 1024 * 1024;
// KeepSome only counts dated archives and excludes the active log file. Four
// archives plus the active file keep the five most recent logs requested by
// the product without silently retaining a sixth file.
const LOG_ARCHIVE_FILE_COUNT: usize = 4;

fn configure_core_storage(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let local_data_directory = app.path().app_local_data_dir()?;
    let cache_directory = app.path().app_cache_dir()?;
    let paths = ApplicationPaths::from_base_directories(local_data_directory, cache_directory)?;
    configure_application_paths(paths)?;
    Ok(())
}

fn restored_size_is_below_minimum(
    width: f64,
    height: f64,
    min_width: Option<f64>,
    min_height: Option<f64>,
) -> bool {
    min_width.is_some_and(|minimum| width < minimum)
        || min_height.is_some_and(|minimum| height < minimum)
}

fn restore_main_window_state(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        log::warn!("window_state_restore_skipped reason=main_window_missing");
        return;
    };

    let state_flags = StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED;
    if let Err(error) = window.restore_state(state_flags) {
        // A damaged state file must never prevent the application from opening
        // with the dimensions declared in tauri.conf.json.
        log::warn!("window_state_restore_failed label={MAIN_WINDOW_LABEL} error={error}");
    }

    let Some(config) = app
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == MAIN_WINDOW_LABEL)
    else {
        log::warn!("window_state_validation_skipped reason=main_window_config_missing");
        return;
    };

    let scale_factor = match window.scale_factor() {
        Ok(scale_factor) => scale_factor,
        Err(error) => {
            log::warn!(
                "window_state_validation_skipped reason=scale_factor_unavailable error={error}"
            );
            return;
        }
    };
    let physical_size = match window.inner_size() {
        Ok(size) => size,
        Err(error) => {
            log::warn!(
                "window_state_validation_skipped reason=window_size_unavailable error={error}"
            );
            return;
        }
    };
    let logical_size = physical_size.to_logical::<f64>(scale_factor);

    if !restored_size_is_below_minimum(
        logical_size.width,
        logical_size.height,
        config.min_width,
        config.min_height,
    ) {
        return;
    }

    let reset_width = config.min_width.unwrap_or(logical_size.width);
    let reset_height = config.min_height.unwrap_or(logical_size.height);
    log::warn!(
        "window_state_size_recovered label={MAIN_WINDOW_LABEL} restored_width={} restored_height={} minimum_width={} minimum_height={}",
        logical_size.width,
        logical_size.height,
        reset_width,
        reset_height
    );

    // Window-state stores physical pixels, while Tauri's configured minimum
    // uses logical pixels. A stale state created with another scale factor can
    // therefore bypass the native minimum during startup. Reset both
    // dimensions together and center the window so recovery is predictable.
    if let Err(error) = window.set_size(LogicalSize::new(reset_width, reset_height)) {
        log::warn!("window_state_size_recovery_failed label={MAIN_WINDOW_LABEL} error={error}");
        return;
    }
    if let Err(error) = window.center() {
        log::warn!("window_state_center_recovery_failed label={MAIN_WINDOW_LABEL} error={error}");
    }
}

pub fn run() {
    let webview_version = tauri::webview_version();
    let webview_update_required = cfg!(target_os = "windows")
        && webview_runtime::requires_update(
            webview_version.as_deref().ok(),
            Some(webview_runtime::MINIMUM_VERSION),
        );
    // This plugin must be registered before every other plugin so a secondary
    // process exits before it can initialize application services.
    let builder = tauri::Builder::default().plugin(tauri_plugin_single_instance::init(
        move |app, args, _| {
            // A second launch must not bypass the native compatibility prompt.
            if webview_update_required {
                return;
            }
            if !resident::main_window::is_background_launch(args) {
                resident::main_window::request(
                    app,
                    resident::main_window::Destination::Main,
                    "manual_relaunch",
                );
            }
        },
    ));
    #[cfg(target_os = "macos")]
    let builder = builder
        .menu(application_menu::build)
        .on_menu_event(application_menu::handle);
    let builder = builder.on_window_event(resident::handle_window_event);
    // Release builds must not expose the WebView's browser context menu or
    // browser-only shortcuts. Debug builds retain them for inspection.
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_prevent_default::init());
    let app = builder
        .manage(resident::main_window::MainWindowState::default())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![resident::main_window::BACKGROUND_ARGUMENT]),
        ))
        .manage(ApplicationUninstallCatalogCache::default())
        .manage(commands::ai::AiRuntime::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(LevelFilter::Info)
                .max_file_size(LOG_FILE_MAX_BYTES)
                .rotation_strategy(RotationStrategy::KeepSome(LOG_ARCHIVE_FILE_COUNT))
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // Visibility remains controlled by the native readiness handshake so
                // restoring state never exposes an unrendered WebView.
                // Decorations are static application configuration.
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                // Restore the main window when it is created, before the readiness handshake.
                .skip_initial_state(MAIN_WINDOW_LABEL)
                .with_filter(|label| label == MAIN_WINDOW_LABEL)
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::resident::monitoring_get_reading,
            commands::resident::monitoring_refresh,
            commands::resident::monitoring_release_memory,
            commands::resident::memory_release_preferences,
            commands::resident::memory_release_save_preferences,
            commands::resident::memory_release_applications,
            commands::resident::memory_release_open_settings,
            commands::resident::monitoring_quit_application,
            commands::resident::resident_get_preferences,
            commands::resident::resident_get_display_status,
            commands::resident::resident_get_catalogue,
            commands::resident::resident_get_panel_metric,
            commands::resident::resident_select_metric,
            commands::resident::resident_save_preferences,
            commands::resident::resident_open_panel,
            commands::resident::resident_panel_ready,
            commands::resident::resident_hide_panel,
            commands::resident::resident_open_main,
            commands::resident::resident_quit,
            commands::resident::resident_main_ready,
            commands::resident::resident_get_autostart,
            commands::resident::resident_set_autostart,
            commands::ai::ai_get_preferences,
            commands::ai::ai_set_enabled,
            commands::ai::ai_get_settings,
            commands::ai::ai_get_configuration,
            commands::ai::ai_save_settings,
            commands::ai::ai_delete_settings,
            commands::ai::ai_begin,
            commands::ai::ai_cancel,
            commands::ai::ai_explain,
            commands::ai::ai_get_quota,
            commands::app_distribution::get_app_distribution,
            commands::app_updates::get_app_update_notice,
            commands::app_updates::acquire_app_update,
            commands::applications::prepare_application_uninstall_batch,
            commands::applications::execute_application_uninstall_batch,
            commands::applications::cancel_application_uninstall_execution,
            commands::applications::close_application_uninstall_applications,
            commands::applications::get_application_icons,
            commands::file_icons::get_file_icons,
            commands::applications::scan_application_leftovers,
            commands::applications::scan_application_uninstall_catalog,
            commands::applications::open_windows_installed_apps,
            commands::applications::remove_application_record,
            commands::applications::log_application_uninstall_details,
            commands::applications::cancel_application_uninstall_catalog_scan,
            commands::applications::execute_application_leftovers,
            commands::applications::cancel_application_leftovers,
            commands::disk::get_system_disk,
            commands::disk::list_disks,
            commands::cleanup::scan_cleanup_candidates,
            commands::cleanup::scan_windows_previous_installations_with_privileges,
            commands::cleanup::cancel_cleanup_scan,
            commands::cleanup::cancel_cleanup_execution,
            commands::cleanup::close_cleanup_applications,
            commands::cleanup::execute_cleanup,
            commands::analysis::cancel_analysis,
            commands::analysis::analyze_path,
            commands::large_files::cancel_large_files,
            commands::large_files::filter_large_files,
            commands::large_files::find_large_files,
            commands::duplicate_files::cancel_duplicate_files,
            commands::duplicate_files::find_duplicate_files,
            commands::duplicate_files::get_duplicate_file_groups,
            commands::duplicate_files::delete_duplicate_files_permanently,
            commands::permanent_delete::delete_files_permanently,
            commands::permanent_delete::delete_analysis_entry_permanently,
            commands::startup::scan_startup_catalog,
            commands::startup::cancel_startup_catalog_scan,
            commands::startup::cancel_startup_change,
            commands::startup::prepare_startup_change,
            commands::startup::execute_startup_change,
            commands::privacy::scan_privacy,
            commands::privacy::cancel_privacy_scan,
            commands::privacy::get_privacy_details,
            commands::privacy::prepare_privacy_execution,
            commands::privacy::close_privacy_browsers,
            commands::privacy::refresh_privacy_browser_status,
            commands::privacy::execute_privacy,
            commands::privacy::cancel_privacy_execution,
            commands::file_manager::open_analysis_entry,
            commands::file_manager::open_large_file_entry,
            commands::file_manager::open_duplicate_file_entry,
            commands::file_manager::reveal_in_file_manager,
            commands::file_manager::open_application_log_directory,
            commands::folder_selection::filter_directory_paths,
            commands::feedback::stage_feedback_attachment,
            commands::feedback::discard_feedback_attachments,
            commands::feedback::submit_feedback,
            commands::history::list_history,
            commands::history::clear_history,
            commands::system_settings::open_privacy_settings,
            commands::system_settings::open_macos_login_items_settings,
            commands::system_settings::open_windows_startup_tool,
            commands::system_settings::scan_system_settings,
            commands::system_settings::cancel_system_settings_scan,
            commands::system_settings::prepare_system_settings_change,
            commands::system_settings::execute_system_settings_change,
            commands::system_maintenance::scan_system_maintenance,
            commands::system_maintenance::cancel_system_maintenance_scan,
            commands::system_maintenance::execute_system_maintenance,
            commands::system_maintenance::cancel_system_maintenance_execution,
            commands::system_maintenance::get_system_maintenance_runtime,
        ])
        .setup(move |app| {
            log::info!(
                "application_started version={} distribution={}",
                app.package_info().version,
                commands::app_distribution::current().diagnostic_name()
            );
            // Tauri reports the available WebView2 runtime on Windows and the
            // system WebKit bundle build on macOS, not the Safari app version.
            // Read once per launch; missing diagnostics must never block startup.
            let webview_engine = if cfg!(target_os = "windows") {
                "webview2"
            } else {
                "webkit"
            };
            match webview_version {
                Ok(version) => log::info!(
                    "webview_runtime_version platform={} engine={} version={}",
                    std::env::consts::OS,
                    webview_engine,
                    version
                ),
                Err(error) => log::warn!(
                    "webview_runtime_version_failed platform={} engine={} error={}",
                    std::env::consts::OS,
                    webview_engine,
                    mangodisk_platform::diagnostics::text(&error)
                ),
            }
            #[cfg(target_os = "windows")]
            if webview_update_required {
                log::warn!(
                    "webview_runtime_update_required minimum={}",
                    webview_runtime::MINIMUM_VERSION
                );
                webview_runtime::show_update_prompt(app.handle());
                return Ok(());
            }
            configure_core_storage(app)?;
            resident::install(app.handle())?;
            services::app_updates::start(app.handle());
            let feedback_store = FeedbackDraftStore::initialize(&app.path().app_cache_dir()?);
            let feedback_cleanup_store = feedback_store.clone();
            app.manage(feedback_store);
            tauri::async_runtime::spawn_blocking(move || {
                feedback_cleanup_store.cleanup_stale_drafts();
            });
            let login_launch = resident::main_window::is_background_launch(std::env::args());
            let resident_enabled = app
                .state::<std::sync::Arc<resident::runtime::ResidentState>>()
                .enabled();
            if resident::main_window::start_hidden(login_launch, resident_enabled) {
                #[cfg(target_os = "macos")]
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                log::info!("background_launch_ready");
            } else {
                // Login startup remains useful without a tray: open the main UI
                // instead of silently exiting or changing the OS login setting.
                resident::main_window::open(
                    app.handle(),
                    resident::main_window::Destination::Main,
                    if login_launch {
                        "login_launch"
                    } else {
                        "manual_launch"
                    },
                )?;
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("MangoDisk failed to start");
    app.run(move |_app, _event| {
        if !webview_update_required && matches!(_event, tauri::RunEvent::Ready) {
            resident::panel::prewarm(_app);
        }
        // Dock reopening does not launch another process, so the single-instance
        // callback alone cannot restore a hidden or minimized macOS window.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = _event {
            log::info!("main_window_reopen_requested");
            resident::main_window::request(
                _app,
                resident::main_window::Destination::Main,
                "macos_reopen",
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::restored_size_is_below_minimum;

    #[test]
    fn restored_window_size_rejects_either_dimension_below_minimum() {
        assert!(restored_size_is_below_minimum(
            999.0,
            700.0,
            Some(1000.0),
            Some(700.0)
        ));
        assert!(restored_size_is_below_minimum(
            1000.0,
            699.0,
            Some(1000.0),
            Some(700.0)
        ));
    }

    #[test]
    fn restored_window_size_accepts_configured_minimum() {
        assert!(!restored_size_is_below_minimum(
            1000.0,
            700.0,
            Some(1000.0),
            Some(700.0)
        ));
    }
}
