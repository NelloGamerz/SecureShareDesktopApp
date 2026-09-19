# ADR-0005 — SDK bindings: Rust crate first, then napi-rs, then UniFFI

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 4 (Rust), 11 (Node), 12 (Swift/Kotlin)
- **Supersedes:** —

## Context

The SDK should be embeddable by third-party developers. The question is which
binding targets to build, in what order, and with which toolchains.

Each binding is a permanent liability: a second crate, an ABI to keep stable,
per-architecture CI, and a release pipeline. Building four speculatively is how
projects acquire four half-maintained bindings.

## Options considered

**A. Build all bindings up front (Node, Swift, Kotlin, WASM, C ABI).**
Rejected. The API will move. Every binding built before the API stabilises is a
binding that must be rewritten.

**B. UniFFI for everything, including Node.**
Considered seriously. `uniffi-bindgen-react-native` now generates Node bindings
and `@ubjs/node` provides a generic N-API runtime. **Not mature enough to bet
on today**, but worth revisiting at Phase 11 — the switching cost is one crate.

**C. Rust crate → CLI → napi-rs → UniFFI. Defer WASM and C ABI.**
**Chosen.**

**D. WASM as the universal binding.**
Rejected. The core needs real sockets and real files. A browser target requires
an entirely different transport family (WebRTC/HTTP). That is a different
product, not a binding.

## Decision

Build in this order:

| # | Target | Tool | Phase |
|---|---|---|---|
| 1 | Rust crate | — | 4 |
| 2 | CLI | `clap` | 7 |
| 3 | Node / TS | **napi-rs** | 11 |
| 4 | Swift / Kotlin | **UniFFI** | 12 |
| 5 | WASM | `wasm-bindgen` | **deferred indefinitely** |
| 6 | C ABI | `cbindgen` | **deferred until asked** |

Rationale for the ordering:

- **Rust first, always.** The desktop shell is the first real consumer. If the
  API is awkward for your own shell, it is awkward for everyone.
- **CLI second.** Not a binding — a shell. Ordering it second is a deliberate
  test that the core has no hidden UI dependency, and it exercises the headless
  auth path.
- **Node third.** Near-native performance, production-proven (SWC, Rspack,
  Biome), and async iterators map cleanly onto the event stream.
- **Swift/Kotlin fourth.** Powers Firefox's Rust components. **Caveat: the
  Kotlin/JVM marshalling path carries materially higher overhead than Swift.**
  Irrelevant for a per-call file-transfer API; relevant if you ever expose a
  per-byte or per-event hot path.

**Do not publish the Rust crate to crates.io until Phase 7 is complete.**
Keep `publish = false`. The CLI is the proof it is ready.

## Consequences

**Positive**

- Bindings are built only once the API has settled, against a proven surface.
- Node comes first because it is the cheapest binding and the largest population
  of potential integrators (assumption A-07 / Q-09).
- Deferring WASM and C ABI avoids two permanent liabilities with no known
  customer.

**Negative**

- If an early integrator needs JVM/.NET, there is no C ABI to build on. The
  answer is UniFFI-plus-a-thin-wrapper, or revisiting this ADR.
- napi-rs prebuilt addon publishing across 3 OSes × 2 arches is fiddly
  (**R-13**).

**Neutral**

- UniFFI and napi-rs are separate crates wrapping the same SDK. That is the
  standard pattern; the duplication is a thin marshalling layer, not logic.

## Related

- [`../04-sdk-cli-mobile-build-plan.md`](../04-sdk-cli-mobile-build-plan.md) §3
- ADR-0006 (mobile shell) — UniFFI is also the mobile escape hatch
- Q-09 in [`../07-risks-and-open-questions.md`](../07-risks-and-open-questions.md)
