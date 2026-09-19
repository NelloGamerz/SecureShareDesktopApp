# ADR-0008 — One `AuthProvider` port; a different flow per client

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 6
- **Supersedes:** —

## Context

Four clients need to authenticate, and they are genuinely different:

| Client | Human? | Browser? | Right flow |
|---|---|---|---|
| Desktop | yes | yes | Browser + PKCE, custom scheme (current) |
| CLI interactive | yes | usually | Device Authorization Grant |
| CLI / CI | **no** | **no** | API key / service token |
| SDK | host decides | host decides | **Bring-your-own-auth** |
| Mobile | yes | yes | System browser + PKCE via Universal/App Links |

The insight that makes one abstraction possible: **the differences are all in
*acquisition*, not in *use*.** Every flow ends with a bearer token, an expiry,
and possibly a refresh token.

The existing implementation (`services/oauth_service.rs`) is a single 903-line
monolith that hardcodes the desktop flow, derives its issuer from a frontend
value, and reaches into `AppHandle` for keychain access.

## Options considered

**A. One provider with a `mode` enum.**
Rejected. The flows differ in *shape* (redirect vs polling vs static), not in
parameters. A mode enum becomes a `match` with unrelated arms.

**B. `AuthProvider` trait; one impl per flow.**
**Chosen.**

**C. Handle auth in each shell.**
Rejected. It is what the CLI would otherwise do, and it puts security-critical
token handling in four places.

## Decision

Define in `vilsend-core`:

- **`AuthProvider`** — `authenticate()`, `token()`, `logout()`, `status()`,
  `capabilities()`, `id()`.
- **`CredentialStore`** — namespaced by `{ provider, account, slot }`.
- **`InteractiveAuthUi`** — the shell's hook for interactive flows:
  `show_device_code()`, `open_browser()`, `await_redirect()`. **This is what
  keeps the core UI-free.**
- **`TokenRefresher`** — per-account lock + double-checked re-read.

Implementations:

| Impl | Notes |
|---|---|
| `ClerkPkceProvider` | Wraps the **existing** `OAuthService`. Phase 6 is a pure refactor. |
| `ClerkDeviceGrantProvider` | CLI. ⚠️ Requires enabling the beta grant on the Clerk OAuth application. |
| `ApiKeyProvider` | CLI/CI. See ADR-0009. |
| `ByoAuthProvider` | SDK. Host-supplied token callback. |

### Design rules

1. **`token()` is called per request and must be cheap when valid.** The current
   lazy, per-request refresh is correct — **do not replace it with a timer**.
2. **Preserve the existing two-tier refresh failure handling verbatim:**
   4xx/malformed → clear the session; transport/5xx → keep it and return the
   stale token. An offline laptop must not be signed out.
3. **Per-account locks, not one global lock.** Multi-account is deferred, but the
   key space and the lock granularity are designed for it now.
4. **The core never prompts.** `InteractiveAuthUi` is injected. `ApiKeyProvider`
   and `ByoAuthProvider` are **provably incapable** of opening a browser — a
   contract test asserts this. An SDK that hijacks its host's UI gets uninstalled.
5. **Adopt OIDC discovery** (`/.well-known/openid-configuration`) instead of
   string-concatenating `/oauth/authorize` and `/oauth/token`
   (`oauth_service.rs:83-85`). This also removes the reverse-engineered
   publishable-key parsing currently done in TypeScript
   (`src/lib/auth-config.ts:26-45`).

## Consequences

**Positive**

- The desktop app's Phase 6 change is **behaviour-preserving by construction** —
  the existing code is wrapped, not rewritten.
- The CLI proves the abstraction by adding a flow with a completely different
  shape (polling, no redirect).
- The SDK becomes possible without dragging Clerk into every embedder — the
  Clerk adapters are behind cargo features.

**Negative**

- ⚠️ The device grant is **beta** and **disabled by default per OAuth
  application**. If it cannot be enabled (**Q-03**), the CLI must fall back to
  loopback PKCE, which does not work on a truly headless machine.
- Device-code phishing has **no cryptographic defence**. The mitigation is UX
  (state clearly what is being authorized). Accepted risk.

**Neutral**

- Multi-account is designed for but **not shipped** in this migration.

## Related

- [`../03-authentication.md`](../03-authentication.md)
- ADR-0009 (service tokens), ADR-0006 (mobile)
- Q-01, Q-02, Q-03 in [`../07-risks-and-open-questions.md`](../07-risks-and-open-questions.md)
