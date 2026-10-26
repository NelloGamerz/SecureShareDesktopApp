# VilSend Codebase Guide

## Executive summary

VilSend is a **desktop application**, not a mobile application. It uses:

- A React and TypeScript user interface rendered inside a Tauri webview.
- A Rust native layer for desktop capabilities, security-sensitive work, networking, and file transfers.
- A remote HTTP API for account, organization, device, billing, and transfer metadata.
- A Rust WebSocket client, Cloudflared tunnel, local SQLite database, and Tauri events for the file-transfer runtime.

The application is designed to run on Windows, macOS, and Linux through Tauri. It is not currently a complete Android or iOS application, even though the Rust entry point contains a `mobile` conditional that comes from Tauri's cross-platform scaffolding.

## Desktop or mobile?

### Why this is a desktop app

The repository contains several desktop-only integrations:

- `src-tauri/` contains the Rust application and Tauri commands.
- The UI invokes native commands such as `start_websocket`, `start_cloudflared_cmd`, `pause_transfer`, and `set_default_download_location`.
- The app uses native directory/file dialogs and filesystem access.
- Authentication uses a desktop deep link (`vilsend://auth/callback`) and the system browser.
- Cloudflared is launched as a bundled operating-system process.
- Local transfer metadata is stored in SQLite under the Tauri application-data directory.
- The window is configured with a desktop size (`800 x 600`) and desktop bundling targets.

The responsive CSS and components such as `MobileDrawer` make the desktop window usable at narrow widths. That is responsive desktop UI, not proof of a mobile build.

### Would it work on a MacBook?

**The architecture supports macOS, and there is a macOS Tauri configuration, but the Mac build should be treated as needing release verification.**

Evidence that macOS is intended:

- `src-tauri/tauri.macos.conf.json` exists.
- Rust has `target_os = "macos"` device-detection code.
- The base bundle includes an `icon.icns`.
- The project has a macOS Cloudflared resource path.
- The release documentation describes Windows, Linux, and macOS distributions.

Important qualification:

- The macOS configuration currently bundles `resources/cloudflared/macos-x64/cloudflared`.
- Apple Silicon MacBooks use ARM64. The Rust source contains a commented reference to a `macos-arm64` binary, but the active macOS configuration shown in the repository only declares the x64 resource.
- Therefore, Intel macOS is the clearest supported target from the checked-in configuration. Apple Silicon requires an ARM64 resource/build or a verified universal-binary strategy.
- macOS distribution also requires the normal Tauri release work: a correctly installed Rust/Node toolchain, Apple signing/notarization configuration, permissions review, and a tested Cloudflared binary.

The frontend alone can run in a browser with `npm run dev`, but native features will not behave as intended without the Tauri runtime.

## High-level architecture

```text
┌─────────────────────────────────────────────────────────────┐
│ React + TypeScript renderer                                │
│                                                             │
│ routes → feature pages → hooks/API adapters                 │
│          local state / Zustand / TanStack Query             │
└───────────────┬─────────────────────────────┬───────────────┘
                │ HTTP                        │ Tauri invoke/events
                v                             v
       Remote VilSend API             Rust/Tauri native layer
                                      ├─ Auth + secure storage
                                      ├─ WebSocket
                                      ├─ Cloudflared
                                      ├─ Transfer engine
                                      ├─ Local SQLite
                                      └─ Native dialogs/filesystem
```

The renderer should not call Rust commands directly from arbitrary components. The normal boundary is:

1. A page or feature hook receives a user action.
2. A feature API module calls Axios for remote HTTP or `src/api/tauri.ts` for native commands.
3. Rust executes the native operation and emits events when work progresses.
4. A context, Zustand store, or TanStack Query cache is updated.
5. React re-renders the affected button, dialog, list, status, or progress bar.

## Repository structure

