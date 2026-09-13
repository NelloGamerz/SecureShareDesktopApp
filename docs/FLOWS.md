# Important Flows

## Startup and service synchronization

```mermaid
sequenceDiagram
    participant T as Tauri
    participant R as Rust setup
    participant U as React
    participant C as Clerk/API
    T->>R: setup app, SQLite, receiver :7878
    T->>U: load Vite/dist frontend
    U->>C: load Clerk/profile
    U->>R: status checks
    U->>C: register device if needed
    U->>R: start Cloudflared
    U->>R: start WebSocket
    R-->>U: events/status
```

## Login and token refresh

```mermaid
sequenceDiagram
    participant U as Clerk/React
    participant R as Rust AuthService
    participant W as Central WebSocket
    U->>U: obtain Clerk JWT
    U->>R: login(token, device info)
    R->>R: store token in memory
    U->>R: update_auth_token every 30s
    U->>R: start WebSocket
    R->>W: connect Bearer JWT + device ID
```

## Device registration and pairing

The visible frontend calls central `/devices/register` and `/devices/pair*` endpoints. Device registration includes an X25519 public key. QR rendering/scanning UI exists in `src/features/devices/qr-pairing-page.tsx`, but the cryptographic pairing protocol and central validation are not found in this repository.

## Transfer

The sender fetches metadata from `/transfers`, then Rust performs public-key retrieval, handshake, encrypted chunk upload, retries, progress events, and completion. The receiver derives a key, decrypts and writes parts, then merges them into the configured download path.

## Logout and shutdown

React logout stops WebSocket and Cloudflared and clears Rust auth. Window destruction kills Cloudflared. Explicit active-transfer cancellation and receiver cleanup during app shutdown are not fully coordinated in the visible implementation.
