# Authentication

This document describes how sign-in, sign-up, session initialization, API authentication, and logout work in the VilSend desktop app.

Clerk is the identity provider and the token issuer. The application does not implement credential validation itself, and it does not issue its own tokens.

> **Scope.** This document covers only the Tauri application (React renderer + Rust layer). The Spring Boot backend is **outside the scope of this change** and must not be modified. The backend is assumed to already validate Clerk-issued tokens; see [Backend contract](#backend-contract) for the one assumption the desktop app makes about it.

---

## Overview

Authentication happens in the **system browser**, not inside the application window. The app uses the OAuth 2.0 **Authorization Code flow with PKCE** (RFC 7636), which is the recommended approach for native/desktop applications (RFC 8252).

The desktop app is a **public client**:

- It holds **no Clerk secret key**. A secret embedded in a desktop binary is not a secret.
- PKCE replaces the client secret. A `code_verifier` is generated in Rust, and only its SHA-256 challenge is sent with the authorization request.
- The authorization code is exchanged for tokens by the Rust layer. The code never passes through the renderer.

Two properties are deliberately maintained:

1. **The token never enters React state.** It lives in the Rust `AuthState` and is fetched per request.
2. **Nothing is persisted.** No token, refresh token, or PKCE verifier is written to disk.

---

## The desktop authentication flow

```
User clicks "Continue in browser"
        │
        ▼
React  signIn() / signUp()                 src/contexts/auth-context.tsx
        │  invoke("start_desktop_auth", { config, mode })
        ▼
Rust   OAuthService::begin()               src-tauri/src/services/oauth_service.rs
        │  • generate code_verifier (32 random bytes, base64url)
        │  • code_challenge = base64url(SHA-256(verifier))
        │  • generate state (32 random bytes)
        │  • store PendingLogin in memory (single-use)
        ▼
Rust   app.opener().open_url(authorize_url)   →  system browser
        │
        ▼
Clerk  hosted sign-in / sign-up page (user authenticates here)
        │  redirect to vilsend://auth/callback?code=…&state=…
        ▼
OS     delivers the URL to the app
        │  • running app  → deep-link plugin → on_open_url
        │  • closed app   → launched with URL in argv (single-instance forwards it)
        ▼
Rust   commands::auth::handle_auth_callback()
        │  • ignore URLs that carry neither `code` nor `error`
        │  • OAuthService::complete()
        │      – consume PendingLogin          (replay protection)
        │      – verify callback matches the requested redirect URI
        │      – verify `state` (constant-time compare)
        │      – POST code + code_verifier to {issuer}/oauth/token
        │      – validate token_type is Bearer
        │      – store access token, refresh token, expiry in AuthState
        ▼
Rust   AuthService::notify_signed_in()
        │  emit("auth-state-changed", { isAuthenticated: true, userId })
        ▼
React  AuthProvider updates session/user state
        │
        ▼
React  AuthGate redirects to /organization
        │
        ▼
Axios  request interceptor → invoke("get_auth_token")
        └─ Authorization: Bearer <clerk token>  +  X-Device-Id
```

### Endpoints

Both are derived from the issuer, which is itself derived from the Clerk publishable key:

| Endpoint | URL |
| --- | --- |
| Authorization | `{issuer}/oauth/authorize` |
| Token | `{issuer}/oauth/token` |

For the key currently in `.env`, the issuer resolves to `https://clerk.vilsend.in`.

---

## Sign-in behaviour

1. A signed-out user opens `/sign-in`, either directly or via the guard redirect.
2. The screen shows a single **Continue in browser** action. No credentials are collected in the app window.
3. Clicking it calls `signIn()`, which invokes the Tauri `start_desktop_auth` command with `mode: "sign_in"`.
4. Rust builds an authorization URL and opens it in the **system browser**.
5. The screen switches to a waiting state and shows a **Cancel** action.
6. The user authenticates in the browser. Clerk redirects to `vilsend://auth/callback`.
7. Rust completes the exchange (see above) and notifies the renderer.
8. `AuthGate` sees `isAuthenticated === true` and redirects to `/organization`.

Duplicate attempts are prevented on both sides:

- React keeps an in-flight ref and ignores repeat clicks while a flow is active.
- Rust rejects a second `start_desktop_auth` while a `PendingLogin` exists.

A 5-minute timeout clears the waiting state and cancels the Rust-side flow so a closed browser tab cannot leave the button spinning forever.

## Sign-up behaviour

Sign-up is the same flow with `mode: "sign_up"`.

Clerk's hosted authorization page handles both registration and sign-in. If the Clerk instance supports steering the authorize page toward registration, set `VITE_CLERK_OAUTH_SIGNUP_PROMPT` and the value is sent as the OIDC `prompt` parameter **only** for the sign-up action. It is unset by default because not every Clerk instance accepts it.

## Route protection

`/sign-in` and `/sign-up` are wrapped in `AuthGate`; `/onboarding` and the main `AppLayout` routes are wrapped in `ProtectedRoute`. Both read the local session from `AuthProvider` — Clerk's React state is no longer involved.

Both guards render a loading screen until the first `get_auth_status` read resolves, so a signed-in user is never briefly bounced to `/sign-in` during startup.

`useCurrentUser()` returns display fields assembled from the local session plus the application's own profile endpoint, keeping the shape the UI already consumed (name, initials, etc.).

---

## Token storage

| What | Where | Persisted? |
| --- | --- | --- |
| Access token | Rust `AuthState.token` (in-memory `RwLock<Option<String>>`) | No |
| Refresh token | Rust `AuthState.refresh_token` | No |
| Expiry (unix seconds) | Rust `AuthState.token_expires_at` | No |
| PKCE `code_verifier` + `state` | Rust `AuthState.pending_login` | No |
| Authorization code | Never stored — exchanged immediately | No |
| Token in the renderer | Only as a transient return value of `get_auth_token` | No |

The renderer holds **no** token in React state, `localStorage`, or `sessionStorage`. The existing Rust in-memory architecture is reused rather than replaced, so tokens are never written to disk unless a future change deliberately adopts the secure-storage plugin.

Consequences to be aware of:

- Restarting the desktop app requires signing in again.
- Closing the app while the browser is open discards the PKCE verifier. The callback then fails cleanly with *"No sign-in is in progress"* and the user starts again. This is intentional: persisting the verifier would mean writing credential-adjacent material to disk.

## Token refresh

Clerk's OAuth access tokens expire (reported via `expires_in`); refresh tokens are issued alongside them.

Refresh is centralised in Rust and is **not** driven by a React timer:

1. The axios interceptor calls `get_auth_token` for every request.
2. `OAuthService::token_for_request` returns the stored token if it is more than 60 seconds from expiry.
3. Otherwise it spends the refresh token at `{issuer}/oauth/token` with `grant_type=refresh_token`.
4. Concurrent refreshes are serialized by a mutex; a waiter re-reads the token the first caller installed rather than spending the refresh token twice.
5. If refresh fails, all credentials are cleared and `auth-state-changed` is emitted with `isAuthenticated: false`, so the UI returns to the sign-in screen instead of looping on 401s.

Because the getter runs per request, the `Authorization` header always reflects the current token with no polling loop.

---

## How React receives authentication state

React is a **consumer** of Rust state, never its owner.

| Direction | Mechanism | Payload |
| --- | --- | --- |
| Rust → React | `auth-state-changed` event | `{ type, isAuthenticated, userId }` |
| Rust → React | `auth-error` event | `{ type, message }` |
| React → Rust | `get_auth_status` command | `{ isAuthenticated, userId, isAuthenticating }` |

`AuthProvider` (`src/contexts/auth-context.tsx`):

- reads `get_auth_status` once on mount and subscribes to both events;
- exposes `session`, `user`, `isAuthenticated`, `isLoaded`, `isAuthenticating`, `error`, `isPreviewMode`, `signIn`, `signUp`, `cancelAuthentication`, `clearError`, `logout`;
- removes both listeners on unmount, and discards listeners that resolve after unmount (React StrictMode safe);
- emits no token material into events, logs, or React state.

## How API requests receive the Clerk token

`AxiosProvider` registers a token getter with `src/lib/api.ts`. For every request the interceptor:

1. calls `get_auth_token`, which reads the token from Rust and refreshes it near expiry;
2. sets `Authorization: Bearer <token>` when a token is returned;
3. always sets `X-Device-Id` from the persisted device identifier;
4. sends the request to `VITE_API_BASE_URL`.

Public endpoints continue to work: with no session, `get_auth_token` resolves to `null` and the request goes out without an `Authorization` header.

The response interceptor still turns HTTP 401 into `Your session has expired. Please sign in again.`

No token, authorization code, or `code_verifier` is logged. The previous token-length diagnostics were removed from the interceptors.

---

## Logout behaviour

Triggered from the user menu or by the device-limit countdown. `logout()` in `AuthProvider`:

1. stops the Tauri WebSocket (`stop_websocket`);
2. stops Cloudflared (`stop_cloudflared_cmd`);
3. invokes the Rust `logout` command, which calls `AuthService::logout`:
   - stops the WebSocket service,
   - clears the access token, refresh token, expiry, session, user id, and authenticated flag,
   - discards any in-flight `PendingLogin` (invalidating local OAuth state),
   - emits `auth-state-changed` with `isAuthenticated: false`;
4. clears the React session, user, and authenticated flag;
5. navigates to `/sign-in`.

Both service shutdowns use `Promise.allSettled`, so a failure in one still allows the session to be cleared.

---

## Configuration

### Environment variables (Tauri app only)

| Variable | Required | Purpose |
| --- | --- | --- |
| `VITE_CLERK_PUBLISHABLE_KEY` | Yes | Clerk instance key. The frontend API origin (issuer) is derived from it. |
| `VITE_CLERK_OAUTH_CLIENT_ID` | Yes | Client ID of the Clerk **OAuth application** (public client). |
| `VITE_CLERK_OAUTH_REDIRECT_URI` | Yes | Defaults to `vilsend://auth/callback`. Must match the registered scheme and be allow-listed in Clerk. |
| `VITE_CLERK_OAUTH_SCOPES` | Yes | Defaults to `openid profile email`. |
| `VITE_CLERK_OAUTH_ISSUER` | No | Overrides the issuer derived from the publishable key. |
| `VITE_CLERK_OAUTH_SIGNUP_PROMPT` | No | OIDC `prompt` sent only for sign-up. |
| `VITE_API_BASE_URL` | Yes | Backend API base URL. |
| `VITE_APP_NAME` | No | Display name. |

**No Clerk secret key may be added to this application.** There is no `CLERK_SECRET_KEY` and there must never be one.

When `VITE_CLERK_OAUTH_CLIENT_ID` is empty, `isDesktopAuthConfigured` is false and the app runs in **preview mode**. In this mode it is **not** considered signed in:

- `isAuthenticated` starts `false` and can only ever become `true` through a completed flow, so protected routes stay protected and redirect to `/sign-in`.
- The sign-in screen shows an "Authentication is not configured" notice and disables the **Continue in browser** action, rather than failing after a click.

Preview mode is therefore a *configuration* state, not a bypass. An earlier version of this app treated it as an implicit session, which let unauthenticated users into protected routes and made sign-out appear to do nothing (the sign-in screen immediately redirected back).

### Clerk dashboard configuration

1. **Developers → OAuth applications → Create application.**
2. **Enable "Public" on the application. This is required.**
   - Clerk's Public toggle "only controls whether Clerk requires a Client Secret on token exchange". With it off, the application is *confidential*, and Clerk answers the token request with `401 invalid_client` — *"Client authentication failed"* — because the desktop app deliberately sends no client secret.
   - The app sends `code_challenge_method=S256` and no secret, so it is a genuine public client.
3. Add the redirect URI: `vilsend://auth/callback`.
   - Clerk accepts custom URL schemes and `127.0.0.1` loopback redirects for native applications.
   - The scheme must be registered for the platform too (see below).
4. Set the client ID as `VITE_CLERK_OAUTH_CLIENT_ID`.
   - It must come from the **same Clerk instance** as `VITE_CLERK_PUBLISHABLE_KEY`. A client ID created in the test instance (`*.clerk.accounts.dev`) will not exist on a live instance and produces the same `401 invalid_client`.
   - The app derives the issuer from the publishable key, so a mismatch is easy to create and hard to spot; the error message prints both the issuer and the client ID so the two can be compared directly.
5. **Enable JWT access tokens** if the backend validates tokens as JWTs. See [Backend contract](#backend-contract) — this is the one setting that must match the backend.
6. Ensure `openid`, `profile`, and `email` are permitted scopes.

---

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `401` at the token endpoint: *"Client authentication failed"* | The OAuth application is confidential, so Clerk expects a client secret the public client does not send. | Enable **Public** on the OAuth application. |
| Same `401`, Public already enabled | The client ID belongs to a different Clerk instance than the publishable key (e.g. test client ID with a live key). | Compare the issuer and client ID printed in the error; reissue the client ID on the matching instance. |
| Callback never reaches the app | The `vilsend://` scheme is not registered for how the app is running. | Use `npm run tauri dev` or an installed build. On macOS only a bundled app in `/Applications` works. |
| *"No sign-in is in progress. Start sign-in again from the app."* | The app restarted while the browser was open, discarding the in-memory PKCE verifier. | Start sign-in again. This is expected — the verifier is intentionally not persisted. |
| Sign-in works but API calls return `401` | The backend does not accept the OAuth access token's `aud`, or access tokens are opaque rather than JWT. | See [Backend contract](#backend-contract). |

---

## Backend contract

The backend is out of scope for this change and must already validate Clerk-issued tokens. The desktop app does not modify, and does not need to modify, any backend code.

**One assumption must be verified by the backend owner.** The token this flow returns is a Clerk **OAuth application access token**, which is *not* the same artifact as the browser session token Clerk's React SDK produces via `getToken()`:

| | Session token (previous flow) | OAuth access token (this flow) |
| --- | --- | --- |
| Issuer (`iss`) | Clerk instance | Clerk instance (same) |
| Audience (`aud`) | — | The OAuth application's client ID |
| Format | JWT | JWT **if** "Generate access tokens as JWTs" is enabled; otherwise opaque (`oat_…`) |

Therefore:

- If the backend validates the JWT signature, `iss`, and `exp` only, the OAuth access token is accepted and **no backend change is needed**.
- If the backend also validates `aud` against a specific expected audience, the OAuth application's client ID must be included in what it accepts.
- If access tokens are opaque rather than JWT, a backend that expects a JWT will reject them — enable JWT access tokens in the Clerk OAuth application settings.

`get_auth_token` returns whatever Clerk issues, and the app sends it verbatim as `Authorization: Bearer <token>`. No transformation or re-signing happens in the desktop app, and Clerk remains the sole token issuer.

---

## Deep link configuration

The callback uses the custom URL scheme `vilsend://auth/callback`.

### Rust plugins

| Plugin | Role |
| --- | --- |
| `tauri-plugin-deep-link` | Receives the callback URL (running app and launch-time). |
| `tauri-plugin-single-instance` (feature `deep-link`) | On Windows/Linux a deep link arrives as a command-line argument to a **new** process. This forwards it to the running instance so an in-flight sign-in completes instead of opening a second copy of the app. |

**Registration order matters.** The single-instance plugin must be registered before all other plugins, and the deep-link plugin immediately after. Its registration is guarded by `#[cfg(any(target_os = "windows", target_os = "linux"))]`.

### Scheme registration

| Platform | How the scheme is registered |
| --- | --- |
| Windows (NSIS/WiX) | `plugins.deep-link.desktop.schemes` in `tauri.conf.json`; the installer writes the registry entry. |
| Windows (MSIX / Store) | MSIX ignores registry writes, so `src-tauri/msix/AppxManifest.xml` declares a `windows.protocol` extension. Keep both in sync. |
| Linux | `plugins.deep-link.desktop.schemes`; written on install. `register_all()` is called in debug builds. |
| macOS | Registration is not possible at runtime; it works only from the bundled app installed in `/Applications`. |

In debug builds `register_all()` is called at startup so `npm run tauri dev` can receive callbacks on Windows and Linux without installing the app.

### Callback handling

The listener is registered exactly once, during Tauri `setup`. It handles both cases:

- **Running app** — `on_open_url` fires when the OS delivers the URL.
- **Launch-time** — `get_current()` is read once at startup, for platforms that launch the app with the URL.

URLs carrying neither `code` nor `error` are ignored, so unrelated deep links never surface an error.

---

## Local development setup

1. Create the Clerk OAuth application (public client) and allow-list `vilsend://auth/callback`.
2. Create `.env` from `.env.example` and set:
   ```
   VITE_CLERK_PUBLISHABLE_KEY=pk_test_…
   VITE_CLERK_OAUTH_CLIENT_ID=<client id>
   ```
3. Run the app: `npm run tauri dev`.
   - The scheme is registered automatically in debug builds on Windows and Linux.
   - **macOS:** runtime registration is not supported. Test the deep-link return only from a bundled app in `/Applications`; sign-in otherwise reaches the callback URL without returning to the app.
4. Click **Continue in browser**, complete sign-in, and confirm the app returns to `/organization`.
5. Verify the token reached the API by checking that a protected request carries `Authorization: Bearer …` and `X-Device-Id`.
6. To test the callback manually on Windows: `start vilsend://auth/callback?code=test&state=test` — the app should report a failed callback rather than crash.

### Platform notes

- **Windows** — Works out of the box with the NSIS/WiX installers. For the Microsoft Store (MSIX) build the protocol extension in `AppxManifest.xml` is required. Running from a portable/unpacked build has no registry entry, so deep links may not return; use `npm run tauri dev` or an installed build.
- **macOS** — Deep links require a bundled, installed app. PKCE and the token exchange are platform-independent.
- **Linux** — Works for AppImage and deb/rpm installs, but an AppImage that is moved after registration loses its deep link, since an absolute path is recorded.

---

## Security properties

- **PKCE (S256).** `code_verifier` is 32 random bytes from a CSPRNG, base64url encoded; the challenge is its SHA-256 digest.
- **State parameter.** 32 random bytes, stored in memory and compared in constant time. A callback whose `state` does not match is rejected.
- **Single-use pending flow.** `PendingLogin` is taken (consumed) before any network I/O, so a replayed callback fails rather than exchanging the code twice.
- **Redirect URI pinning.** The callback's scheme, host, and path must match the requested redirect URI.
- **No client secret.** The app is a public client; nothing secret ships in the binary.
- **No token material in logs.** Authorization codes, access tokens, refresh tokens, id tokens, and `code_verifier` values are never logged. Token-length diagnostics were removed from the API client.
- **No tokens in URLs after the callback.** The callback URL is consumed and discarded; the token is returned to the renderer only through a request-scoped command.
- **Refresh failure clears the session** rather than retrying indefinitely.
- **`sub` extraction is unverified by design.** The `sub` claim is read from the returned token purely to label local session state; it is never used for an authorization decision. All authorization remains the backend's responsibility.
- **Errors are sanitised.** Only Clerk's `error_description` and HTTP status are surfaced; the token endpoint's raw body is never forwarded.

---

## Related source files

| File | Responsibility |
| --- | --- |
| `src-tauri/src/services/oauth_service.rs` | PKCE generation, authorize URL, code exchange, refresh, state validation. |
| `src-tauri/src/commands/auth.rs` | Auth commands and the deep-link callback handler. |
| `src-tauri/src/services/auth_service.rs` | Session lifecycle and sign-in/out notifications. |
| `src-tauri/src/state/auth_state.rs` | In-memory token, refresh token, expiry, and pending flow. |
| `src-tauri/src/lib.rs` | Plugin registration, deep-link listener, command handler list. |
| `src-tauri/tauri.conf.json` | `plugins.deep-link.desktop.schemes`. |
| `src-tauri/msix/AppxManifest.xml` | `windows.protocol` extension for the Store build. |
| `src/lib/auth-config.ts` | OAuth configuration and issuer derivation. |
| `src/contexts/auth-context.tsx` | Session state, sign-in/sign-up, logout, event subscriptions. |
| `src/providers/axios-provider.tsx` | Registers the Tauri token getter with axios. |
| `src/providers/auth-guard.tsx` | Route guards and normalized display user. |
| `src/features/auth/browser-auth-panel.tsx` | Sign-in / sign-up screen. |
| `src/lib/api.ts` | `Authorization` and `X-Device-Id` injection, 401 handling. |
