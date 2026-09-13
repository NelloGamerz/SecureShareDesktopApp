# Security Review

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
