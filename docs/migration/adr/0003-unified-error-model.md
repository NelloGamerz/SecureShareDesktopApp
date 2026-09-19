# ADR-0003 — Unified, typed error model across FFI and CLI

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 2
- **Supersedes:** —

## Context

Two error types coexist with no mapping discipline:

- `AppError` (`src-tauri/src/error.rs:3-23`) — 7 variants, Tauri-facing.
- `TransferError` (`src-tauri/src/transfer/errors.rs:3-10`) — 6 variants,
  including `Receiver(String)`, `Crypto(String)`, `Invalid(String)`.

Two problems:

1. **`AppError` serializes to the frontend as a flat string.** Its `Serialize`
   impl (`error.rs:53-60`) calls `Display`. Every discriminant is lost at the
   IPC boundary — the UI cannot distinguish "not signed in" from "network
   down" without string matching.
2. **`TransferError::Receiver(String)`** collapses "wrong credential",
   "insufficient storage", and "peer crashed" into one opaque string. The
   current receiver even returns `507 INSUFFICIENT_STORAGE` as an HTTP status
   (`transfer/writer.rs`), which the sender then discards into a `String`.

With four shells, this becomes four different string-parsing hacks.

## Options considered

**A. Keep strings; parse them per shell.**
Rejected. It is what happens today and it is already broken.

**B. Per-crate error types with `From` conversions.**
Rejected as the *boundary* type. Internal to a crate, fine; at the FFI edge, it
means the binding layer must understand every crate's taxonomy.

**C. One `#[non_exhaustive]` enum with a stable machine-readable `kind()`.**
**Chosen.**

**D. Use a crate like `snafu` or `miette` with context chains.**
Rejected for the boundary, though `miette`-style context is fine for *human*
output in the CLI. The FFI boundary needs a flat, stable, matchable taxonomy.

## Decision

Define `VilsendError` in `vilsend-core` with:

- `#[non_exhaustive]` — new variants are not breaking for consumers.
- A `kind() -> ErrorKind` accessor that is **stable across versions** and is the
  **only** thing shells may switch on.
- No `String`-payload variant at the top level except `Internal`.

Mapping is done **once per shell**, at the boundary:

| `ErrorKind` | CLI exit | gRPC-ish | Swift/Kotlin |
|---|---|---|---|
| `Unauthenticated` | 3 | `UNAUTHENTICATED` | sealed-class variant |
| `NoRoute` | 4 | `UNAVAILABLE` | variant |
| `IntegrityMismatch` | 5 | `DATA_LOSS` | variant |
| `InsufficientStorage` | 6 | `RESOURCE_EXHAUSTED` | variant |
| `Cancelled` | 130 | `CANCELLED` | variant |
| everything else | 1 | `INTERNAL` | `Internal(msg)` |

**Rule: a message string may never be the only signal crossing a boundary.**

## Consequences

**Positive**

- The Tauri IPC boundary can serialize `{ kind, message }` and the UI can
  branch on `kind` without string matching.
- UniFFI generates sealed classes / Swift enums for free.
- CLI exit codes become testable and stable.

**Negative**

- Requires touching every `?` site during Phase 2–3 as `AppError` disappears.
- `#[non_exhaustive]` means consumers must have a catch-all arm, which is a
  small ergonomic cost in exchange for evolvability.

**Neutral**

- `Internal(Box<dyn Error>)` is deliberately opaque. It exists so the core never
  needs a variant it cannot name — not as a general escape hatch. **Adding a
  new `Internal` case instead of a real variant should be rejected in review.**

## Related

- [`../01-target-architecture.md`](../01-target-architecture.md) §8.1
- [`../04-sdk-cli-mobile-build-plan.md`](../04-sdk-cli-mobile-build-plan.md) §7.1
