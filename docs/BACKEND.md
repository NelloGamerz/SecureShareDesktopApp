# Backend and Local Services

## Scope

No Spring Boot, Java, Maven, Gradle, Docker, or central backend source is present in this repository. The phrase “backend” here refers to the embedded Rust services and the external API/WebSocket contracts consumed by the desktop app.

## Embedded Rust services

- `AuthService`: stores the current Clerk token and auth flags in process memory.
- `WebSocketService` and `WebSocketManager`: connect to the configured central WebSocket, attach Bearer JWT and device ID, heartbeat, reconnect with backoff, and dispatch messages.
- `CloudflaredService`: loads tunnel credentials, launches/stops the bundled Cloudflared process, and detects a live connection from stderr text.
- `LocalTransferFileService`: creates and queries the SQLite `local_transfer_files` table.
- `UploadManager`: coordinates scanning, key agreement, chunk scheduling, retries, progress, pause/resume/cancel, and events.
- Axum receiver in `transfer/writer.rs`: performs handshake, derives the transfer key, decrypts chunks, writes parts, merges them, and emits progress.

## Central service contract inferred from calls

The frontend calls `/devices`, `/devices/health`, `/devices/{id}`, `/devices/register`, `/devices/{id}/rename`, `/devices/{id}`, `/devices/pair`, `/devices/pair/{code}`, `/devices/pair/{code}/connect`, `/devices/pair/{code}/cancel`, `/transfers`, and `/transfers/{id}`. Exact server implementation, authorization, persistence, and response validation are not found in repository.

## Lifecycle and failure handling

WebSocket reconnection uses internet checks, heartbeat, bounded attempts, and exponential-style delays. Cloudflared startup blocks a synchronous command thread for up to 15 seconds while waiting for a log line. Transfer retries use delays of 1, 2, 4, and 8 seconds, with four default retries.

## Gaps

There is no local HTTP API for the UI; central API calls are made directly from the webview. There is no visible database migration framework, metrics endpoint, request correlation system, or integration test harness. Central authorization and device-pairing implementation must be documented by the external backend owner.
