# API Contracts

These are client-observed contracts, not a server specification. The central server implementation is not found in repository.

## Central HTTP API

| Method | Path | Purpose | Auth |
|---|---|---|---|
| GET | `/devices` | List devices | Clerk Bearer + `X-Device-Id` |
| GET | `/devices/health` | Device health summary | Same |
| GET | `/devices/{id}` | Device detail | Same |
| POST | `/devices/register` | Register current or supplied device | Same |
| PUT | `/devices/{id}/rename` | Rename device | Same |
| DELETE | `/devices/{id}` | Remove device | Same |
| POST | `/devices/pair` | Create pairing session | Same |
| GET | `/devices/pair/{code}` | Read pairing status | Same |
| POST | `/devices/pair/{code}/connect` | Connect pairing session | Same |
| POST | `/devices/pair/{code}/cancel` | Cancel pairing | Same |
| POST | `/transfers` | Create transfer and receive endpoint/token metadata | Same |
| GET | `/transfers` | List current transfers | Same |
| PATCH | `/transfers/{id}` | Accept, reject, or cancel transfer | Same |

Axios normalizes 401 into a session-expired error. Other errors expose the server `error`, `message`, or Axios message.

## WebSocket

Production default: `wss://api.vilsend.in/ws`; development default: `ws://localhost:8080/ws`. The Rust client sends `Authorization: Bearer <Clerk token>` and `x-device-id`. The message protocol is defined in `src-tauri/src/websocket/protocol.rs` and the receiver dispatches Tauri events.

## Local transfer receiver

The Rust receiver binds `0.0.0.0:7878` and exposes:

- `GET /transfer/public-key`: returns the receiver X25519 public key.
- `POST /transfer/start`: accepts transfer ID, sender ephemeral public key, and total file size.
- `POST /transfer/chunk`: accepts encrypted bytes and headers `Authorization`, `Transfer-Id`, `File-Id`, `Relative-Path`, `Chunk-Index`, `Total-Chunks`, `Chunk-Nonce`, and `Encryption`.

The current code checks that an Authorization header exists on start/chunk but does not visibly validate its token value against a central authority or transfer session. Treat this as a high-priority contract defect until proven otherwise by external infrastructure.
