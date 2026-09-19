# ADR-0012 — Events as a port; progress as a stream

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 2 (the walking skeleton), 5
- **Supersedes:** —

## Context

Transfer progress reaches the UI through exactly one seam:
`transfer/events.rs:2` — `use tauri::{AppHandle, Emitter}` and
`app.emit(name, progress)`. This is also the **only** Tauri coupling in the
transfer module that is not about storage.

Two problems:

1. **Event names are string literals scattered across call sites** —
   `"transfer-progress"`, `"transfer-completed"`, `"transfer-failed"`,
   `"transfer-paused"`, `"transfer-resumed"`, `"transfer-cancelled"` appear in
   `transfer/manager.rs:160-198` and throughout `transfer/writer.rs`. A typo in
   one is a silently dead listener in the UI, with no compile error and no
   runtime error.
2. **The seam is Tauri-shaped.** A CLI, SDK, or mobile shell cannot reuse it.

This is also the smallest possible change that establishes the whole ports
pattern — which is why it is the walking skeleton.

## Options considered

**A. Keep `app.emit`; add a CLI wrapper that subscribes to Tauri events.**
Rejected. It would require a Tauri runtime in the CLI.

**B. A callback-based `EventSink` trait with typed events.**
**Chosen** for the sink.

**C. A `futures::Stream` for progress.**
**Chosen** for the SDK surface, layered on top of B.

**D. A global event bus / channel.**
Rejected. Global mutable state in a library is a defect, not a feature — see
the existing `Logger::init()` calling `try_init()` on a global subscriber
(`utils/logger.rs:4-10`), which is already commented out of `lib.rs:55` for
exactly this reason.

## Decision

### 1. A typed `DomainEvent` enum in `vilsend-core`

```rust
#[non_exhaustive]
pub enum DomainEvent {
    TransferProgress { id: TransferId, bytes: u64, total: u64, /* ... */ },
    TransferCompleted { id: TransferId, outcome: Outcome },
    TransferFailed { id: TransferId, error: ErrorKind },
    TransferPaused { id: TransferId },
    TransferResumed { id: TransferId },
    TransferCancelled { id: TransferId },
    TransportDegraded { id: TransferId, from: TransportKind, to: TransportKind },
    AuthStateChanged { state: AuthState },
}
```

**Not** a Tauri event name. The wire name lives in the shell, in one exhaustive
`match` — so renaming an event becomes a compile error rather than a dead
`listen()` in the UI.

### 2. `EventSink` port

```rust
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: DomainEvent);
}
```

| Shell | Implementation |
|---|---|
| Desktop | `TauriEventSink` — maps to `app.emit(name, payload)`. **Wire format unchanged.** |
| CLI | NDJSON on stdout, or a progress bar on stderr |
| SDK | Feeds the `EventStream` |
| FFI | A UniFFI callback interface |
| Tests | `RecordingEventSink` |

### 3. SDK exposes a `Stream`, not a callback

```rust
pub fn events(&self) -> impl Stream<Item = DomainEvent> + Send + 'static;
```

Rationale: streams compose (`select!`, `merge`, `throttle`), apply backpressure
naturally, and map cleanly onto every binding target — Node `AsyncIterator`,
Swift `AsyncSequence`, Kotlin `Flow`, Python async generator. A callback API
forces every binding to re-implement backpressure and re-entrancy protection.

### 4. `TransportDegraded` is a first-class event

Silently falling back from LAN to tunnel is exactly what makes downgrade attacks
invisible (ADR-0007). Surfacing the degradation is a security control, not a
UX nicety.

## Consequences

**Positive**

- The walking skeleton is ~30 lines of change and establishes the entire
  pattern for every later phase.
- Event-name typos become compile errors.
- One seam serves four shells.
- `RecordingEventSink` makes event sequencing testable — currently impossible.

**Negative**

- One extra `match` to maintain per shell.
- The `DomainEvent` enum is `#[non_exhaustive]`, so shells need a catch-all arm.

**Neutral**

- **Phase 2 must be a pure refactor.** The frontend's `listen("transfer-progress")`
  calls and the payload shapes must be byte-identical before and after. This is
  the acceptance criterion that proves the skeleton is sound.

## Related

- [`../01-target-architecture.md`](../01-target-architecture.md) §4.3, §5
- [`../06-testing-and-quality.md`](../06-testing-and-quality.md) §4
- ADR-0001, ADR-0011
