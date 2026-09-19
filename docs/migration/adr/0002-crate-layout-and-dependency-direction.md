# ADR-0002 — Crate layout and dependency direction

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 2 (partially), 3, 5, 8
- **Supersedes:** —

## Context

Given ADR-0001 (extract a core), how many crates, and which way do dependencies
point?

The risk in both directions is real. Too few crates and the boundary is
decorative. Too many and the workspace becomes bureaucratic for a ~10k-line
codebase.

## Options considered

**A. One crate with modules.**
Rejected — see ADR-0001 option B.

**B. Two crates: `core` and everything else.**
Rejected. The transport and auth adapters have genuinely different dependency
sets (one needs `reqwest` + a process supervisor, the other needs `aes-gcm`),
and the CLI must be able to build without the Tauri adapters.

**C. Twelve crates, one per concern.**
**Chosen** — with an explicit "fold it in if it does not earn its place" rule.

**D. Crates split by target (desktop/cli/sdk).**
Rejected. That is a *packaging* split, not an architecture split, and it would
put business logic in the shells.

## Decision

Adopt this layout, with dependencies pointing strictly inward:

| Layer | Crates |
|---|---|
| Domain (depends on nothing) | `vilsend-core` |
| Adapters (depend on domain) | `vilsend-crypto`, `vilsend-protocol`, `vilsend-transport`, `vilsend-auth`, `vilsend-runtime` |
| Orchestration | `vilsend-engine` |
| Public API | `vilsend-sdk` |
| Shells | `vilsend-desktop`, `vilsend-cli`, `vilsend-ffi`, `vilsend-node` |

Rules:
1. `vilsend-core` depends on nothing but `serde`, `thiserror`, `async-trait`,
   `bytes`, `futures-core`, `tracing`.
2. No crate may depend on a shell.
3. `vilsend-engine` may depend on all adapters; adapters depend on nothing but
   `core`.
4. **A crate that needs a new dependency edge needs an ADR.**

**Explicit deviation:** `vilsend-crypto` starts **folded into
`vilsend-protocol`**. Split it only if a consumer needs crypto without the
protocol. Splitting preemptively is the over-engineering this ADR is meant to
avoid.

## Consequences

**Positive**

- `cargo tree -p vilsend-core` is a one-line CI assertion that the architecture
  holds.
- The CLI can build with `--no-default-features` and pull no Tauri code.
- A new transport is one crate-local module plus a feature flag.

**Negative**

- Twelve `Cargo.toml` files to maintain.
- Cross-crate refactors require touching several manifests.

**Neutral**

- The number is a *starting* point. Folding crates back together is cheap;
  splitting later is not — but the plan explicitly permits folding.

## Related

- [`../01-target-architecture.md`](../01-target-architecture.md) §4
- ADR-0001
