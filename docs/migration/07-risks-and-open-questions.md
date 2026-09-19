# 07 — Risks, Assumptions, and Open Questions

> Everything I could not determine from the code, and everything I need from you
> before implementation starts. Each item is actionable.

---

## Part A — Open questions

**These are the decisions I cannot make for you.** Ordered by how much they
block.

### Q-01 · Is the backend validating Clerk tokens, and how? 🔴 **Blocking**

**Why it matters:** `docs/AUTHENTICATION.md` already flags this as an
assumption. It determines whether the *current* desktop app works at all, and
it determines whether API keys can be adopted.

The desktop app receives an **OAuth application access token**, which is
different from the browser session token. Clerk configures the format per
OAuth application ("Generate access tokens as JWTs"):

- If **JWT** and the backend validates signature + `iss` + `exp` → works today.
- If **JWT** and the backend also validates `aud` → the backend must be
  configured to accept the **OAuth client ID** as an audience.
- If **opaque (`oat_…`)** and the backend expects a JWT → **the app is broken**,
  or the backend is validating only existence.

**What I need:**
1. How does the backend verify tokens? (Which library, which checks?)
2. Is the Clerk OAuth application issuing JWT or opaque access tokens?
3. Does the backend check `aud`?

**Impact if unanswered:** Phase 6 (auth abstraction) and Phase 7 (API keys)
cannot be designed correctly. Everything else is unaffected.

---

### Q-02 · Is `vilsend://auth/callback` allow-listed on the production Clerk instance? 🟡

Clerk requires production instances to allow-list native redirect URLs (Dashboard
→ Native applications → mobile SSO redirect allowlist, or the Backend API).
Clerk passes security-critical nonces **only to allow-listed URLs**.

**What I need:** confirmation that the live instance allow-lists
`vilsend://auth/callback`, and that the OAuth application has `public: true` and
`pkce_required: true`.

**Impact:** if it is not allow-listed, the desktop flow is fragile in ways that
would be hard to debug.

---

### Q-03 · Can we enable the Device Authorization Grant? 🟡 **Blocking for Phase 7**

Clerk's device grant is **beta and disabled by default per OAuth application**.
It must be turned on in the Dashboard or via
`PATCH /oauth_applications/{id}` with `{"device_authorization_grant_enabled": true}`.

**What I need:** confirmation that it can be enabled on your instance and that
you accept a beta dependency for the CLI. If not, the CLI falls back to
loopback PKCE (which does not work on a truly headless machine).

---

### Q-04 · What is Clerk's token revocation endpoint? 🟡

Logout currently deletes tokens locally and calls nothing, so **refresh tokens
stay valid server-side after sign-out**.

**What I need:** the `revocation_endpoint` from
`{issuer}/.well-known/openid-configuration`, or confirmation that Clerk does not
expose one for OAuth applications. **I have deliberately not guessed the URL.**

**Impact:** Phase 6.5.

---

### Q-05 · Are API keys viable, or should the backend mint its own service tokens? 🔴 **Blocking for Phase 7 CI**

| Option | Cost | Notes |
|---|---|---|
| **A. Clerk API Keys** | $100/mo Pro plan + per-op fees | `ak_` prefix, GA since 2026-04, backend uses `acceptsToken: 'api_key'` |
| **B. Backend-minted service tokens** (`vls_…`) | Backend work | No Clerk dependency, no plan requirement, full control over scopes and rotation |

**What I need:** your preference. **My recommendation: B.** You already own the
backend; a service token is ~a table, a hash, and a middleware. It avoids a plan
upgrade, avoids a beta/GA dependency for a load-bearing capability, and gives you
rotation semantics you control. Choose A only if you want to avoid backend work
at all costs.

---

### Q-06 · Is the central backend changeable, and by whom? 🔴 **Blocking**

Four phases need backend changes:

| Phase | Change |
|---|---|
| 4 | Acknowledge `proto_v2` in the handshake; optionally mint session tokens |
| 7 | Accept `Authorization: Bearer ak_…` / service tokens on transfer endpoints |
| 9 | **Mint and sign `SessionToken`s**; accept a device signing public key at registration |
| 10 | Additive `transport_hints` in `START_TRANSFER` |

