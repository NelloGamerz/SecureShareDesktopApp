# Improvement Roadmap

## P0: fix immediately

| Problem | Solution | Complexity | Benefit |
|---|---|---:|---|
| Receiver accepts any present Authorization header | Validate short-lived transfer-scoped credential and bind it to transfer/device/expiry | Difficult | Prevent unauthorized sessions and abuse |
| Receiver exposed on `0.0.0.0:7878` | Bind least-privilege interface or enforce firewall/tunnel policy | Moderate | Reduce attack surface |
| Fixed Stronghold password | Remove unused plugin or use platform-protected secret | Moderate | Protect stored secrets |
| Missing quotas/expiry/cleanup | Add limits, deadlines, active-session caps, and guaranteed cleanup | Moderate | Prevent disk/memory exhaustion |

## P1: high impact

| Problem | Solution | Complexity | Benefit |
|---|---|---:|---|
| Global chunk write lock | Use per-transfer/file locks and atomic counters | Moderate | Improve concurrent throughput |
| No restart recovery | Persist transfer/session/chunk state in SQLite | Difficult | Resume after crash/sleep |
| UI owns service lifecycle | Add one idempotent Rust `sync_services` façade | Moderate | Fewer IPC calls and race conditions |
| No transfer end-to-end tests | Add Rust receiver tests and desktop integration tests | Moderate | Catch security/regression failures |

## P2: important

| Problem | Solution | Complexity | Benefit |
|---|---|---:|---|
| Credential diagnostics | Remove token logs; use transfer/device correlation IDs | Easy | Safer debugging |
| Null CSP/devtools in production | Define CSP and disable devtools for release | Easy | Harden desktop surface |
| Unbounded WebSocket messages | Retain bounded history and classify events | Easy | Stable memory use |
| Stale commented Rust code/warnings | Delete obsolete blocks and resolve warnings | Moderate | Better maintainability |

## P3: nice to have

- Benchmark chunk/concurrency combinations per platform and network type.
- Add metrics for transfer throughput, retries, queue depth, and tunnel readiness.
- Add signed release artifact verification and platform smoke tests.
- Generate an API schema from the central backend and validate client contracts.

## Migration order

Start with receiver authentication and exposure controls, then add tests that freeze the current intended protocol. Introduce persisted transfer sessions and per-transfer scheduling behind the existing `UploadManager` API. Finally simplify lifecycle orchestration and clean stale code. Do not rewrite the stack; retain Tauri, Rust, and the current cryptographic primitives while improving protocol authentication and operational controls.
