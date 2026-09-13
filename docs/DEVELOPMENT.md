# Development

## Prerequisites

Install Node.js/npm, Rust stable, and the Tauri v2 platform prerequisites. On Linux, the release workflow installs WebKitGTK, GTK, AppIndicator, and librsvg packages. A working Clerk publishable key and central API URL are needed for authenticated behavior.

## Configuration

Copy `.env.example` to `.env` and set `VITE_CLERK_PUBLISHABLE_KEY`, `VITE_API_BASE_URL`, and `VITE_APP_NAME`. Rust reads `WS_URL` and `API_URL`; development defaults are `ws://localhost:8080/ws` and `http://localhost:8080`.

## Commands

```bash
npm ci
npm run dev
npm run build
npm run typecheck
npm run lint
npm run tauri dev
npm run tauri build
cargo check --manifest-path src-tauri/Cargo.toml
```

`npm run tauri dev` uses Vite on port 1420 and the Tauri development window. `npm run tauri build` runs `npm run build` first through Tauri config and packages the app.

## Debugging

Use browser/WebView developer tools in development, Rust logs through `tauri-plugin-log`, and `RUST_LOG`/`RUST_BACKTRACE` as appropriate. Start by correlating transfer ID, device ID, and WebSocket status. Avoid logging credentials, token lengths, private keys, or full file paths when paths are sensitive.

## Testing reality

No frontend, Rust, integration, or E2E test suite was found. The current minimum validation is TypeScript typecheck, ESLint, and Cargo check. Add tests before changing authentication, pairing, receiver authorization, or transfer concurrency.