**What I need:** is there a backend team? What is their lead time? **This is the
long pole of the entire plan** — start the conversation in Phase 1 even though
the first backend dependency does not land until Phase 7.

---

### Q-07 · What does the receiver's authorization actually need to protect against? 🟡

The current receiver accepts any present `Authorization` header and binds
`0.0.0.0:7878`. Two readings:

- **Benign:** the tunnel is the only route in, Cloudflare terminates TLS to a
  hostname the control plane owns, and the payload is E2E-encrypted anyway. The
  weakness is theoretical.
- **Adversarial:** any process on the LAN, any co-tenant on the host, and any
  malicious browser page can reach `:7878`.

**What I need:** how exposed is `7878` in production today — is it firewalled?
Is the machine ever on an untrusted network?

**Impact:** determines whether Phase 9 is P0-urgent or P1-important. **My
assumption is the adversarial reading**, and the plan reflects that.

---

### Q-08 · What is the actual product priority for mobile? 🟡

The plan orders mobile last (Phase 12). That is correct if the SDK/CLI are the
strategic bet. If mobile is actually the priority — e.g. because the market
demand is there — the order changes substantially, and the FFI work moves much
earlier.

**What I need:** relative priority of SDK vs CLI vs mobile over the next two
quarters.

---

### Q-09 · Who are the SDK's customers? 🟡

This changes the binding order more than anything else:

- **Node/TypeScript developers** → Phase 11 (napi-rs) is the right first binding.
- **Mobile app developers** → UniFFI + mobile shell first; napi-rs may never be needed.
- **Enterprise integrators on JVM/.NET** → a C ABI becomes worth considering.
- **"We'll see"** → build the Rust crate only, defer all bindings.

**What I need:** even a rough answer. **My assumption: Node/TS first**, because
it is the cheapest binding to build and the largest population of potential
integrators.

---

### Q-10 · Is the LAN transport actually wanted? 🟡

The plan makes it Phase 10, and it is the largest single new feature. It is
justified by (a) big speed wins, (b) privacy — payloads never transit Cloudflare,
(c) it is the natural first test of the transport abstraction.

**But** it also introduces: a Windows firewall prompt, mDNS on corporate
networks, a new attack surface, and a meaningful amount of new UI.

**What I need:** is "fast transfer between two devices on the same Wi-Fi" a
feature users are asking for? If not, Phase 10 can be replaced by a different
second transport (e.g. a VilSend-operated relay) that exercises the same
abstraction with a different risk profile.

---

### Q-11 · Do you have telemetry or support data? 🟢

`docs/IMPROVEMENT_ROADMAP.md` P3 suggests metrics. Any real data on transfer
failure rates, tunnel availability, or chunk retry frequency would let the
migration prioritise on evidence rather than on my reading of the code.

**What I need:** crash reports, support tickets, or any error-rate data. If
none, that is itself a finding — Phase 6 of the architecture doc proposes a
`Telemetry` port for exactly this reason.

---

## Part B — Assumptions I made

Stated so you can correct them. Every one of these is load-bearing somewhere.

| # | Assumption | If wrong |
|---|---|---|
| A-01 | The tunnel is the *typical* route in production, so `:7878` is not directly internet-exposed | Phase 9's urgency changes (down), but not its necessity |
| A-02 | The central backend **can** be changed (Q-06) | Phases 7, 9 degrade to client-only interims; peer auth becomes impossible to do properly |
| A-03 | The installed base (v1.0.4) must keep working through every phase | If a forced-upgrade path is acceptable, Phases 4 and 9 get much simpler |
| A-04 | Clerk remains the identity provider | The `AuthProvider` abstraction absorbs this — it is exactly why the abstraction exists |
| A-05 | One engineer works this plan, or a small team | Parallelism would reorder phases (see [`05`](./05-migration-plan.md) dependency graph) |
| A-06 | The desktop app's current feature set must not regress | Any intentional behaviour change needs an explicit decision |
| A-07 | Node/TS is the first SDK target (Q-09) | Binding order changes |
| A-08 | The `docs/` folder's 2026-09-05 assessment is a fair baseline and I have only corrected it where the code disagrees | — |
| A-09 | "Adding a new transport without modifying existing code" is a real requirement, not aspirational | If it is aspirational, Phase 8's contract suite is over-engineering and can be cut |
| A-10 | The receiver's `Authorization` check is genuinely exploitable, not dead code behind a firewall | Phase 9 drops from P0 to P1 |

