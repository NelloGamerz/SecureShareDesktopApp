# ADR-0006 — Mobile: Tauri 2 mobile, with UniFFI as an escape hatch

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 12
- **Supersedes:** —

## Context

The core is portable. The question is the **shell**: how do iOS and Android apps
get built?

Two viable paths:

- **A. Tauri 2 mobile** — reuse the React UI, add a mobile target.
- **B. Native shell (SwiftUI/Compose) over the Rust core via UniFFI** — rewrite
  the UI, share only the core.

The `vilsend-core` extraction (ADR-0001) makes the *core* shared either way.
This decision is therefore entirely about the UI shell and the platform
integration surface.

## Verified constraints

| Plugin | Mobile support | Source |
|---|---|---|
| `tauri-plugin-updater` | **Not available** | Tauri plugin compatibility tables |
| `tauri-plugin-single-instance` | **Desktop only** | Official README (Linux/Win/macOS ✓; Android ✗, iOS ✗) |
| `tauri-plugin-stronghold` | **Desktop only** | Tauri plugin tables |
| `tauri-plugin-deep-link` | **Supported**, per-platform config required | Tauri docs; used for OAuth callbacks on mobile |
| Tauri 2 mobile overall | **Stable** — 2.11.x line; production users exist | Tauri release notes |

Concrete blockers in the current code:

| # | Blocker | Location |
|---|---|---|
| 1 | `window.open_devtools()` called **unconditionally**, including release builds | `src-tauri/src/lib.rs:123` |
| 2 | `tauri-plugin-stronghold` registered (desktop-only) | `src-tauri/src/lib.rs:112` |
| 3 | `tauri-plugin-single-instance` — needs `#[cfg(desktop)]` | `src-tauri/src/lib.rs:66-72` |
| 4 | `keyring` crate — no mobile backend (currently **unused anyway**) | `Cargo.toml:44` |
| 5 | In-process axum receiver on `0.0.0.0:7878` — **iOS suspends the app** | `src-tauri/src/lib.rs:298-337` |

Items 1 and 2 should be fixed **now** (Phase 1), regardless of mobile: 1 is a
security defect and 2 is dead weight.

## Options considered

**A. Tauri 2 mobile.**
Chosen, with a caveat.

**B. Native shell + UniFFI.**
Not chosen as the primary path, but **the UniFFI crate is built anyway** as an
escape hatch.

**C. React Native / Expo.**
Rejected — a third UI stack, and the Rust core would need yet another binding
without the maturity of UniFFI.

**D. Defer mobile entirely.**
Defensible. Recorded as the fallback if Q-08 says mobile is not a near-term
priority.

## Decision

Adopt **Tauri 2 mobile**, because:

1. The team already owns the React codebase. Option B means writing and
   maintaining **two** UIs — the single most expensive thing available at this
   stage.
2. The plugins genuinely depended on (`dialog`, `fs`, `store`, `opener`,
   `deep-link`) are available or have documented replacements.
3. The core is portable either way, so the mobile decision is only about the UI.

**Build `vilsend-ffi` (UniFFI) alongside at Phase 12**, even while shipping
Tauri mobile. It is ~200 lines of UDL plus a build step, and it keeps option B
available if Tauri mobile disappoints. **Build it *alongside*, not *instead
of*.** Do not ship it; just keep it compiling.

**Spike the iOS background-transfer problem first.** Task 12.5 is the hardest
item in the entire migration and may be impractical within the Tauri mobile
model. Timebox it before committing to 12.7.

## Consequences

**Positive**

- One React UI across desktop and mobile.
- Desktop-only plugins are already correctly `cfg`-gated or trivially gated.
- The core, transfer engine, and transports need no mobile-specific forks.

**Negative**

- Binary size baseline of roughly 8–15 MB per platform.
- **The in-process axum receiver does not survive iOS backgrounding.** The
  receiver model must be replaced with `NSURLSession` background transfers on
  iOS. This is a genuine architectural risk (**R-05**), not a detail.
- Mobile needs its own `CredentialStore` (Keychain/Keystore) and its own
  `InteractiveAuthUi` (`ASWebAuthenticationSession` / Custom Tabs).

**Neutral**

- Custom URL schemes should be replaced by Universal Links / App Links on
  mobile — a scheme is claimable by any installed app, a Universal Link is bound
  to the signing identity.

## Related

- [`../04-sdk-cli-mobile-build-plan.md`](../04-sdk-cli-mobile-build-plan.md) §6
- [`../03-authentication.md`](../03-authentication.md) §3.5
- R-05, R-13 in [`../07-risks-and-open-questions.md`](../07-risks-and-open-questions.md)
