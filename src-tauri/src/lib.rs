mod app;
mod commands;
mod error;
mod events;
mod models;
mod services;
mod state;
mod transfer;
mod utils;
mod websocket;

use axum::extract::DefaultBodyLimit;
use services::local_transfer_file_service::LocalTransferFileService;
use sqlx::sqlite::SqlitePoolOptions;

use std::sync::Arc;

use app::AppState;

use tauri_plugin_log::{log, Builder as LogBuilder, Target, TargetKind};

use commands::auth::{
    cancel_desktop_auth, clear_all, delete_tunnel_hostname, delete_tunnel_token, get_auth_status,
    get_auth_token, get_connection_status, get_default_download_location, get_tunnel_hostname,
    get_tunnel_token, logout, save_tunnel_hostname, save_tunnel_token, send_message,
    set_default_download_location, start_desktop_auth, start_websocket,
};

use commands::transfer_commands::{
    cancel_transfer, get_transfer_status, pause_transfer, resume_transfer, start_transfer,
};

#[cfg(feature = "enable-updater")]
use services::updates_service::handle_pending_update;

use commands::local_transfer_commands::{
    check_local_transfer_exists, delete_local_transfer_file, delete_local_transfer_files,
    get_local_transfer_files, save_local_transfer_file,
};

use commands::cloudflared::{cloudflared_status, start_cloudflared_cmd, stop_cloudflared_cmd};
use commands::device::{create_device_identity, detect_device_type};

use state::cloudflared_state::Cloudflared;
use tauri_plugin_stronghold::Builder;

use tauri::Manager;

use utils::{config::AppConfig, logger::Logger};

