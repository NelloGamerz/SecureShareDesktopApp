# The v2 framed session — format, durability, locking, and compatibility

Status: **stage 1b, partially implemented.** Written before the receiver work
began, because three of these decisions are not reversible once code depends on
them.

## Implementation status, precisely

| Piece | State |
|---|---|
| Frame codec (`transfer/frame.rs`) | **done**, 19 tests, bounded, peer-panic-proof |
| Durable chunk-write core (`transfer/chunk_sink.rs`) | **done**; v1 routes through it |
| Per-transfer locking (replaces the process-wide guard) | **done**; asserted by test |
| Layer 2 above the transport (`ProtocolSession` reshape) | **done**; v1 byte-identical |
| `FramedSession` — the sender half of the protocol | **not written** |
| `serve_v2_session` — the receiver half | **not written** |
| `GET /v2/session` | **not routed**, deliberately — see §8 |
| WebSocket client transport in `vilsend-transport` | **not written** |
| End-to-end framed tests | **not written** |

Two consequences follow from that table and are worth stating plainly rather
than inferring:

- **The v2 wire is unreachable from outside the process.** Nothing serves it and
  nothing dials it. That is the strongest available answer to "inaccessible by
  default", and it is also why the four authorization tests the brief asks for
  cannot yet be written (§8.5).