```text
SecureShareDesktopApp/
├── src/                         React/TypeScript renderer
│   ├── main.tsx                 Application bootstrap
│   ├── App.tsx                  Root component and desktop lifecycle overlay
│   ├── router.tsx               Routes, lazy pages, and route guards
│   ├── api/tauri.ts             Typed wrappers around Tauri commands/events
│   ├── components/              Reusable layout and UI primitives
│   ├── contexts/                Auth and WebSocket React contexts
│   ├── features/                Feature-specific pages, hooks, and APIs
│   ├── hooks/                   Shared hooks and desktop lifecycle orchestration
│   ├── lib/                     Axios, environment, formatting, and utilities
│   ├── providers/               Global provider composition and guards
│   ├── services/                Frontend service adapters
│   ├── store/                   Zustand stores
│   └── types/                   Shared TypeScript types
├── src-tauri/                   Rust/Tauri native application
│   ├── src/commands/             IPC commands exposed to the renderer
│   ├── src/services/             Auth, storage, tunnel, and other services
│   ├── src/transfer/             Chunking, encryption, upload/download logic
│   ├── src/websocket/             WebSocket client and reconnect behavior
│   ├── src/state/                 Shared native runtime state
│   ├── src/events/                Native event dispatch
│   ├── src/models/                Rust request/response models
│   ├── src/lib.rs                 Tauri builder, plugins, and startup wiring
│   ├── Cargo.toml                 Rust dependencies
│   └── tauri.*.conf.json          Platform packaging configuration
├── docs/                         Architecture and engineering documentation
├── public/                       Static frontend assets
├── package.json                  Node dependencies and scripts
├── vite.config.ts                Vite configuration
└── tailwind.config.js            Tailwind CSS configuration
```

## Application startup

`src/main.tsx` mounts the application in this order:

1. React `StrictMode`.
2. `AppProviders`.
3. `BrowserRouter`.
4. `App`.

`AppProviders` supplies:

```text
AxiosProvider
  └─ QueryProvider
      └─ ThemeProvider
          └─ AuthProvider
              └─ WebSocketProvider
                  └─ application routes
```

`App.tsx` loads the route tree and calls `useDesktopServices()`. It also renders a full-screen device-limit overlay while a forced logout countdown is active.

## Routing and screen structure

`src/router.tsx` separates routes into:

- Public authentication pages: sign-in, sign-up, forgot password, and email verification.
- Protected onboarding.
- Protected application pages inside `AppLayout`: transfers, devices, pairing, organization, members, billing, and settings.
- A root redirect and a not-found page.

Pages are lazy-loaded where appropriate. `ProtectedRoute` waits for the initial native auth status before redirecting. `AuthGate` prevents an already signed-in user from returning to sign-in/sign-up. Billing and members have additional profile-based access guards.

## Technology stack

### Frontend

| Area | Technology |
|---|---|
| Language | TypeScript 5.5 |
| UI | React 18.3, React DOM |
| Build/dev server | Vite 5.4 |
| Routing | React Router 6 |
| Styling | Tailwind CSS 3, PostCSS, CSS modules/global CSS patterns |
| UI primitives | Radix UI |
| Icons | lucide-react |
| Animation | Framer Motion |
| Server-state/cache | TanStack React Query 5 |
| Client shared state | Zustand 4 |
| Forms/validation | React Hook Form, Zod, `@hookform/resolvers` |
| HTTP | Axios |
| Notifications | Sonner and a Zustand notification store |
| Charts | Recharts |
| QR codes | `qrcode` |

### Desktop/native layer

| Area | Technology |
|---|---|
| Desktop shell | Tauri 2 |
| Native language | Rust 2021 edition |
| Async runtime | Tokio |
| Native HTTP | Reqwest |
| WebSocket | tokio-tungstenite |
| Local HTTP receiver | Axum |
| Local database | SQLite through SQLx |
| Secure storage | Keyring, Tauri secure-storage, Stronghold |
| Cryptography | AES-GCM, Ed25519, X25519, HKDF, SHA-2, zeroization |
| Tunnel | Bundled Cloudflared process |
| Native integrations | Tauri dialog, filesystem, opener, OS, log, updater, deep-link, store plugins |

### Commands

```bash
npm run dev       # Vite renderer development server
npm run build     # Type-check/build the renderer
npm run typecheck # TypeScript check
npm run lint      # ESLint
npm run tauri     # Tauri CLI passthrough
```

The complete desktop development command is normally run through Tauri so the renderer can use `invoke`, native plugins, and native events.

## State management

There is no single state store. State is intentionally divided by ownership and lifetime.

### 1. React local state: temporary UI state

Use `useState` for state that belongs to one component or one short interaction:

- `TransfersPage`: whether the “New transfer” dialog is open.
- `DevicesPage`: which device rename/remove dialog is open.
- `SettingsPage`: the selected download directory displayed on screen.
- `AuthProvider`: bootstrap/loading/error/authentication flags.
- `useDesktopServices`: device-limit overlay and logout countdown.
- Multi-step transfer dialog: current step, selected files, selected destination, and review data.

A local-state button usually follows this pattern:

```tsx
<Button onClick={() => setIsNewTransferOpen(true)}>
  New transfer
</Button>

<NewTransferDialog
  open={isNewTransferOpen}
  onOpenChange={setIsNewTransferOpen}
/>
```

