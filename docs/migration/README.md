# VilSend Migration Documentation

Architecture and migration plan for turning this Tauri desktop app into a
platform that ships the same core as a **desktop app, SDK, CLI, and mobile app**.

**Read this file first.** It gives the reading order and the ten-line summary.

> **Status:** proposal. Nothing in this folder has been implemented.
> **Scope:** documentation only — no application code, config, or dependency was
> changed to produce it.
> **Baseline:** repository at commit `7c6b82b`, branch `main`, 2026-09-19.

---

## The recommendation in ten lines

1. Split into a Cargo workspace with a **platform-agnostic `vilsend-core`** that
   depends on nothing — no Tauri, no HTTP, no OS. Enforce it with `cargo tree` in CI.
2. Tauri desktop, CLI, SDK, and mobile become **thin shells** over that core.
3. Make the **transport a port** (`Transport` + a byte-oriented `DuplexStream`),
   so LAN, P2P, relay, and Bluetooth can be added **without modifying existing code**.
4. A `TransportSelector` picks and switches transports by **capability scoring**
   (LAN > P2P > tunnel > relay), with racing, health checks, and mid-transfer failover.
5. Keep the existing crypto primitives — X25519 + HKDF-SHA256 + AES-256-GCM — but
   **add associated data**, a whole-file integrity check, and a **device signature**
   that authenticates the peer.
6. Make **auth a port** (`AuthProvider` / `CredentialStore` / `InteractiveAuthUi`),
   with one flow per client: PKCE for desktop/mobile, device grant for the CLI,
   API/service tokens for CI, bring-your-own-auth for the SDK.
7. The existing PKCE and refresh implementations are **good — wrap them, do not rewrite them.**
8. Ship the **Rust crate first**, then the CLI, then napi-rs for Node, then UniFFI.
   Defer WASM and a C ABI until someone asks.
9. **Mobile: Tauri 2 mobile**, with UniFFI built alongside as an escape hatch.
10. Migrate by **strangler pattern in 12 phases** — the desktop app ships at the
    end of every single one.

---

## Reading order

| Order | Document | Read it when |
|---|---|---|
| 1 | **[00-current-state.md](./00-current-state.md)** | First, always. What exists today, with file-path evidence. |
| 2 | **[01-target-architecture.md](./01-target-architecture.md)** | The principles, crate layout, ports, and SDK surface. |
| 3 | **[02-transport-layer.md](./02-transport-layer.md)** | The `Transport` port, selector, state machines, security. |
| 4 | **[03-authentication.md](./03-authentication.md)** | Per-client auth flows, threat models, Clerk capabilities. |
| 5 | **[04-sdk-cli-mobile-build-plan.md](./04-sdk-cli-mobile-build-plan.md)** | Public API, bindings, build matrix, packaging. |
| 6 | **[05-migration-plan.md](./05-migration-plan.md)** | **The plan.** 12 phases, each with acceptance criteria and rollback. |
| 7 | **[06-testing-and-quality.md](./06-testing-and-quality.md)** | Contract suites, in-memory adapters, CI gates. |
| 8 | **[07-risks-and-open-questions.md](./07-risks-and-open-questions.md)** | **What I need from you.** Read at least Part D. |
| 9 | **[adr/](./adr/)** | As needed. One record per major decision. |

### If you only have fifteen minutes

Read this README, then **Part D of `07-risks-and-open-questions.md`** (the five
decisions you need to make), then **`05-migration-plan.md`'s phase overview**.

### If you only want the security findings

Go to **`00-current-state.md` §3.3, §4.3, and §11**.

---

## The documents

### [00-current-state.md](./00-current-state.md)

What was actually found, verified against the code. Includes:

- Tauri **v2** (resolved 2.11.5), 32 commands, 11 events, one crate, **zero tests**.
- The **complete** command and event surface, plus 7 boundary defects found
  (including `stop_websocket` being invoked but never registered).
- Exact crypto: X25519 + HKDF-SHA256 + AES-256-GCM, **no AAD**, **no whole-file
  integrity**, `transfer/checksum.rs` a 0-byte stub.
- The auth reality: hand-rolled PKCE, **no OIDC discovery, no JWKS, no JWT
  verification**, issuer derived in TypeScript from an undocumented key format.
- **Three token-storage stacks exist on paper; only one is live.** Stronghold and
  the `keyring` crate are dead weight; `secure_storage.rs` (657 lines) is 100%
  commented out.
- Corrections to the existing `docs/` where they have gone stale.

### [01-target-architecture.md](./01-target-architecture.md)

Principles, C4 context/container/component diagrams, the 12-crate workspace,
dependency rules, the port traits, the public SDK surface, feature flags, the
build matrix, the mobile decision, and cross-cutting concerns (errors,
observability, config, testing, performance).

Includes an honest **"what to defer"** section: where a clean design would be
over-engineering at your current stage.

### [02-transport-layer.md](./02-transport-layer.md)

The `Transport` port and its invariants, `LanTransport` and `TunnelTransport`,
the handshake/negotiation protocol, the `TransportSelector` with capability
scoring, connect and failover state machines (Mermaid), how to add a transport
(step-by-step checklist), the security model — **including downgrade-attack
prevention** — and the v1/v2 backward-compatibility matrix.

### [03-authentication.md](./03-authentication.md)

