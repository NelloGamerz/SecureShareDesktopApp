# Performance Review

## Current design

React lazy-loads most feature pages and uses TanStack Query. Rust performs file reads and network I/O asynchronously. Default transfer settings are 4 MiB chunks, four concurrent workers, and four retries. Progress crosses the Rust/UI boundary as events.

## Likely bottlenecks

1. `writer.rs` holds one global async mutex around directory creation, part write, and completion detection, serializing chunks from otherwise independent transfers.
2. Each worker allocates a chunk-sized read buffer and an AES-GCM ciphertext, so baseline in-flight memory is roughly `concurrency * (chunk + encrypted chunk)` plus receiver buffers.
3. Receiver completion checks every part with synchronous `std::fs::metadata` and merge rereads every part, adding filesystem overhead.
4. `WebSocketProvider` retains every message indefinitely and can grow without bound.
5. `useDesktopServices` performs repeated status IPC calls whenever auth/profile dependencies change.
6. Cloudflared startup uses blocking process and sleep loops inside a synchronous command.
7. Transfer progress is emitted for each successful chunk, which can be noisy for many small chunks.

## Recommendations

Use per-transfer/per-file locks, track received chunk indices in state, and perform completion checks from counters. Stream chunks where practical, bound message history, coalesce progress events, and expose one idempotent Rust service-sync command. Move Cloudflared readiness waiting to an async task with cancellation. Benchmark 1/4/16 MiB chunks and concurrency 2/4/8 on LAN and tunnel paths before changing defaults.

## Measurement gaps

No metrics, profiling configuration, transfer benchmark, memory budget, or request correlation system is present. Claims about central API/database performance, N+1 queries, Redis, or Spring thread pools cannot be made because that code is not in repository.