- **`frame.rs` has no production consumer.** The codec is exercised by its own
  tests and exported through `lib.rs::wire`; the sender and receiver that would
  use it are the remaining work. This is the same state `crypto_v2` shipped in —
  its module docs say so itself ("nothing in this module is called by the
  transfer path yet").

See also: [`migration/02-transport-layer.md`](./migration/02-transport-layer.md)
(the proposal this implements) and [`SECURITY.md`](./SECURITY.md) (the two open
blockers this must not worsen).

---

## 1. Frame format

```text
frame := u32_le(body_len + 1) ‖ u8 kind ‖ body
```

Little-endian, matching the integer convention `crypto_v2::ChunkAad::encode`
already uses, so v2 has one convention rather than two. The `+ 1` keeps the kind
inside the declared length, so a frame can be validated and skipped without
understanding its body.

Implemented in `crates/desktop/src/transfer/frame.rs`. The cap is **33 MiB**
(`MAX_CHUNK_SIZE` plus 1 MiB of header room), applied to the declared length
*before anything is buffered*: a peer that opens with `u32_le(0xFFFF_FFFF)` is
refused having sent four bytes.

| kind | byte | body |
|---|---|---|
| `HELLO` | `0x01` | JSON |
| `HELLO_ACK` | `0x02` | JSON |
| `MANIFEST` | `0x03` | JSON (`integrity::ExpectedTransfer`) |
| `MANIFEST_ACK` | `0x04` | JSON |
| `CHUNK` | `0x05` | binary |
| `ACK` | `0x06` | binary |
| `FINISH` | `0x07` | 32 bytes |
| `COMPLETE` | `0x08` | empty |
| `CANCEL` | `0x09` | empty |
| `ERROR` | `0x0A` | JSON `{code, message}` |
| `PING` | `0x0B` | empty |
| `PONG` | `0x0C` | empty |

`CHUNK` body: `u32_le(file_id_len) ‖ file_id ‖ u64_le(chunk_index) ‖
u64_le(total_chunks) ‖ nonce[12] ‖ ciphertext(‖ tag[16])`.

The relative path is **not** in a chunk. It travels once in the `MANIFEST`, and
the receiver resolves it by `file_id`, so a chunk cannot claim a destination the
receiver never accepted — and a 4 MiB chunk does not carry a path it already has.

`ACK` body: `u32_le(file_id_len) ‖ file_id ‖ u64_le(chunk_index)`.

### Version validation

`HELLO` carries `proto_v: u16`. A receiver that does not speak it answers
`ERROR {code: "unsupported_protocol"}` and closes. There is **no** `HELLO_REJECT`
frame: a v1 peer is detected by the endpoint (`GET /v2/session` → `404`), so a
version mismatch *inside* v2 can only mean a future v3 peer, which is a clean
failure rather than a negotiation.

`HELLO_ACK` carries `proto_v` too, and the sender validates it. A receiver
answering with a version the sender did not offer is a protocol error, not a
downgrade to accept.

### What is not authenticated (and this matters)

Stage 1b ships **without the device signature** ADR-0007 specifies, because no
Ed25519 device key exists yet (`ed25519-dalek` was removed from
`crates/desktop/Cargo.toml`).

The consequence is specific and worth stating plainly: `derive_transfer_key_v2`
salts the KDF with the two handshake nonces *because* those nonces are supposed to
live inside a transcript signed by the receiver's device key. Without the
signature, an active attacker who can modify the handshake can choose the nonces.
The salt still does its other job — two sessions reaching the same ECDH secret no
longer derive the same key — but the §7.5 downgrade defence is **not** delivered
by this stage. v2 is no weaker than v1 here, and it is not yet stronger.

---

## 2. ACK semantics and crash durability

**An `ACK` for `(file_id, chunk_index)` asserts:** the receiver has written that
chunk's plaintext to `<index>.part` **and** recorded it in `manifest.json`, both
through `atomic_file::write_atomic` — write to a temp file, `flush()`,
**`sync_all()`**, then `rename`.

So an ACK is not "the bytes reached the socket", and not "the receiver's process
holds it in memory". It is the receiver's own durable record accepting the chunk.

Precisely what that survives:

| Event | Survives? | Why |
|---|---|---|
| Sender crashes | **yes** | nothing about the receiver's state depends on the sender |
| Receiver process crashes | **yes** | `.part` and `manifest.json` are on disk, fsynced before the ACK |
| Receiver power loss / kernel panic | **no, not guaranteed** | the `rename` is not durable: nothing fsyncs the parent directory, so the directory entry can be lost |

The last row is the honest limit, and it is the **same guarantee v1 has** — v1's
`POST /transfer/chunk` returns `200` after the identical sequence. Closing it
would mean an `fsync` of the containing directory per chunk, which on Windows is
a genuine throughput cost, and it buys protection against a failure the sender's
resume path already tolerates.

**Therefore the sender's resume logic must treat an ACK as "almost certainly
present", never as "certainly present".** It already does: `resume::confirmed_chunks`
re-verifies every claim by re-hashing the bytes on disk and rejects anything it
cannot reproduce, so a chunk lost to power failure is simply re-sent. The ACK
buys the *stall detector* a real signal; it does not buy the resume path an
assumption it may not already make.

---

## 3. Chunk state transitions (receiver)

```text
                      CHUNK received
                           │
              decrypt + AAD + nonce-fresh?  ── no ──►  (no state change) ─► NACK
                           │ yes
                           ▼
                    ┌─────────────┐
                    │   Written   │  <index>.part fsynced on disk
                    └─────────────┘
                           │ manifest record + store (fsynced)
                           ▼
                    ┌─────────────┐
                    │  Recorded   │ ──────────────────────────────►  ACK sent
                    └─────────────┘
                           │ all indices recorded (cheap check)
                           ▼
                    ┌─────────────┐
                    │  Verified   │  every chunk re-hashed against its entry
                    └─────────────┘
                       │        │
              hash ok  │        │ hash mismatch
                       ▼        ▼
                ┌───────────┐  ┌────────────┐
                │  Merged   │  │ Discarded  │  part deleted, entry forgotten,
                └───────────┘  └────────────┘  manifest re-stored, transfer REFUSED
                                                → sender must resend those chunks
```

Two invariants that the transitions must preserve, both already true in v1 and
both relied on by the resume tests:

1. **`Written` without `Recorded` is not acceptance.** `manifest::chunk_complete`
   requires an entry *and* bytes that match it. A `.part` with no entry — from a
   crash between the two writes — is evidence of nothing and is written again.
2. **`Recorded` without matching bytes is corruption, not progress.** The
   completion pass prunes it rather than counting it, and refuses the transfer so
   the sender hears about it.

---

## 4. Lock ordering

### Today

`ReceiverState.guard` (`writer.rs:147`) is a **single process-wide**
`tokio::sync::Mutex<()>`, acquired once at `writer.rs:901` and held to the end of
the handler — across `create_dir_all`, the `manifests` map lock, `write_part`,
`manifest::store`, the completion pass, `assemble_and_publish` (which rewrites
the whole file) and cleanup.

Two consequences:

- Every chunk of *every* transfer on the device serialises through one lock, and
  the lock is held across disk I/O.
- A session that stalls while holding it blocks every other transfer on the
  device. That is survivable for v1, where each request is short and
  independent. It is **not** survivable for a long-lived stream, which is what a
  framed session is.

### Intended

**`state.guard` is removed from the chunk path.** In its place:

- **A per-transfer lock.** `ReceiverState` gains
  `writers: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>`
  — an outer map lock held only long enough to clone the inner `Arc`, then the
  inner lock held for the transfer's own read-modify-write of its manifest.

**Acquisition order** (never inverted; never two of them across an `await`):

```text
  1. writers map lock        (brief: clone the per-transfer Arc, then drop)
  2. per-transfer lock       (the transfer's own manifest read-modify-write)
  3. manifests map lock      (brief: get or replace one entry)
  4. disk I/O                (write_part, store) — no other lock held
```

- **Rule: no network `await` while any of these is held.** The framed session
  sends its ACK *after* dropping the per-transfer lock. A stall therefore blocks
  other chunks of the *same* transfer only, and never the port.
- `state.keys` and `state.downloads` are `RwLock`s read for microseconds; they
  are not part of the ordering and are never held across I/O.

### Why per-transfer and not per-file

A transfer's manifest is one JSON document holding every file's entries, so two
files of the same transfer still contend on one document. Per-transfer is the
finest granularity that does not require restructuring the manifest format —
which would change the v1 on-disk format and break restart compatibility.

---

## 5. Compatibility boundaries

**Frozen and untouched:**

- The v1 wire: `GET /transfer/public-key`, `POST /transfer/start`,
  `POST /transfer/chunk`, `GET /transfer/status`, `POST /transfer/cancel`, with
  their exact headers and status codes. `tests/compat_v1.rs` pins this and stays
  green in both feature configurations.
- The v1 crypto (`crypto.rs`) and its key derivation.
- The on-disk formats: `<index>.part` and `manifest.json`.

**New, behind `protocol-v2` (off by default):**

- `GET /v2/session` — no `Upgrade` header → `200` + a capabilities document;
  `Upgrade: websocket` → `101`. A v1 receiver answers `404`, which is how a peer
  is detected as v1 (ADR-0010 decision 3: by capability, never by a version
  string).
- The framed protocol above, over that WebSocket.

**Detection order on the sender:** probe `GET /v2/session`. `404` → v1 peer →
`LegacyV1Session`, the legacy HTTP path, no framing, no resume over the stream.
Anything else → v2.

---

## 6. Unavoidable v1 behaviour changes

These are consequences of the decisions above, not accidents. They are listed
before the code is written.

### 6.1 Chunk writes for *different* transfers no longer serialise

**Change:** two chunks belonging to different transfers can now be written
concurrently; today the process-wide guard makes that impossible.
**Why it cannot be avoided:** removing the process-wide guard is the entire point
of §4; keeping it while adding a per-transfer lock would give a long-lived
session the worst of both.
**Risk:** low. The two paths write to different `incoming/<transfer_id>/`
directories, and `atomic_file`'s temp names are derived from the target path, so
there is no shared mutable state between them.
**What changes observably:** throughput when several transfers run at once; and a
transfer's chunk writes no longer wait behind another transfer's file merge.

### 6.2 A file merge no longer blocks other transfers

Same cause as 6.1. `assemble_and_publish` rewrites a whole file and today does so
under the global guard.

### 6.3 `resumability_tests.rs` must be updated — the only test file affected

`crates/desktop/src/transfer/resumability_tests.rs:878` and `:1061` currently
acquire `state.guard` to freeze the write path mid-transfer so a test can
interrupt it deterministically. With the guard gone from the chunk path, those
tests would no longer freeze anything.

**They will be changed to take the per-transfer lock instead.** The *invariant*
they assert is unchanged — a transfer interrupted between the part write and the
manifest store resumes correctly — and it is worth stating that the test is being
retargeted, not relaxed: if anything it becomes more precise, because it now
freezes exactly the transfer under test rather than the whole process.

This is the one change to existing test code, and it is the reason this section
exists.

### 6.4 The `refused` check tightens

`receive` reads `state.refused` at `:744`, *before* the guard, so a concurrent
refusal can currently be missed. Under the per-transfer lock the read moves
inside it, closing a real race in the direction of refusing more reliably.

### 6.5 No wire change

None. Every v1 request and response is byte-identical; the extraction is internal
to the receiver. `compat_v1.rs` is the assertion of this and must stay green
unmodified apart from what 6.3 describes (which is in a different file).

### 6.6 The relative-path check tightened on Windows — a real fix, made deliberately

`is_contained` (`chunk_sink.rs`) now rejects any path with a `RootDir`, `Prefix`
or `ParentDir` component, rather than testing `Path::is_absolute()` and scanning
for `..`.

This is **not** cosmetic. On Windows `Path::new("/etc/passwd").is_absolute()` is
**false** — a Windows absolute path needs a drive letter or a UNC prefix — so the
old check admitted rooted POSIX-style paths. A sender could therefore ask the
receiver to write outside the transfer directory with a leading `/`. It is a
narrow hole (the receiver still chooses the root it joins onto) but it is a hole,
and it was found by a test written for this refactor rather than by review.

**Effect on v1:** a v1 sender that sent a rooted path and had it accepted will now
be refused with `400`, which is what the check always intended — `docs/FILE_TRANSFER.md`
describes it as rejecting "absolute paths and `..`". No legitimate sender sends
one, and no header, status code or body shape changes. Recorded here because it is
a behaviour change on the v1 path and the brief required those to be declared.

---

## 7. What is out of scope for 1b

- **Peer authentication** (the device signature) — see §1. Stage 3.
- **iroh, route racing, stall-based failover.** Stage 1c and 2.

---

## 8. The authorization contract — and why `/v2/session` is not routed yet

**Status: the endpoint is not registered in any build.** `router()` in
`crates/desktop/src/transfer/writer.rs` does not serve `/v2/session`, and it will
not until the contract below is satisfied end to end.

That is a deliberate choice, not an omission. `SECURITY.md` blocker A is that the
receiver's `Authorization` check tests *presence* and never the value
(`writer.rs:302, 768, 1257, 1442, 1489`), and that the sender sends `Bearer `
with nothing after it (`dispatcher.rs:109`). Adding a new endpoint that copied
that check would widen the hole to a second route and a second protocol. Adding
one that validated properly would reject the product's own transfers, because the
sender has no real token to send.

The fix is coupled across three sides, and none of them can move alone.

### 8.1 What the backend must mint

A per-transfer session token, delivered to the **sender** with `START_TRANSFER`
and to the **receiver** out of band, with these properties:

| Property | Why |
|---|---|
| Bound to `transfer_id` | A leaked token must not open a different transfer |
| Bound to the sender's `device_id` | A leaked token must not be usable by another device |
| Short TTL (≤ 15 min, recommended) | Bounds the window a leaked token is useful in |
| Single-use per session | A token that survives its session is a token that can be replayed |
| **Minted by the backend, not the sender** | This is the whole point: a value the sender can produce is a value an attacker can produce |

Concretely, ADR-0009's shape:

```rust
pub struct SessionToken {
    pub transfer_id: TransferId,
    pub sender_device: DeviceId,
    pub expires_at: UnixTime,
    /// HMAC over the above, keyed by a value the RECEIVER obtains from the
    /// control plane — not a value the sender can mint.
    pub mac: [u8; 32],
}
```

`vilsend_core::SessionToken` already carries the port's shape for this, including
`is_empty()`. It is an opaque `String` today; moving it to the struct above must
not change the port's signature, which is why it is a newtype rather than a
`String` in the first place.

### 8.2 What the receiver must validate, and in what order

Before any session state is created, and before any filesystem work:

1. **Reject an empty bearer.** Unconditionally, whatever else is true. This is
   the rule the v1 endpoints break.
2. **Reject a bearer that is not a well-formed token** — bad MAC, wrong length,
   unparseable.
3. **Reject an expired token.**
4. **Reject a token whose `transfer_id` or `sender_device` does not match the
   session being opened.** A valid token for transfer A must not open transfer B.
5. **Reject a token already spent.** Single-use means the receiver, not the
   sender, decides what "already spent" means.

Only then may the handshake proceed. A failure answers `ERROR {code:
"unauthorized"}` and closes — never a partially established session, and never a
fallback to the old presence check.

### 8.3 What the sender must do

Send the token it was given, and **fail loudly if it was given none** rather than
sending an empty string. `dispatcher.rs:109` currently sets
`auth_token: String::new()`; that line is the sender half of blocker A, and until
it is changed the sender cannot satisfy this contract.

### 8.4 Why this is not being faked

There is no backend endpoint to mint the token, no field on `START_TRANSFER` to
carry it, and no device-key binding to check it against. Building `/v2/session`
against an authorization seam that always says "yes" would produce a route that
looks tested and is unprotected — and the tests would encode the wrong contract,
which is worse than having none.

So the seam is specified here rather than stubbed, and the endpoint stays off.

### 8.5 What unblocks it

1. Backend mints and delivers the token (§8.1). **Requires backend change.**
2. `START_TRANSFER` gains `session_token: Option<String>` — additive and
   optional, so a v1 client ignores it.
3. The sender sends it and refuses to open a v2 session without one.
4. The receiver implements §8.2 behind a `SessionAuthorizer` seam, so tests can
   supply a known token and assert each rejection above.
5. **Then** `/v2/session` is routed, and the four authorization tests the brief
   asks for (empty, wrong, expired, mismatched) become writable.

Until step 5, the v2 wire is unreachable from outside the process by
construction, which is the strongest available answer to "keep it inaccessible by
default".
