# Security Review

## Blockers carried by the transport work (recorded 2026-10-16)

Both of these were found while mapping the transfer path for the transport port
(ADR-0004). **Neither is fixed, and neither was touched by that change** — they
are recorded here so that they are tracked as their own work rather than folded
into a refactor that is meant to change no behaviour.

### Blocker A — the receiver's authorization is presence-only, and the product sends it an empty token

Two defects that combine into one:

1. **The receiver never reads the `Authorization` value.** Every endpoint checks
   only that the header is *present* — `crates/desktop/src/transfer/writer.rs:302`
   (`/transfer/start`), `:768` (`/transfer/chunk`), `:1257`
   (`/transfer/public-key`), `:1442` (`/transfer/status`), `:1489`
   (`/transfer/cancel`). `writer.rs:313` even logs `"authorization header
   present"`, which is the whole of the check.

2. **The sender sends `Bearer ` with nothing after it.** The control plane
   supplies an `auth_token`, but `crates/desktop/src/events/dispatcher.rs:109`
   sets `auth_token: String::new()`, so `TransferMetadata.auth_token` is empty
   and the v1 wire carries an empty bearer on every request. This is why the
   presence check has never failed in practice: the product satisfies it with an
   empty string.

Together: any process that can reach the receiver's port can open a session,
write chunks, and read or cancel transfer state. The receiver binds
`0.0.0.0:7878` (`crates/desktop/src/lib.rs`), so that is every interface.

It is *not* a confidentiality break on its own — chunks are AES-256-GCM sealed
under a transfer key an attacker does not have — but it is endpoint abuse,
resource exhaustion, and transfer interference, and the transport work makes it
**strictly worse**: a direct P2P transport makes the receiver reachable from
anywhere, not just from a network the peer is already on.

The fix is ADR-0009's receiver-side session token: minted per transfer, bound to
`transfer_id` + `sender_device` + expiry, validated **before** any filesystem
work. The port already carries the type for it
(`vilsend_core::SessionToken`), including an `is_empty` so a receiver can refuse
an empty token rather than accept it — but nothing validates one yet.

*This entry sharpens the older one below (`High: local receiver authorization is
presence-only`), which named the first half and not the second.*

### Blocker B — one process-wide mutex is held across disk I/O for every chunk

> **Status, 2026-10-16: resolved for the chunk path.** The process-wide
> `ReceiverState.guard` is gone. Chunk writes now take a **per-transfer** lock
> (`ReceiverState::transfer_lock`, `crates/desktop/src/transfer/writer.rs`) for
> that transfer's own manifest read-modify-write only, and the durable write
> moved into `crates/desktop/src/transfer/chunk_sink.rs`, which never holds a
> lock across a network await — the caller sends its acknowledgement after the
> write returns.
>
> The unreachable pause loop described below was moved *outside* the lock and
> made cancellation-aware, so implementing receiver-side pause can no longer turn
> it into a device-wide stall.
> `resumability_tests::one_transfers_write_lock_does_not_block_another` asserts
> both halves of the new contract: different transfers never wait on each other,
> and one transfer's own chunks still serialise.
>
> The description below is kept as what the fix is measured against.

`crates/desktop/src/transfer/writer.rs:147` holds a single
`tokio::sync::Mutex<()>`, created once per receiver at `:179`. It is acquired at
`:901` and **held to the end of the chunk handler**, across:

- `fs::create_dir_all(&parts).await` (`:907`) — disk I/O;
- `state.manifests.lock().await` (`:939`) and `manifest::load_or_default(...)`
  (`:942`) — a manifest read that walks the transfer directory;
- the whole chunk write and manifest record.

So every chunk of every transfer on the device is serialized through one lock,
and the lock is held across `await`s that touch the disk. Today that is a
throughput ceiling that the per-transfer design hides. It becomes a correctness
hazard the moment a session is long-lived rather than per-request, which is what
a streaming transport introduces: a session that stalls while holding the guard
blocks *every other transfer on the device*, including ones on other transports.

There is also a latent hang: `:922` spins `while download.paused.load(..)` with a
100 ms sleep, inside the guard, and only checks `cancelled` *after* the loop. That
loop is **not reachable today** — nothing in `writer.rs` ever sets
`DownloadState::paused`, so it cannot be entered (verified: the only writes are
`manager.rs:713/771` on the *sender's* `UploadState` and `writer.rs:1512` for
`cancelled`). It is recorded because implementing receiver-side pause would turn
it into a permanent, device-wide stall.

The fix is a per-transfer lock held only across the disk mutation and **never
across a network await**, plus either deleting the unreachable pause loop or
making it cancellable.

## High priority findings

### High: local receiver authorization is presence-only
`src-tauri/src/transfer/writer.rs` rejects a request only when `Authorization` is absent. It does not validate the Bearer value against the transfer, central service, tunnel identity, or a receiver-side session. Because the receiver binds to `0.0.0.0:7878`, any reachable caller with a syntactically present header can attempt the handshake and chunk endpoints. Encryption still protects chunks from decryption without the transfer key, but endpoint abuse, resource exhaustion, and transfer-session interference remain possible.

**Fix:** use short-lived transfer-scoped credentials, validate them before state creation and every chunk, bind to an authenticated tunnel or localhost where possible, and add rate/size/expiry limits.

### High: fixed Stronghold password
`src-tauri/src/lib.rs` initializes the Stronghold plugin with the literal `your-stronghold-password`. A static source-visible password is not a meaningful secret.

**Fix:** remove unused Stronghold initialization or derive a platform-protected secret without logging or hardcoding it.

### High: unbounded/weak receiver resource controls
The receiver accepts a declared file size and creates state/temporary files. It checks available disk space but lacks visible per-peer quotas, request rate limits, transfer expiry, maximum header/path lengths, and cleanup guarantees.

**Fix:** enforce authenticated quotas, deadlines, bounded active transfers, body limits per route, and scheduled cleanup.

## Medium findings

- The application CSP is null and devtools are enabled in the main Tauri window. Tighten CSP and disable devtools in production.
- The receiver binds all interfaces rather than a least-exposure address.
- Raw token existence/length and token-bearing operational context are logged by frontend/Rust diagnostic prints. Remove token diagnostics and use redacted IDs.
- `set_default_download_location` accepts a user path and creates it, but the receiver later joins relative paths. Add canonicalization, symlink-aware checks, and destination containment tests.
- X25519 transfer derivation lacks visible authentication/signatures and associated data. Confidentiality/integrity of chunks should not be confused with peer authentication.
- `clear_all` removes tunnel credentials but not device identity keys, despite its name.

## Positive controls

Secrets use the secure-storage plugin; transfer chunks use AES-GCM; relative paths reject absolute paths and parent components; temporary merge files are renamed atomically; central requests use Bearer tokens; production defaults use HTTPS/WSS.

## Not found

No hardcoded Clerk secret or private tunnel token was found in the inspected source. Server-side authorization, CORS, central refresh-token policy, certificate configuration, and revocation behavior are not found in repository.