---

## Part C — Risk register

**Likelihood** L/M/H · **Impact** L/M/H · **Severity** = the product of the two.

### Technical risks

| ID | Risk | L | I | Sev | Mitigation | Owner phase |
|---|---|---|---|---|---|---|
| R-01 | **Backend changes slip** — 4 phases depend on a codebase not in this repo | **H** | **H** | 🔴 | Start in Phase 1; design client-side interims (task 9.3) so the client is never blocked | 7, 9 |
| R-02 | **No tests exist**, so the refactor is unverifiable | **H** | **H** | 🔴 | Phase 2 stands up the harness; characterisation tests before any behaviour change | 2–5 |
| R-03 | **AAD/protocol v2 breaks the installed base** | M | **H** | 🔴 | v2 is additive and lives on new endpoints; a CI compatibility test against a pinned v1 binary | 4 |
| R-04 | **Windows firewall prompt** on first LAN use terrifies or blocks users | **H** | M | 🟠 | Bind only when a LAN transfer is desired; explain in UI; consider an installer rule | 10 |
| R-05 | **iOS background transfer does not fit Tauri mobile** | M | **H** | 🟠 | **Timeboxed spike before committing**; keep the UniFFI escape hatch (task 12.8) | 12 |
| R-06 | **The workspace move breaks the release pipeline** | M | **H** | 🟠 | One atomic commit; dry-run on a throwaway tag; update the CI cache path in the same commit | 2 |
| R-07 | **Device grant is beta** and Clerk changes it | M | M | 🟠 | Keep loopback PKCE as a fallback; isolate behind `AuthProvider` | 7 |
| R-08 | **Cloudflare tunnel is a single point of failure** for the majority of transfers | M | **H** | 🟠 | LAN transport (Phase 10) reduces exposure; a relay transport is the strategic answer | 10 |
| R-09 | **macOS is unsigned/notarized** — Gatekeeper warns users | **H** | M | 🟠 | Out of scope here but should be scheduled; an unsigned macOS build undermines the whole product | — |
| R-10 | **Cargo dependency cycles** appear once crates split | M | M | 🟡 | Enforce the rules in CI from day one ([`01`](./01-target-architecture.md) §4.2) | 2 |
| R-11 | **The port abstraction is wrong** and Phase 10's transport does not fit | M | M | 🟡 | The `MockTransport` test in Phase 8 is the canary — if it needs special-casing, fix the abstraction then | 8 |
| R-12 | **Scope creep**: the refactor becomes a rewrite | M | **H** | 🟠 | Phase 2 is a **pure** refactor; notarize that a phase is "done" per [`06`](./06-testing-and-quality.md) §10 | — |
| R-13 | **iOS/Android build times** make CI painful (6 targets) | **H** | L | 🟡 | Decouple core releases from UI releases via xcframework/.aar | 12 |
| R-14 | **A LAN transport makes the receiver-auth hole exploitable** | **H** | **H** | 🔴 | **Phase 9 strictly precedes Phase 10.** Non-negotiable ordering | 9 |

### Organisational risks

| ID | Risk | L | I | Sev | Mitigation |
|---|---|---|---|---|---|
| R-15 | The migration stalls halfway, leaving two architectures | M | **H** | 🟠 | Every phase ships; the desktop app works throughout; a paused migration is a *coherent* state, not a broken one |
| R-16 | Bus factor — the architecture lives in one head | M | **H** | 🟠 | These documents; ADRs; a mandatory `CLAUDE.md`/`CONTRIBUTING` update |
| R-17 | User-visible regression during the move erodes trust | M | M | 🟡 | Phase 2 changes no behaviour; enforce per-phase acceptance criteria |
| R-18 | The SDK ships before it is ready and acquires a bad reputation | M | **H** | 🟠 | `publish = false` until Phase 7 is complete; semver-checks; an API snapshot in review |

