# Architectural Decisions

## Tauri for the desktop shell
React provides the UI while Rust owns privileged process, key, filesystem, and large-file operations. This is appropriate for desktop constraints, but the IPC façade should remain narrow.

## Clerk for user identity
The frontend delegates user sessions to Clerk and mirrors the access token into Rust for WebSocket and transfer-related control. This avoids implementing password auth locally, but creates two token consumers and requires consistent refresh/revocation handling.

## Cloudflared for remote reachability
Remote transfer uses a bundled Cloudflared tunnel process. This simplifies NAT traversal but introduces sidecar lifecycle, credential, update, and observability concerns.

## X25519 plus AES-GCM for transfer payloads
The implementation uses ephemeral sender X25519, receiver device X25519, HKDF-SHA256, and per-chunk AES-GCM nonces. This is a reasonable payload-protection primitive set, but peer/session authentication and replay binding remain incomplete in visible code.

## SQLite for local transfer metadata
SQLite stores file paths and sizes in app data. This supports restart-independent metadata lookup but does not yet persist active transfer state or chunk completion.

## Recommended decisions to record next

Define the receiver authentication protocol, transfer expiry/replay model, shutdown contract, filesystem containment policy, supported resource packaging matrix, and test ownership between frontend, Rust, and external backend teams.