`AuthProvider` / `CredentialStore` / `TokenRefresher` / `InteractiveAuthUi`,
per-client flows with sequence diagrams, token lifecycle, scopes, backend
verification requirements, and a **threat model per flow**. Ends with a
**verified Clerk capability matrix** — every claim marked as verified or not.

### [04-sdk-cli-mobile-build-plan.md](./04-sdk-cli-mobile-build-plan.md)

The public API in full, the four verbs, the async/stream/error models, the
versioning policy, the bindings order with trade-offs, the build matrix, the two
existing build bugs to fix first, packaging, the CLI design, and the mobile plan
with its eight concrete blockers.

### [05-migration-plan.md](./05-migration-plan.md)

Twelve phases, each with goal, exact tasks, files affected, acceptance criteria,
required tests, risks, rollback plan, and effort. Plus a phase dependency graph
and the **walking-skeleton milestone** (Phase 2).

### [06-testing-and-quality.md](./06-testing-and-quality.md)

The contract-suite strategy, in-memory test doubles, what to test per layer,
the CI pipeline split, the architecture lint, and the tests that would have
caught the existing bugs.

### [07-risks-and-open-questions.md](./07-risks-and-open-questions.md)

Eleven open questions, ten stated assumptions, and a 28-item risk register split
across technical, organisational, and security risks.

---

## Architecture Decision Records

| ADR | Decision | Phase |
|---|---|---|
| [0001](./adr/0001-extract-a-platform-agnostic-core.md) | Extract a platform-agnostic core | 2 |
| [0002](./adr/0002-crate-layout-and-dependency-direction.md) | Crate layout and dependency direction | 2–8 |
| [0003](./adr/0003-unified-error-model.md) | Unified typed error model across FFI and CLI | 2 |
| [0004](./adr/0004-transport-abstraction.md) | Model the transport as a port | 8 |
| [0005](./adr/0005-bindings-strategy.md) | Rust → CLI → napi-rs → UniFFI; defer WASM and C ABI | 4–12 |
| [0006](./adr/0006-mobile-shell.md) | Tauri 2 mobile, with UniFFI as an escape hatch | 12 |
| [0007](./adr/0007-peer-authentication.md) | Peer authentication via a device signing key | 9 |
| [0008](./adr/0008-auth-provider-per-client.md) | One `AuthProvider` port, a different flow per client | 6 |
| [0009](./adr/0009-service-tokens-and-receiver-auth.md) | Service tokens and receiver session authorization | 7, 9 |
| [0010](./adr/0010-protocol-versioning-and-backcompat.md) | Protocol versioning and backward compatibility | 4 |
| [0011](./adr/0011-testing-and-contract-suites.md) | Contract test suites and in-memory adapters | 2–8 |
| [0012](./adr/0012-events-as-a-port.md) | Events as a port; progress as a stream | 2 |

---

## The single most important constraint

**The desktop app must ship at the end of every phase.**

Everything in this plan is ordered around that. It is why:

- Phase 2 is a **pure refactor** with no behaviour change.
- Phase 8 builds the transport abstraction with **only the transport you already
  have**, before adding a new one.
- Phase 9 (peer authentication) **strictly precedes** Phase 10 (LAN), because
  LAN without it would make an existing hole exploitable.
- Every phase has a rollback plan.

If a phase cannot ship the desktop app, the phase is too big — split it.

---

## The five things blocking implementation

Detailed in [`07-risks-and-open-questions.md`](./07-risks-and-open-questions.md)
Part D:

1. **Q-01** — Is the backend validating Clerk tokens, and does it expect JWT or
   opaque access tokens? *(Determines whether the current app works at all.)*
2. **Q-06** — Can the central backend be changed, and by whom? *(Four phases
   depend on it. It is the critical path.)*
3. **Q-05** — Clerk API Keys, or backend-minted service tokens? *(I recommend
   backend-minted — it avoids a $100/mo plan and a vendor dependency for a
   capability you can own.)*
4. **Q-10** — Is the LAN transport actually wanted? *(It is the largest new
   feature in the plan.)*
5. **Q-08/Q-09** — Priority order of SDK vs CLI vs mobile, and who the SDK's
   customers are. *(Changes the binding order and the back half of the plan.)*

---

## Relationship to the existing `docs/`

The repository already contains 19 documents in `../` (written 2026-09-05).
They are unusually good — honest, specific, and self-critical — and this set
**builds on them rather than replacing them**.

Where the existing docs and the code disagree, **the code wins**, and the
discrepancies are listed in `00-current-state.md` §10. The most significant:
the docs describe a 30-second token refresh timer and a `login` command; neither
exists, and the actual implementation is better than the docs describe.

---

## Conventions used in this set

| Marker | Meaning |
|---|---|
| **NOT FOUND** | I looked and it is not there |
| **ASSUMPTION** | I inferred it; verify before relying on it |
| ✅ / ❌ / ⚠️ | Verified / verified absent / unverified |
| `requires backend change` | Needs work outside this repository |
| `[file.rs:12](...)` | Clickable evidence for a claim about current code |

**Rules this set follows:** every claim about current code cites a real file
path; anything else is labelled as an assumption; the code is never quoted as
correct without saying why; and where a "clean" design would be
over-engineering, that is stated rather than hidden.
