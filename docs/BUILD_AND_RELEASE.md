# Build and Release

## Tauri build

`crates/desktop/tauri.conf.json` builds the Vite `dist` directory, enables all bundle targets, includes updater artifacts, and points the updater at `https://update.vilsend.in/latest.json`. Platform overrides exist for Linux, macOS, Windows, and Windows Store.

The normal GitHub workflow triggers on `v*` tags, uses Node 22 and Rust stable, caches npm/Cargo, and runs a matrix for Ubuntu, Windows, and macOS. It passes Clerk/API/app environment values as build-time variables and uses Tauri signing secrets.

## Resources and sidecar

Cloudflared is resolved from Tauri's resource directory under platform-specific paths. The active base config has an empty resource list; platform configs and the Store config are responsible for resource inclusion. Verify each artifact contains the expected Cloudflared executable before release.

## Microsoft Store

The Store configuration disables the updater and targets MSI in the checked-in config, while the Store packaging/publish jobs in `.github/workflows/release.yml` are commented out. Existing Store documents describe intended MSIX behavior but are not proof of an active pipeline.

## Risks

The workflow uses `tauri-apps/tauri-action@v1` while the project uses Tauri v2 CLI/dependencies; validate action compatibility. Platform-specific notarization, signing identity setup, Linux package testing, macOS Intel/ARM artifact verification, and automatic rollback are not documented in the repository.
