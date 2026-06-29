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
        .item(&MenuItem::with_id(
            app,
            "new_tab",
            "New Query Tab",
            true,
            Some("CmdOrCtrl+T"),
        )?)
        .item(&MenuItem::with_id(
            app,
            "close_tab",
            "Close Tab",
            true,
            Some("CmdOrCtrl+W"),
        )?)
        .separator()
        .item(&MenuItem::with_id(
            app,
            "new_connection",
            "New Connection…",
            true,
            Some("CmdOrCtrl+N"),
        )?)
        .item(&MenuItem::with_id(
            app,
            "export_results",
            "Export Results…",
            true,
            Some("CmdOrCtrl+E"),
        )?)
        .item(&MenuItem::with_id(
            app,
            "import_data",
            "Import Data…",
            true,
            Some("CmdOrCtrl+I"),
        )?)
        .separator()
        .item(&MenuItem::with_id(
            app,
            "dump_database",
            "Dump Database…",
            true,
            Some("CmdOrCtrl+Shift+D"),
        )?)
        .item(&MenuItem::with_id(
            app,
            "restore_database",
            "Restore Database…",
            true,
            Some("CmdOrCtrl+Shift+R"),
        )?)
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
        .item(&MenuItem::with_id(
            app,
            "command_palette",
            "Command Palette",
            true,
            Some("CmdOrCtrl+K"),
        )?)
        .item(&MenuItem::with_id(
            app,
            "toggle_tree",
            "Toggle Connection Tree",
            true,
            Some("CmdOrCtrl+B"),
        )?)
        .separator()
        .fullscreen()
        .build()?;

    let help_submenu = SubmenuBuilder::new(app, "Help")
        .item(&MenuItem::with_id(
            app,
            "docs",
            "Documentation",
            true,
            None::<&str>,
        )?)
        .item(&MenuItem::with_id(
            app,
            "shortcuts",
            "Keyboard Shortcuts",
            true,
            Some("CmdOrCtrl+/"),
        )?)
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
            commands::data_model::scan_data_model,
            commands::data_model::get_data_model,
            commands::data_model::update_relationship,
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
            commands::mongo::find_page,
            commands::mongo::aggregate_documents,
            commands::mongo::aggregate_page,
            commands::mongo::count_documents,
            commands::mongo::list_indexes,
            commands::mongo::create_index,
            commands::mongo::drop_index,
            commands::mongo::index_stats,
            commands::mongo::explain_find,
            commands::mongo::explain_aggregate,
            commands::mongo::sample_schema,
            commands::mongo::sample_shape,
            commands::mongo::insert_document,
            commands::mongo::insert_many_documents,
            commands::mongo::update_documents,
            commands::mongo::replace_document,
            commands::mongo::delete_documents,
            commands::mongo::preview_delete,
            commands::mongo::preview_update,
            commands::mongo::safe_change_preview,

            commands::mongo::translate_vqb,
            commands::sql::translate_sql,
            commands::driver_code::generate_pipeline_code,
            commands::export::export_documents,
            commands::export::cancel_import_export,
            commands::export::copy_documents,
            commands::import::preview_import,
            commands::import::run_import,
            commands::dump::dump_database,
            commands::restore::preview_archive,
            commands::restore::restore_database,
            commands::jobs::list_jobs,
            commands::jobs::get_job,
            commands::jobs::cancel_job,
            commands::jobs::delete_job,
            commands::jobs::rerun_job,
            commands::jobs::update_schedule,
            commands::shell::eval_shell,
            commands::shell::shell_autocomplete,
            commands::timeline::list_timeline,
            commands::timeline::get_timeline_entry,
            commands::timeline::add_timeline_note,
            commands::timeline::delete_timeline_entry,
            commands::rollback::execute_rollback,
            audit::commands::audit_get_status,
            audit::commands::audit_list_events,
            audit::commands::audit_get_root,
            audit::commands::audit_generate_proof,
            audit::commands::audit_verify_proof_onchain,
            audit::commands::audit_record_event,
            audit::commands::audit_commit_root,
            audit::commands::audit_get_onchain_root,
            audit::commands::audit_list_epochs,
            audit::commands::audit_current_epoch,
            audit::commands::audit_close_epoch,
            audit::commands::audit_mark_epoch_committed,
            audit::commands::audit_reset_data,
            audit::commands::audit_list_domains,
            audit::commands::audit_get_domain_root,
            audit::commands::audit_get_domain_super_root,
            audit::commands::audit_generate_domain_proof,
            audit::commands::audit_generate_domain_super_proof,
            audit::commands::audit_set_legal_hold,
            audit::commands::audit_prune_domain,
            audit::commands::audit_verify_reader_mode,
            audit::commands::audit_list_verification_history,
            audit::commands::audit_publish_epoch_to_ipfs,
            audit::commands::audit_get_ipfs_cid,
            audit::commands::audit_check_ipfs_daemon,
            audit::commands::audit_get_onchain_root_rpc,
            audit::commands::audit_check_onboarding,
            audit::commands::audit_save_pinata_config,
            audit::commands::audit_test_pinata_connection,
            audit::commands::audit_generate_stellar_account,
            audit::commands::audit_check_replica_set,
            audit::commands::audit_commit_root_native,
            audit::commands::audit_publish_epoch_to_pinata,
            audit::commands::audit_add_publisher,
            audit::commands::audit_remove_publisher,
            audit::commands::audit_list_publishers,
            audit::commands::audit_set_attestation_threshold,
            audit::commands::audit_get_attestation_threshold,
            audit::commands::audit_submit_attestation,
            audit::commands::audit_list_attestations,
            audit::commands::audit_get_attestation_status,
            audit::commands::audit_verify_oplog_integrity,
            audit::commands::audit_get_oplog_commitment,
            audit::commands::audit_commit_root_production,
            // ─── Audit mode selection (dev / production) ───────────────
            audit::audit_mode::audit_get_mode_config,
            audit::audit_mode::audit_set_audit_mode,
            audit::audit_mode::audit_set_production_network,
            audit::audit_mode::audit_import_production_keypair,
            audit::audit_mode::audit_clear_production_keypair,
            audit::audit_mode::audit_get_active_account,
            // ─── Dev mode Docker orchestration ─────────────────────────
            audit::dev_stack::audit_check_dev_prerequisites,
            audit::dev_stack::audit_dev_stack_status,
            audit::dev_stack::audit_dev_stack_up,
            audit::dev_stack::audit_dev_stack_down,
            audit::dev_stack::audit_dev_stack_reset_data,
            audit::dev_stack::audit_dev_stack_logs,
            audit::dev_stack::audit_dev_stack_setup,
            // ─── Dev mode daemon HTTP proxy ────────────────────────────
            audit::audit_dev_proxy_get,
            audit::audit_dev_proxy_post,
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

                    // Wire epoch (batch) persistence to the same audit dir and
                    // reconcile the open batch's event count with the audit log
                    // that was just replayed. Without this the desktop app's
                    // batch counter resets to 0 on every launch while the audit
                    // log (and on-chain anchor) keep advancing.
                    let epoch_file = dir.join("audit").join("epochs.json");
                    let app_state = app.state::<AppState>();
                    if let Err(e) = app_state.epoch_manager.enable_persistence(&epoch_file) {
                        tracing::error!(
                            error = %e,
                            epoch_file = %epoch_file.display(),
                            "failed to initialize epoch persistence; batch state will reset on restart"
                        );
                    } else if let Err(e) = app_state
                        .epoch_manager
                        .sync_open_epoch_with_audit_log(&app_state.audit_log)
                    {
                        tracing::error!(
                            error = %e,
                            "failed to sync open epoch with audit log"
                        );
                    }

                    // Wire verification-history persistence to the same audit
                    // dir. Without this the reader-mode "tamper timeline"
                    // lived only in the frontend's React state and was lost on
                    // every app restart.
                    let verify_file =
                        dir.join("audit").join("verification_history.json");
                    if let Err(e) = app_state
                        .verification_store
                        .enable_persistence(&verify_file)
                    {
                        tracing::error!(
                            error = %e,
                            verify_file = %verify_file.display(),
                            "failed to initialize verification history persistence; verification timeline will reset on restart"
                        );
                    }

                    // Wire the attestation manager's sled store using
                    // the same sled DB path as the audit log. The
                    // attestation manager uses a separate tree within
                    // the same DB for publisher/attestation storage.
                    if let Some(sled_path) =
                        app.state::<AppState>().audit_log.sled_db_path()
                    {
                        match crate::audit::sled_store::SledTreeStore::open(&sled_path) {
                            Ok(store) => {
                                app.state::<AppState>()
                                    .attestation_manager
                                    .set_store(store);
                                tracing::info!("attestation manager sled store initialized");
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    "failed to open sled store for attestation manager"
                                );
                            }
                        }
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

            // Start the background job scheduler (checks every 60s for
            // scheduled dump/export jobs whose next_run_at has passed).
            // Use `async_runtime::spawn` so the scheduler runs inside
            // Tauri's tokio runtime rather than the main thread.
            let scheduler_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                crate::mongo::scheduler::start_scheduler(scheduler_handle);
            });

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
