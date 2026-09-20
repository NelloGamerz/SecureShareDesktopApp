# Project Structure

This document explains the folder structure of the VilSend repository and the responsibility of each major area.

## Top-level structure

```text
server-frontend/
├── .github/workflows/       GitHub Actions release automation
├── docs/                    Project architecture and engineering documentation
├── public/                  Static frontend assets copied by Vite
├── src/                     React and TypeScript frontend
├── crates/                  Cargo workspace members
│   └── desktop/             Tauri configuration and Rust desktop runtime
├── Cargo.toml               Cargo workspace manifest
├── Cargo.lock               Locked Rust dependency versions
├── target/                  Generated Rust build output (workspace root)
├── dist/                    Generated frontend production build
├── node_modules/            Installed npm dependencies
├── package.json             Frontend scripts and dependencies
├── package-lock.json        Locked npm dependency versions
├── vite.config.ts           Vite development and build configuration
├── tsconfig*.json           TypeScript project configuration
├── tailwind.config.js       Tailwind CSS configuration
├── postcss.config.js        PostCSS configuration
├── eslint.config.js         ESLint configuration
├── index.html               Vite HTML entry point
├── .env.example             Frontend environment variable template
└── README.md                Minimal starter README
```

`dist/`, `node_modules/`, and `target/` are generated directories. They should not be edited manually.

## Frontend: `src/`

```text
src/
├── main.tsx                 React application entry point
├── App.tsx                  Root component and desktop service bootstrap
├── router.tsx               React Router route definitions and guards
├── index.css                Global styles and Tailwind layers
├── App.css                  Application-level styles
├── api/
│   └── tauri.ts             Typed wrappers for Tauri commands and events
├── assets/                  Frontend images and bundled UI assets
├── components/
│   ├── layout/              Application shell, sidebar, navbar, dialogs
│   └── ui/                  Reusable UI primitives
├── contexts/
│   ├── auth-context.tsx     Frontend/Rust authentication synchronization
│   └── websocket-context.tsx WebSocket status and message context
├── events/                  Frontend event definitions and adapters
├── features/
│   ├── auth/                Sign-in, sign-up, password, and auth hooks
│   ├── billing/             Subscription and billing screens
│   ├── dashboard/           Dashboard models and UI
│   ├── devices/             Device listing, registration, and pairing
│   ├── members/             Organization member management
│   ├── onboarding/          Initial user/device setup
│   ├── organization/        Organization screens and APIs
│   ├── settings/            User/application settings
│   ├── transfers/           Transfer screens, APIs, and controls
│   ├── activity/            Activity UI, where enabled
│   └── not-found/           Fallback route
├── hooks/                   Reusable React hooks and desktop lifecycle logic
├── lib/
│   ├── api.ts               Axios client and auth/device interceptors
│   ├── env.ts               Vite environment variables and defaults
│   ├── constants.ts         Shared frontend constants
│   ├── format.ts            Formatting helpers
│   └── utils.ts             General utilities
├── providers/
│   ├── app-providers.tsx    Top-level provider composition
│   ├── auth-guard.tsx       Protected/public route behavior
│   ├── axios-provider.tsx   Clerk token wiring for Axios
│   ├── query-provider.tsx   TanStack Query configuration
│   ├── theme-provider.tsx   Theme handling
│   └── ...                  Other provider components
├── services/                Frontend adapters for auth, WebSocket, devices
├── store/                   Zustand stores for UI and runtime state
└── types/                   Shared TypeScript types and declaration files
```

### Frontend dependency direction

Feature pages should call feature APIs or hooks. Feature APIs should use `src/lib/api.ts` for central HTTP calls or `src/api/tauri.ts` for desktop capabilities. Components should not call Rust commands directly when a feature/service adapter can own that contract.

## Tauri and Rust: `crates/desktop/`

```text
crates/desktop/
├── src/
│   ├── lib.rs                Tauri builder, plugins, app state, receiver startup
│   ├── main.rs               Native application entry point
│   ├── build.rs              Tauri build script
│   ├── app/                  Aggregated application state and service wiring
│   ├── commands/             Public IPC commands invoked by React
│   ├── error.rs              Shared Rust application errors
│   ├── events/               Tauri event dispatch and transfer event routing
│   ├── models/               Rust request, response, transfer, and device models
│   ├── services/             Auth, Cloudflared, secure storage, WebSocket services
│   ├── state/                Shared authentication, tunnel, receiver, and WebSocket state
│   ├── transfer/              Chunking, encryption, upload, receiving, retries, merging
│   ├── utils/                Configuration, filesystem, logging, hashing, and networking
│   └── websocket/             WebSocket client, manager, heartbeat, reconnect, protocol
├── capabilities/             Tauri v2 permissions and window capabilities
├── resources/                Bundled Cloudflared binaries by platform
├── icons/                    Desktop and mobile application icons
├── msix/                     Microsoft Store packaging assets
├── gen/                      Generated Tauri schemas
├── Cargo.toml                Rust dependencies and feature flags
├── tauri.conf.json           Base Tauri configuration
├── tauri.linux.conf.json     Linux packaging override
├── tauri.macos.conf.json     macOS packaging override
├── tauri.windows.conf.json   Direct Windows packaging override
└── tauri.windows.store.conf.json Store packaging override
```

### Rust ownership

- `commands/` is the IPC boundary and should stay thin.
- `services/` contains reusable application operations.
- `transfer/` owns the file-transfer protocol and data plane.
- `websocket/` owns the central event/control connection.
- `state/` owns synchronized runtime state.
- `lib.rs` should primarily compose these pieces and manage lifecycle.

## Runtime data locations

- Application data: SQLite `transfer.db` and transfer storage under the Tauri app-data directory.
- Secure storage: tunnel token, tunnel hostname, and device private key.
- Download directory: platform Downloads directory by default, or the persisted custom path.
- Temporary transfer parts: app-data `transfers/incoming/<transfer-id>/`.

## Build and release structure

The frontend is built by Vite into `dist/`. Tauri packages that output with Rust and platform-specific bundlers. `.github/workflows/release.yml` builds Linux, Windows, and macOS distributions on version tags. The Microsoft Store workflow is currently commented out, although Store configuration and packaging files are present.

## What is not present

The repository does not contain a central backend implementation, Spring Boot project, Docker configuration, database migrations for a server database, or an automated test suite. Those systems must be documented separately when their source repositories become available.
