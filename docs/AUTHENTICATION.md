# Clerk Authentication

This document describes how signup, login, session initialization, API authentication, and logout currently work in VilSend.

## Overview

Clerk is the identity provider. The application does not implement password validation itself. Clerk renders the sign-in/sign-up UI, creates and maintains the browser session, and exposes the current session as a JWT through `getToken()`.

After Clerk considers the user signed in, VilSend performs a second local initialization step:

1. React receives the Clerk signed-in state.
2. `AuthProvider` requests a Clerk JWT and device information.
3. The frontend invokes the Tauri `login` command with that JWT and device information.
4. Rust stores the token in in-memory `AuthState` and marks the desktop session authenticated.
5. API requests obtain the current Clerk JWT and send it as `Authorization: Bearer <token>`.

Clerk remains the source of truth for whether the user is signed in. The Rust session is a local runtime session that lets desktop services use the same credential.

## Application startup

`AppProviders` checks `VITE_CLERK_PUBLISHABLE_KEY`:

- When a valid key is present, the app mounts `ClerkProvider`.
- `AxiosProvider` is mounted inside `ClerkProvider`, so it can call Clerk's `getToken()`.
- `AuthProvider` is also mounted inside `ClerkProvider` and synchronizes Clerk with Tauri.
- When no valid key is present, the app runs in preview mode. Clerk is not mounted and authentication guards allow the UI to render without a real account.

The configured key is treated as invalid when it is empty or starts with the known placeholder prefix in `src/lib/env.ts`.

## Signup flow

1. A signed-out user opens `/sign-up`, either directly or through the link on the sign-in screen.
2. `SignUpPage` renders Clerk's `<SignUp />` component.
3. Clerk collects and verifies the configured signup credentials or social-provider credentials. The exact fields and verification steps are controlled by the Clerk instance configuration.
4. On successful signup, Clerk creates the session and redirects to `/onboarding` because `fallbackRedirectUrl` is configured to that path.
5. The Clerk signed-in state changes. `AuthProvider` gets a JWT, calls Tauri `login`, and marks the local session authenticated.
6. The protected onboarding page can call the API using the Clerk bearer token.

The signup screen links existing users to `/sign-in` with `signInUrl="/sign-in"`.

## Login flow

1. A signed-out user opens `/sign-in`.
2. `SignInPage` renders Clerk's `<SignIn />` component.
3. Clerk authenticates the user and restores or creates the Clerk session.
4. Clerk redirects to `/onboarding` using `fallbackRedirectUrl`.
5. `AuthProvider` bootstraps the local desktop session:
	- calls `getToken()`;
	- calls `getDeviceInfo()`;
	- invokes `loginWithTauri(token, deviceInfo)`, which invokes the Rust `login` command;
	- stores the token and Clerk user ID in React state; and
	- sets `isAuthenticated` to `true`.
6. The user can access protected application routes. Onboarding/profile data determines whether the user remains on `/onboarding` or proceeds to `/organization` through the post-auth redirect helper.

## Route protection

The router uses two Clerk-aware guards:

- `AuthGate` wraps `/sign-in` and `/sign-up`. Signed-in users should not remain on authentication pages.
- `ProtectedRoute` wraps `/onboarding` and the main `AppLayout` routes, including transfers, devices, organization, members, billing, and settings.

The root route checks Clerk's `isLoaded` and `isSignedIn`. It shows a loading screen while Clerk initializes, then redirects signed-in users to `/organization` and signed-out users to `/sign-in`.

`useCurrentUser()` normalizes the Clerk user into the fields used by the UI: name, email, profile image, initials, and signed-in status.

## API authentication

`AxiosProvider` registers a token getter with `src/lib/api.ts`. Before an API request:

1. Axios calls Clerk's `getToken()` when the user is signed in.
2. If a token is available, Axios adds `Authorization: Bearer <token>`.
3. Axios also adds the current `X-Device-Id`.
4. The request is sent to `VITE_API_BASE_URL`, which defaults to `http://localhost:8000`.

The response interceptor converts HTTP 401 responses into the error `Your session has expired. Please sign in again.`

## Token refresh and Tauri state

Clerk tokens are not assumed to remain valid indefinitely. While Clerk is loaded and the user is signed in, `AuthProvider`:

- synchronizes a token immediately; and
- calls the Tauri `update_auth_token` command every 30 seconds.

Rust keeps the token, session, user ID, and authentication flag in `AuthState`, backed by asynchronous in-memory locks. `AuthService` does not persist the token to disk. Restarting the desktop app therefore requires Clerk to initialize again.

The Rust `auth-state-changed` event is listened to by `AuthProvider`, allowing native auth state changes to update the React session state.

## Logout flow

When Clerk sign-out is triggered from the Clerk `UserButton` or another Clerk flow:

1. Clerk ends the Clerk session.
2. Clerk reports `isSignedIn === false`.
3. `AuthProvider` clears its React session, user, and authenticated flag.

When the app explicitly calls the local `logout()` helper, it also:

1. stops the Tauri WebSocket;
2. stops Cloudflared;
3. invokes the Rust `logout` command;
4. clears the Rust token/session/authenticated state; and
5. clears the React auth state.

The Clerk `UserButton` is configured with `afterSignOutUrl="/sign-in"`.

## Current security boundaries

- Clerk owns identity, credential verification, and the Clerk session.
- The frontend holds the current raw token in React memory as part of `Session`.
- Rust holds a copy of the token in process memory through `AuthState`.
- No auth token is persisted by `AuthService`.
- The backend must validate the Clerk JWT on every protected request. Sending a bearer token from the client does not provide authentication by itself.
- A 401 response is surfaced to the frontend, but the current code does not automatically redirect or force Clerk sign-out from that interceptor.

## Related source files

- `src/providers/app-providers.tsx`: Clerk provider and provider order.
- `src/features/auth/sign-in-page.tsx`: sign-in screen and redirect.
- `src/features/auth/sign-up-page.tsx`: sign-up screen and redirect.
- `src/contexts/auth-context.tsx`: Clerk-to-Tauri session bootstrap, refresh, and local logout.
- `src/providers/auth-guard.tsx`: route guards and normalized current user.
- `src/lib/api.ts` and `src/providers/axios-provider.tsx`: bearer-token injection.
- `src-tauri/src/services/auth_service.rs`: Rust in-memory session lifecycle.
- `src-tauri/src/commands/auth.rs`: Tauri login, logout, and token-update commands.
