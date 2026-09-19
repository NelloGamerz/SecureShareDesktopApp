# ADR-0007 — Peer authentication via a device signing key

- **Status:** Proposed
- **Date:** 2026-09-19
- **Phase:** 9
- **Supersedes:** —

## Context

Today the sender obtains the receiver's X25519 public key from
`GET {endpoint}/transfer/public-key` (`transfer/http_client.rs:142-183`) with no
authentication of the responder beyond "the endpoint string came from the
control plane."

That is **probably** adequate over the tunnel — Cloudflare terminates TLS to a
hostname the control plane assigned. It is **definitely** inadequate over any
direct path, where mDNS spoofing or ARP spoofing makes the endpoint
attacker-controlled.

The LAN transport (Phase 10) therefore cannot ship until this is fixed.

**There is a second, compounding gap:** the AEAD has **no associated data**
(`transfer/crypto.rs:116-138`). A captured ciphertext is bound to nothing — not
to the transfer, the file, the chunk index, or the path — so it can be replayed
into a different slot.

## Options considered

**A. Rely on the control plane; do nothing.**
Rejected for the LAN path. Acceptable only for the tunnel path, and only because
the tunnel already exists.

**B. TOFU (trust on first use) — pin the peer's key on first connect.**
Rejected as the primary mechanism. There is already a control plane holding
**registered** device public keys (`docs/FLOWS.md:38`), so TOFU would be
strictly weaker than the information you already have. TOFU also has no answer
for key rotation or a re-installed device.

**C. A device signing key, verified against the control-plane-registered key.**
**Chosen.**

**D. A PAKE or a short authentication string (SAS) shown to both users.**
Considered, rejected for v1. A SAS is a genuinely good defence (it is what
Signal and Magic Wormhole use) but it requires a UI affordance and user
attention on every transfer. **Recorded as a possible v2 addition**, especially
if a future transport has no control-plane registration to lean on.

## Decision

1. **Add a device signing identity.** The receiver signs a transcript with a
   long-term key whose public half is registered with the control plane at
   device registration.

   ```
   device_sig = Sign_device_key(
       session ‖ sender_ephemeral_pub ‖ receiver_ephemeral_pub
       ‖ nonce_s ‖ nonce_r ‖ chosen_transport ‖ proto_v
   )
   ```

   The signature is carried in `HELLO_ACK.device_sig` and verified by the sender
   against the **control-plane-registered** public key.

2. **Blocking technical problem: X25519 cannot sign.** The live device key is
   X25519 (`services/generate_device_keypair.rs:25-45`); the Ed25519
   implementation sitting commented out above it (lines 1–23) is dead, though
   `ed25519-dalek` is still a declared dependency.

   Resolution options, in preference order:
   - **(i)** Revive Ed25519 as a **second** device key, registered alongside
     X25519, used exclusively for signing. X25519 stays the KEX key.
   - **(ii)** Replace the device key with Ed25519 and convert to X25519 for KEX.
   - **(iii)** Use a signature scheme over the X25519 key (e.g. a KEM-style
     proof).

   **Recommendation: (i).** It is additive, it does not disturb the existing
   KEX path, and it lets both keys be registered during a transition window.

3. **Bind `chosen_transport` into the signed transcript.** This is the
   downgrade defence: an attacker who forces a LAN→tunnel downgrade cannot do so
   invisibly, because the choice is inside the signed material and both sides
   observe a mismatch or a surfaced `TransportDegraded` event.

4. **Add AAD to chunk encryption** (Phase 4, prerequisite):

   ```
   AAD = transfer_id ‖ file_id ‖ chunk_index ‖ relative_path ‖ proto_v
   ```

5. **Salt the HKDF** with both handshake nonces. Today's HKDF
   (`transfer/crypto.rs:82-91`) has no salt.

**Enforcement scope:** signatures are required **only on v2 handshakes**. v1
clients do not sign and continue to work over the tunnel, protected by the
session token (ADR-0009).

## Consequences

**Positive**

- The LAN transport becomes safer than today's tunnel path, not merely equal to it.
- Downgrade attacks become detectable and surfaceable rather than silent.
- Replay across files/indices is closed by the AAD change.

**Negative**

- **Requires backend work:** the device registration payload must carry a signing
  public key, and the backend must distribute it for peer verification.
- A re-installed device gets a new identity and must re-register.
- An extra signature verification on a cold path — negligible cost.

**Neutral**

- v1 clients are unaffected. The signature is v2-only, and the compatibility
  floor (v2 sender → v1 receiver over the tunnel) does not require it.

## Related

- [`../02-transport-layer.md`](../02-transport-layer.md) §7
- ADR-0004 (transport abstraction), ADR-0010 (protocol versioning)
- Q-06 in [`../07-risks-and-open-questions.md`](../07-risks-and-open-questions.md)