use crate::services::KeyringService;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Logger::init();

    /*
     * The single-instance plugin must be registered before any other plugin.
     *
     * On Windows and Linux a `vilsend://` deep link is delivered by launching
     * the executable with the URL as an argument. Without this, completing
     * sign-in in the browser would spawn a second copy of the app instead of
     * completing the flow in the instance that started it. The `deep-link`
     * feature forwards the URL on to the deep-link plugin.
     */
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let builder =
        tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }));

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let builder = tauri::Builder::default();

    builder
        // .plugin(
        //     LogBuilder::default()
        //         .level(log::LevelFilter::Info)
        //         .targets([
        //             Target::new(TargetKind::LogDir {
        //                 file_name: Some("app".into()),
        //             }),
        //             Target::new(TargetKind::Stdout),
        //         ])
        //         .build(),
        // )
        .plugin(tauri_plugin_deep_link::init())
        .plugin(
            LogBuilder::default()
                .level(log::LevelFilter::Trace)
                .targets([
                    // Log file
                    Target::new(TargetKind::LogDir {
                        file_name: Some("app".into()),
                    }),
                    // Console (cargo tauri dev)
                    Target::new(TargetKind::Stdout),
                    // Errors
                    // Target::new(TargetKind::Stderr),
                    // Frontend console (devtools)
                    // Target::new(TargetKind::Webview),
                ])
                .build(),
        )
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_secure_storage::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(Builder::new(|_| b"vilSend-strongHold-password".to_vec()).build())
        .setup(|app| {
            // tracing::info!("Executable: {:?}", app.path().executable()?);
            /*
             * Application config
             */

            // let window = app
            //     .get_webview_window("main")
            //     .expect("main window not found");

            // window.open_devtools();

            let config = if cfg!(debug_assertions) {
                AppConfig::development()
            } else {
                AppConfig::production()
            };
            let app_data_dir = app.path().app_data_dir()?;

            std::fs::create_dir_all(&app_data_dir)?;

            let database_path = app_data_dir.join("transfer.db");

            let database_url = format!("sqlite:{}?mode=rwc", database_path.to_string_lossy());

            let pool = tauri::async_runtime::block_on(async {
                SqlitePoolOptions::new()
                    .max_connections(5)
                    .connect(&database_url)
                    .await
                    .expect("Failed to connect sqlite database")
            });
            let local_transfer_service = Arc::new(tauri::async_runtime::block_on(async {
                LocalTransferFileService::new(pool).await
            }));
            let (app_state, rx) =
                AppState::new(app.handle().clone(), config, local_transfer_service.clone());

            let app_state = Arc::new(app_state);

            /*
             * Resume the desktop session from secure storage.
             *
             * This runs to completion before the webview can call
             * `get_auth_status`, so a user who has already signed in opens
             * straight into the app rather than being shown the sign-in screen
             * on every launch. It reads the local keychain only — no network —
             * so a slow or absent connection cannot delay startup.
             */
            {
                let restored =
                    tauri::async_runtime::block_on(app_state.oauth_service.restore());

                tracing::info!(
                    target: "auth",
                    event = "session_restore_checked",
                    restored,
                    "checked secure storage for a previous desktop session"
                );
            }

            #[cfg(feature = "enable-updater")]
            {
                let update_app = app.handle().clone();
                let dispatcher = Arc::clone(&app_state.event_dispatcher);

                tauri::async_runtime::spawn(async move {
                    handle_pending_update(update_app, dispatcher).await;
                });
            }

            let dispatcher2 = Arc::clone(&app_state.event_dispatcher);

            /*
             * Local SQLite storage
             */

            /*
             * Application state
             */
            // let app_state = Arc::new(AppState::new(
            //     app.handle().clone(),
            //     config,
            //     local_transfer_service.clone(),
            // ));

            /*
             * Start transfer event listener
             */

            tauri::async_runtime::spawn(async move {
                dispatcher2.listen_transfer_events(rx).await;
            });

            app.manage(app_state);

            /*
             * Make SQLite service globally available
             */
            app.manage(local_transfer_service.clone());

            /*
             * Cloudflared state
             */
            app.manage(Cloudflared::new());

            /*
             * Desktop OAuth: `vilsend://auth/callback`
             *
             * Registered only after `app.manage(app_state)` above, because the
             * callback handler resolves `AppState` from the app handle. The
             * listener runs on the multithreaded async runtime, so registering
             * it earlier would allow a callback to race setup and panic on an
             * unmanaged state.
             *
             * The browser redirects to the registered custom scheme; the OS
             * hands the URL to this process, and Rust completes the token
             * exchange. The webview never sees the authorization code.
             */
            {
                use tauri_plugin_deep_link::DeepLinkExt;

                let handle = app.handle().clone();

                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        let handle = handle.clone();

                        tauri::async_runtime::spawn(async move {
                            commands::auth::handle_auth_callback(handle, url).await;
                        });
                    }
                });

                // Launch-time callback, e.g. the app was closed while the user
                // was in the browser. The in-memory PKCE verifier is gone in
                // that case, so the callback fails cleanly and the UI asks the
                // user to try again rather than hanging.
                match app.deep_link().get_current() {
                    Ok(Some(urls)) => {
                        let handle = app.handle().clone();

                        for url in urls {
                            let handle = handle.clone();

                            tauri::async_runtime::spawn(async move {
                                commands::auth::handle_auth_callback(handle, url).await;
                            });
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(
                            target: "auth",
                            %error,
                            "could not read the launch deep link"
                        );
                    }
                }

                // In development the app is not installed, so Windows and Linux
                // have no registry entry for the scheme. Register it so
                // `npm run tauri dev` can receive callbacks; in a bundled build
                // the installer writes the entry instead.
                #[cfg(all(debug_assertions, any(target_os = "windows", target_os = "linux")))]
                if let Err(error) = app.deep_link().register_all() {
                    tracing::warn!(
                        target: "auth",
                        %error,
                        "could not register the vilsend:// scheme for development"
                    );
                }
            }

            /*
             * Transfer storage
             */
            let transfer_root = app_data_dir.join("transfers");

            /*
             * Local receiver server
             */
            let receiver_state =
                transfer::writer::ReceiverState::new(app.handle().clone(), transfer_root);

            tauri::async_runtime::spawn(async move {
                let address =
                    std::net::SocketAddr::from(([0, 0, 0, 0], transfer::constants::RECEIVER_PORT));

                match tokio::net::TcpListener::bind(address).await {
                    Ok(listener) => {
                        tracing::info!("Transfer receiver started");

                        let router = axum::Router::new()
                            .route(
                                "/transfer/public-key",
                                axum::routing::get(transfer::writer::get_public_key),
                            )
                            .route(
                                "/transfer/start",
                                axum::routing::post(transfer::writer::start_transfer),
                            )
                            .route(
                                "/transfer/chunk",
                                axum::routing::post(transfer::writer::receive),
                            )
                            .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
                            .with_state(receiver_state);

                        if let Err(error) = axum::serve(listener, router).await {
                            tracing::error!(
                                %error,
                                "transfer receiver stopped"
                            );
                        }
                    }

                    Err(error) => {
                        tracing::error!(
                            %error,
                            "could not start transfer receiver"
                        );
                    }
                }
            });

            /*
             * Debug: Print stored device identity
             */
            // match KeyringService::get_device_public_key(app.handle()) {
            //     Ok(public_key) => {
            //         tracing::info!(
            //             device_public_key = %public_key,
            //             "Loaded device public key"
            //         );
            //     }
            //     Err(err) => {
            //         tracing::warn!(
            //             error = %err,
            //             "Device public key not found"
            //         );
            //     }
            // }

            // match KeyringService::get_device_private_key(app.handle()) {
            //     Ok(private_key) => {
            //         tracing::info!(
            //             device_private_key = %private_key,
            //             "Loaded device private key"
            //         );
            //     }
            //     Err(err) => {
            //         tracing::warn!(
            //             error = %err,
            //             "Device private key not found"
            //         );
            //     }
            // }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_desktop_auth,
            cancel_desktop_auth,
            get_auth_status,
            get_auth_token,
            logout,
            start_websocket,
            send_message,
            get_connection_status,
            save_tunnel_token,
            get_tunnel_token,
            delete_tunnel_token,
            get_default_download_location,
            set_default_download_location,
            start_transfer,
            pause_transfer,
            resume_transfer,
            cancel_transfer,
            get_transfer_status,
            save_tunnel_hostname,
            get_tunnel_hostname,
            delete_tunnel_hostname,
            start_cloudflared_cmd,
            stop_cloudflared_cmd,
            cloudflared_status,
            clear_all,
            create_device_identity,
            save_local_transfer_file,
            get_local_transfer_files,
            delete_local_transfer_files,
            delete_local_transfer_file,
            check_local_transfer_exists,
            detect_device_type
        ])
        // .run(tauri::generate_context!())
        // .expect("error while running tauri application");
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<Cloudflared>();

                {
                    let mut guard = match state.process.lock() {
                        Ok(guard) => guard,
                        Err(_) => return,
                    };

                    if let Some(mut child) = guard.take() {
                        tracing::info!("Stopping cloudflared...");

                        let _ = child.kill();
                        let _ = child.wait();

                        tracing::info!("cloudflared stopped");
                    }
                } // <-- guard is dropped here
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
