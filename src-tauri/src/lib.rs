//! NoSQLBuddy — library root.
//!
//! Owns app entry, plugin registration, state management, command registration,
//! native menu, and the event loop. Command handlers live in [`commands`];
//! state in [`state`]; error types in [`error`]; event payloads in [`events`];
//! Mongo domain in [`mongo`].

pub mod audit;
pub mod commands;
pub mod error;
pub mod events;
pub mod mongo;
pub mod state;

use state::AppState;
use tauri::menu::{AboutMetadata, Menu, MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_log::{Target, TargetKind};

const APP_NAME: &str = "NoSQLBuddy";

/// Build the native application menu (macOS menu bar + Windows/Linux in-window).
fn build_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
    let app_meta = AboutMetadata {
        name: Some(APP_NAME.into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        short_version: Some(env!("CARGO_PKG_VERSION").into()),
        authors: Some(vec!["NoSQLBuddy".into()]),
        comments: Some("Cross-platform MongoDB management studio".into()),
        copyright: Some("(c) 2026 NoSQLBuddy".into()),
        website: Some("https://nosqlbuddy.studio".into()),
        ..Default::default()
    };
    let app_submenu = SubmenuBuilder::new(app, APP_NAME)
        .about(Some(app_meta))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let file_submenu = SubmenuBuilder::new(app, "File")
        .item(
            &MenuItem::with_id(app, "new_tab", "New Query Tab", true, Some("CmdOrCtrl+T"))?,
        )
        .item(
            &MenuItem::with_id(app, "close_tab", "Close Tab", true, Some("CmdOrCtrl+W"))?,
        )
        .separator()
        .item(
            &MenuItem::with_id(
                app,
                "new_connection",
                "New Connection…",
                true,
                Some("CmdOrCtrl+N"),
            )?,
        )
        .item(
            &MenuItem::with_id(
                app,
                "export_results",
                "Export Results…",
                true,
                Some("CmdOrCtrl+E"),
            )?,
        )
        .separator()
        .quit()
        .build()?;

    let edit_submenu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view_submenu = SubmenuBuilder::new(app, "View")
        .item(
            &MenuItem::with_id(
                app,
                "command_palette",
                "Command Palette",
                true,
                Some("CmdOrCtrl+K"),
            )?,
        )
        .item(
            &MenuItem::with_id(
                app,
                "toggle_tree",
                "Toggle Connection Tree",
                true,
                Some("CmdOrCtrl+B"),
            )?,
        )
        .separator()
        .fullscreen()
        .build()?;

    let help_submenu = SubmenuBuilder::new(app, "Help")
        .item(
            &MenuItem::with_id(app, "docs", "Documentation", true, None::<&str>)?,
        )
        .item(
            &MenuItem::with_id(
                app,
                "shortcuts",
                "Keyboard Shortcuts",
                true,
                Some("CmdOrCtrl+/"),
            )?,
        )
        .build()?;

    let menu = MenuBuilder::new(app)
        .item(&app_submenu)
        .item(&file_submenu)
        .item(&edit_submenu)
        .item(&view_submenu)
        .item(&help_submenu)
        .build()?;
    Ok(menu)
}

/// Entry point invoked by `main.rs`. Builds the Tauri app, registers plugins,
/// manages shared state, registers all IPC command handlers, installs the
/// native menu and tray, and runs the event loop.
pub fn run() {
    let log_level = if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Warn
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.unminimize();
            }
        }))
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log_level)
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                    Target::new(TargetKind::Webview),
                ])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::system::app_info,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::connections::list_profiles,
            commands::connections::save_profile,
            commands::connections::delete_profile,
            commands::connections::test_profile,
            commands::connections::open_connection,
            commands::connections::close_connection,
            commands::connections::list_active_connections,
            commands::connections::resolve_profile_uri,
            commands::mongo::list_databases,
            commands::mongo::list_collections,
            commands::mongo::collection_stats,
            commands::mongo::find_documents,
            commands::mongo::aggregate_documents,
            commands::mongo::count_documents,
            commands::mongo::list_indexes,
            commands::mongo::create_index,
            commands::mongo::drop_index,
            commands::mongo::index_stats,
            commands::mongo::explain_find,
            commands::mongo::explain_aggregate,
            commands::mongo::sample_schema,
            commands::mongo::insert_document,
            commands::mongo::update_documents,
            commands::mongo::delete_documents,
            commands::mongo::preview_delete,
            commands::mongo::preview_update,
            commands::mongo::translate_vqb,
            commands::sql::translate_sql,
            commands::driver_code::generate_pipeline_code,
            commands::shell::eval_shell,
            commands::shell::shell_autocomplete,
            audit::commands::audit_get_status,
            audit::commands::audit_list_events,
            audit::commands::audit_get_root,
            audit::commands::audit_generate_proof,
            audit::commands::audit_record_event,
        ])
        .setup(|app| {
            // Native menu (macOS menu bar + Windows/Linux in-window).
            let menu = build_menu(app.handle())?;
            app.set_menu(menu)?;
            app.on_menu_event(|app, event| {
                let _ = app.emit("menu-action", event.id().0.clone());
            });

            // Wire audit log persistence: replay any existing JSONL log
            // from <app_data_dir>/audit/events.jsonl and append to it on
            // every subsequent record(). This must happen before any
            // audit event is recorded (no commands run between manage()
            // and setup(), so the in-memory log is still empty here).
            // If the data dir can't be resolved or the log is corrupt,
            // we log the error and continue with a non-persistent log
            // rather than bricking the app — the user can still use the
            // tool, they just won't have audit history across restarts.
            let data_dir = app.path().app_data_dir();
            match &data_dir {
                Ok(dir) => {
                    if let Err(e) = app.state::<AppState>().audit_log.set_persistence_dir(dir) {
                        tracing::error!(
                            error = %e,
                            data_dir = %dir.display(),
                            "failed to initialize audit log persistence; audit events will not survive restart"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        "could not resolve app data dir; audit log persistence disabled"
                    );
                }
            }

            // System tray: quick "Show/Hide NoSQLBuddy" + Quit.
            let tray_menu = tauri::menu::MenuBuilder::new(app)
                .item(
                    &MenuItem::with_id(app, "show", "Show NoSQLBuddy", true, None::<&str>)?,
                )
                .item(
                    &MenuItem::with_id(app, "hide", "Hide NoSQLBuddy", true, None::<&str>)?,
                )
                .separator()
                .quit()
                .build()?;
            let _ = TrayIconBuilder::with_id("nosqlbuddy-tray")
                .icon(
                    app.default_window_icon()
                        .cloned()
                        .ok_or("missing default window icon")?,
                )
                .tooltip("NoSQLBuddy")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "hide" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    _ => {
                        let _ = app.emit("menu-action", event.id().0.clone());
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app);

            // Close-to-tray: clicking the close button on macOS hides the
            // window; the app stays alive in the tray until the user quits.
            if let Some(window) = app.get_webview_window("main") {
                let window_clone = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = window_clone.hide();
                    }
                });
            }

            // macOS Reopen: clicking the dock icon shows the main window.
            let app_handle = app.handle().clone();
            #[cfg(target_os = "macos")]
            {
                let _ = app_handle.clone();
            }
            let _ = app_handle;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .map_err(|e| {
            tracing::error!(error = %e, "tauri build failed");
            e
        })
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Reopen { .. } = event {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });
}
