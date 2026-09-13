# Tauri and Rust

## Runtime composition

`src-tauri/src/lib.rs` registers Tauri v2 plugins for filesystem, store, opener, secure storage, dialog, Stronghold, log, and updater. It manages `AppState`, `Cloudflared`, and SQLite-backed local transfer storage. The window-destroyed handler kills Cloudflared, but there is no equally explicit transfer/WebSocket shutdown coordinator.

Commands registered in `lib.rs` include auth (`login`, `logout`, `update_auth_token`), WebSocket (`start_websocket`, `send_message`, `get_connection_status`), tunnel settings/process control, transfer lifecycle, local transfer metadata, identity creation, device type detection, and download-location settings.

Capabilities in `src-tauri/capabilities/default.json` allow core, opener, store, dialog, filesystem stat/read, and log. The application CSP is `null` and the main window has devtools enabled in `src-tauri/tauri.conf.json`.

## Native integrations

- Cloudflared is spawned as a bundled executable with `tunnel --no-autoupdate run --token ...`.
- Tunnel token, hostname, and device X25519 private key use the secure-storage plugin.
- Device identity is generated in `commands/device.rs` and persisted through `KeyringService`.
- A local Axum receiver is started during setup and accepts `/transfer/public-key`, `/transfer/start`, and `/transfer/chunk`.
- SQLite database: app-data `transfer.db`, table `local_transfer_files`, pool max five connections.

## IPC and events

The frontend calls commands through `@tauri-apps/api/core.invoke`. Rust emits `auth-state-changed`, `server-event`, `websocket-message`, and transfer progress/completion/failure/pause/resume/cancel events. `EventDispatcher` connects transfer and WebSocket events to the webview.

## Important risks

The Stronghold builder uses the literal password `b"your-stronghold-password"`; although current application state primarily uses secure storage, this is not an acceptable production secret. The null CSP and broad default permissions deserve a least-privilege review. The receiver is intentionally reachable on all interfaces and therefore must enforce real request authorization and bind/firewall appropriately.

Rust compiles successfully, but `cargo check` reports 27 warnings, including dead state, unused imports, unused config fields, unreachable code, and unused token-request code. These are maintenance signals, not individually confirmed runtime failures.