### Security risks

| ID | Risk | L | I | Sev | Mitigation | Phase |
|---|---|---|---|---|---|---|
| R-19 | Receiver auth hole exploited | M | **H** | 🔴 | Phase 9 (interim in 9.3 if the backend lags) | 9 |
| R-20 | Committed `TAURI_SIGNING_PRIVATE_KEY` in `.env` | — | **H** | 🔴 | Phase 1.2 — **remove from the repo; rotate out-of-band with a dual-key transition** | 1 |
| R-21 | Devtools enabled in production builds | **H** | M | 🟠 | Phase 1.1 | 1 |
| R-22 | **No AAD** — chunks replayable across files/indices | M | **H** | 🟠 | Phase 4.1 | 4 |
| R-23 | **No whole-file integrity check** — silent corruption undetected | M | **H** | 🟠 | Phase 4.3 | 4 |
| R-24 | Refresh token valid after logout (no revocation) | **H** | M | 🟠 | Phase 6.5 (**verify the endpoint first**) | 6 |
| R-25 | Token material in logs | M | **H** | 🟠 | Phase 1.5 + the redaction test ([`06`](./06-testing-and-quality.md) §6.4) | 1 |
| R-26 | 20 npm vulnerabilities (9 high) | M | M | 🟡 | `npm audit fix`; add `cargo deny` + `npm audit` to CI | 3 |
| R-27 | A leaked API key is a full account takeover | M | **H** | 🟠 | Scope API keys; short expiry; **authorize on scopes on the backend** | 7 |
| R-28 | Device-code phishing (user approves an attacker's device) | M | M | 🟡 | **No cryptographic defence exists.** Mitigate with UX: state clearly what is being authorized | 7 |

---

## Part D — Top 5 decisions I need from you

If you read nothing else:

1. **Q-01 — JWT or opaque access tokens, and does the backend check `aud`?**
   Determines whether the current app works and how Phase 6/7 are built.
2. **Q-06 — Can the backend change, and when?** The long pole. Four phases
   depend on it. Start the conversation now.
3. **Q-05 — Clerk API Keys, or backend-minted service tokens?**
   **I recommend backend-minted.** It avoids a $100/mo plan and a GA dependency
   for a capability you can own in a table and a middleware.
4. **Q-10 — Is the LAN transport actually wanted?** It is the largest new
   feature in the plan, and it is the thing the transport abstraction exists to
   enable. If the answer is no, Phase 10 should be replaced by a relay transport.
5. **Q-08/Q-09 — Priority order of SDK vs CLI vs mobile, and who the SDK's
   customers are.** Changes the binding order and the whole back half of the
   plan.

---

## Part E — What I am confident about

So the risk register above is read in proportion:

- **The extraction seam is narrow.** Only three concerns weld the transfer
  engine to Tauri; the `websocket/` and `utils/` modules are already clean.
  ([`00`](./00-current-state.md) §3.4)
- **The PKCE and refresh implementations are genuinely good.** Well-structured,
  correct constant-time comparison, correct two-tier refresh failure handling.
  Keep them; wrap them. ([`00`](./00-current-state.md) §5.4)
- **The frontend's token handling is right.** The access token never enters
  React state; it is fetched per-request from Rust.
- **The order is right.** Peer auth before LAN, abstraction before
  implementation, tests with the seam.
- **Every phase can ship.** The desktop app has no phase in which it is broken.

---

## See also

- [`05-migration-plan.md`](./05-migration-plan.md) — where each risk is addressed
- [`00-current-state.md`](./00-current-state.md) — the evidence behind each finding
- [`03-authentication.md`](./03-authentication.md) §8 — the Clerk verification matrix