The button does not perform the transfer itself. It changes local UI state; the dialog owns the form and eventually calls a feature mutation.

### 2. React Context: cross-cutting session/runtime state

#### Authentication context

`AuthProvider` is the renderer's view of the native session:

- `isLoaded`: the first Rust auth-status check has finished.
- `isAuthenticated`: whether Rust has a valid desktop session.
- `isAuthenticating`: whether the system-browser PKCE flow is active.
- `error`: the last safe-to-display auth error.
- `signIn`, `signUp`, `cancelAuthentication`, and `logout`.

The access token is deliberately not put in React state. Rust stores the session/token, and the Axios token getter asks Rust for the current token for each request.

Sign-in button flow:

```text
click sign in
  → prevent competing second flow
  → set isAuthenticating = true
  → invoke Rust start_desktop_auth
  → system browser opens
  → browser returns to vilsend://auth/callback
  → Rust validates/exchanges/stores the session
  → Rust emits auth-state-changed
  → AuthProvider sets authenticated state
  → ProtectedRoute allows the application
```

The provider also handles cancellation, a five-minute timeout, listener cleanup, and auth errors.

#### WebSocket context

`WebSocketProvider` listens for:

- `server-event` with connection-status payloads.
- `websocket-message` payloads.

It exposes `status`, a derived `connected` boolean, accumulated messages, and `send()`. `send()` delegates to the Tauri `send_message` command.

### 3. TanStack Query: remote server state

TanStack Query owns data fetched from the HTTP API and the status of asynchronous mutations:

- Current user profile.
- Devices and device health.
- Transfers.
- Organization members and related feature data.

Typical query state is:

```text
isLoading → show skeleton/loading text
isError   → show error and a retry button
data      → render the result
```

Mutations expose `isPending`, `error`, `mutate`, and `mutateAsync` as appropriate. Successful mutations invalidate a query key so the UI refetches authoritative data. For example, transfer actions invalidate `["transfers"]`.

The query provider configures one-minute freshness, five-minute garbage collection, one retry, and no refetch on window focus.

### 4. Zustand: shared client/UI state

Zustand stores are used for state that multiple unrelated components need:

- `ui-store.ts`: sidebar collapse, mobile drawer, command palette, and theme. Sidebar/theme are persisted as `vilsend-ui`; transient drawer/command state is not persisted.
- `transfer-request-store.ts`: incoming transfer requests received from native events. Duplicate transfer IDs are ignored.
- `notification-store.ts`: notifications, unread count, read actions, and clear-all.
- `websocket-store.ts`: a store-level WebSocket status/message path. The current page-level WebSocket context is the primary provider path.
- `auth-store.ts`: exists for historical/alternate auth state, but `AuthProvider` is the authoritative current auth path.

Zustand actions update state synchronously through `set(...)`. Components subscribe to only the selected fields, which limits unnecessary re-renders.

### 5. Rust/Tauri state and events: native runtime state

Rust owns long-lived or security-sensitive state:

- Auth/session data and secure tokens.
- Cloudflared process state.
- WebSocket connection state.
- Transfer workers and progress.
- Local transfer-file metadata in SQLite.
- Device identity and secure settings.

React invokes typed wrappers from `src/api/tauri.ts`. Rust emits events such as `auth-state-changed`, `auth-error`, `server-event`, `websocket-message`, and transfer progress/completion/failure/pause/resume/cancel events.

## How buttons and actions manage state

### Sign-in and sign-up

The button calls `signIn()` or `signUp()` from `useAuth()`. While the browser flow is active, the auth provider's `isAuthenticating` state should be used by the page to show a waiting/disabled state. A ref prevents double-clicks from starting two PKCE flows. Rust events, not the button itself, decide when authentication succeeded.

### New transfer

The page button sets `isNewTransferOpen` to `true`. Inside the dialog:

1. The user selects files/folders through the Tauri dialog plugin.
2. File metadata is read through the Tauri filesystem API.
3. The user selects an online destination device/member.
4. The review step calls `useCreateTransfer()`.
5. The mutation posts transfer metadata through Axios.
6. On success, the `["transfers"]` query is invalidated and the list refreshes.

The dialog's step, selected files, selected destination, and validation are local component state. The actual transfer engine is Rust-driven after the transfer is created.

### Accept, reject, pause, resume, and cancel transfer

On the transfers page:

