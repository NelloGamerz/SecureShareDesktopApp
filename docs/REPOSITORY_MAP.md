# Repository Map

```text
project/
├── src/                         React/Vite frontend
│   ├── api/tauri.ts             Typed Tauri command/event wrappers
│   ├── components/              Layout and reusable UI
│   ├── contexts/                Auth and WebSocket contexts
│   ├── features/                Auth, devices, transfers, organization, billing
│   ├── hooks/                   Desktop service synchronization
│   ├── lib/                     Axios, environment, formatting utilities
│   ├── providers/               Clerk/auth/query/theme providers and guards
│   ├── services/                Frontend service adapters
│   └── store/                   Zustand UI, auth, notifications, requests
├── src-tauri/
│   ├── src/lib.rs               Tauri setup, plugins, state, receiver, commands
│   ├── src/commands/            IPC command handlers
│   ├── src/services/             Auth, WebSocket, Cloudflared, secure storage
│   ├── src/state/                Shared Rust state
│   ├── src/transfer/             Chunking, crypto, upload, receiver, merge
│   ├── src/websocket/             Client, manager, heartbeat, reconnect
│   ├── capabilities/             Tauri permissions
│   ├── resources/                Bundled Cloudflared binaries
│   └── tauri*.conf.json          Platform packaging overrides
├── .github/workflows/release.yml CI release matrix
├── package.json                  Frontend scripts/dependencies
├── src-tauri/Cargo.toml          Rust dependencies/features
└── docs/                         Architecture and engineering documentation
```

## Ownership guidance

Change `src/api/tauri.ts` when an IPC contract changes; change the corresponding Rust command and type together. Change transfer protocol code as a unit across `http_client.rs`, `writer.rs`, `crypto.rs`, and `models/`. Treat `lib.rs` as lifecycle wiring, not a place for business logic. External API behavior must be confirmed with the central backend owner because its source is not in this repository.

Not found: backend source, Docker, migrations, server deployment manifests, automated test directories, and a formal API schema.