- Accept/reject buttons call `useTransferAction()`, which PATCHes the transfer action to the remote API.
- Pause/resume/cancel controls call `useTransferControls()`, which invokes Rust commands.
- Successful actions invalidate `["transfers"]`.
- Native transfer events update the cached transfer row in place with bytes, percentage, speed, ETA, and status.
- Status values such as `CONNECTING`, `HANDSHAKING`, `UPLOADING`, `DOWNLOADING`, `COMPLETED`, and `FAILED` determine the badge and progress display.

This is why a progress button can change without a full page reload: the event listener writes the new progress into TanStack Query's cache, and React re-renders the row.

### Incoming transfer requests

The desktop lifecycle hook registers a native transfer-request listener. It adds a normalized request to `useTransferRequestStore` and a notification to `useNotificationStore`. The transfers page subscribes to the request store and renders Accept/Reject buttons when requests exist.

The store deduplicates requests by transfer ID. The current page has removal calls commented out, so an accepted/rejected request may remain in the request list until the store is explicitly cleaned up or the process is restarted; this is a behavior to verify if request cards appear to persist.

### Device page

Device data is query state. The page shows loading skeletons, an error card with `refetch()`, an empty state, or device cards. Rename/remove dialogs are controlled with local `Device | null` state:

```text
null          → no dialog
selectedDevice → dialog open for that device
dialog close  → set state back to null
```

The actual rename/remove operation is handled by the dialog's feature hooks/mutations, followed by query refresh behavior.

### Settings

- Theme reads and writes `useUIStore`, and `ThemeProvider` applies the selected theme.
- “Choose folder” opens a native directory picker, saves the selected path through `set_default_download_location`, then updates local display state.
- Profile fields currently use `defaultValue`; they are not wired to a save mutation in the checked-in page.
- Some security/notification controls are present as UI examples but are not active routes or persistence flows because their tabs/actions are commented or not connected.

### Layout controls

Sidebar collapse, mobile drawer, command palette, theme toggle, and notification actions call Zustand actions. These controls do not need a server request:

```text
button click → Zustand action → store update → subscribed components re-render
```

## Authentication and API data flow

```text
Rust AuthState
   ↑ get_auth_token() for each request
Axios request interceptor
   ├─ Authorization header
   └─ X-Device-Id header
Remote VilSend API
   ↓ response/query data
TanStack Query cache
   ↓
Feature page
```

The Rust session is authoritative. A 401 is converted into an auth error by the Axios response interceptor. The profile query is also used for onboarding and role-based route decisions.

## Transfer data flow

```text
New transfer dialog
   → POST /transfers
   → transfer metadata returned
   → native transfer runtime
   → Cloudflared/WebSocket/local receiver as needed
   → Rust emits progress/status events
   → useTransferProgressListener updates Query cache
   → transfer row/progress controls re-render
```

Incoming transfers follow the reverse notification path:

```text
Rust event → useDesktopServices listener
           → transfer-request-store + notification-store
           → transfers page and notification dropdown
```

## Build and platform notes

- Vite builds the renderer into `dist/`.
- Tauri packages the renderer together with the Rust binary and platform resources.
- `tauri.conf.json` enables the updater, deep-link scheme, desktop window, and all bundle targets.
- `tauri.windows.conf.json`, `tauri.linux.conf.json`, and `tauri.macos.conf.json` supply platform overrides.
- Windows has additional power-management code and a single-instance/deep-link dependency.
- Linux and Windows explicitly configure the single-instance deep-link forwarding dependency; macOS uses the platform's separate deep-link path.
- Before claiming macOS release support, test Intel and Apple Silicon separately, verify the Cloudflared resource architecture, and perform signing/notarization.

## Practical reading order for new developers

1. Read `src/main.tsx`, `src/App.tsx`, and `src/providers/app-providers.tsx`.
2. Read `src/router.tsx` and `src/providers/auth-guard.tsx`.
3. Read `src/contexts/auth-context.tsx` to understand the session boundary.
4. Read `src/api/tauri.ts` to see the renderer/native contract.
5. Read one complete feature, preferably `src/features/transfers/`.
6. Read `src/hooks/useDesktopServices.ts` for lifecycle synchronization.
7. Read `src-tauri/src/lib.rs` and then the relevant Rust command/service module.
8. Read the matching platform config before changing packaging or native resources.

## Current implementation caveats

These are relevant when interpreting the code, not necessarily blockers for the desktop architecture:

- Some route entries and feature actions are intentionally commented out.
- There are two WebSocket state approaches: the active React context and a Zustand store.
- Transfer request removal is currently commented in the transfers page.
- Settings contains UI controls that are not all connected to persistence/API mutations.
- The macOS config visibly names an x64 Cloudflared resource; Apple Silicon packaging needs explicit verification.

