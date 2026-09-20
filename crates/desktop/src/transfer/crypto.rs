//! The **frozen v1** transfer crypto.
//!
//! Every function here is on the wire between an installed v1.0.4 client and
//! anything newer, so none of it may change — including the parts that are
//! wrong. Two of them are wrong on purpose-of-history:
//!
//! * [`derive_transfer_key`] calls HKDF with **no salt**, so two sessions that
//!   reach the same ECDH secret share a key (task 4.2).
//! * [`encrypt_chunk`] passes **no associated data**, so a captured ciphertext
//!   is bound to no transfer, file or index and can be replayed into another
//!   slot (task 4.1). This is the replay-binding gap recorded in
//!   `docs/DECISIONS.md:13`.
//!
//! Neither is fixed here. v2 is a separate implementation in
//! `crate::transfer::crypto_v2`, compiled only under the `protocol-v2`
//! feature, and the two never share a key derivation — see the module header
//! there for the domain separation that makes that true.
//!
//! The `tests` module below is the tripwire: it pins the v1 bytes by value,
//! and it names the two defects above as behaviour rather than fixing them.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};

use base64::{engine::general_purpose::STANDARD, Engine};

use hkdf::Hkdf;

use rand::RngCore;

use sha2::Sha256;

use x25519_dalek::{PublicKey, StaticSecret};

pub struct EncryptedChunk {
    pub nonce: Vec<u8>,
    pub data: Vec<u8>,
}

pub struct EphemeralKeyPair {
    pub private_key: StaticSecret,
    pub public_key: PublicKey,
}

//
// ----------------------------------------------------------------------
// Identity / Ephemeral Keys
// ----------------------------------------------------------------------
//

pub fn generate_ephemeral_keypair() -> EphemeralKeyPair {
    let private = StaticSecret::random_from_rng(rand::thread_rng());

    let public = PublicKey::from(&private);

    EphemeralKeyPair {
        private_key: private,
        public_key: public,
    }
}

pub fn decode_public_key(encoded: &str) -> Result<PublicKey, String> {
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|e| format!("invalid base64 public key: {e}"))?;

    if bytes.len() != 32 {
        return Err("public key must be 32 bytes".into());
    }

    let mut key = [0u8; 32];

    key.copy_from_slice(&bytes);

    Ok(PublicKey::from(key))
}

pub fn encode_public_key(public: &PublicKey) -> String {
    STANDARD.encode(public.as_bytes())
}

//
// ----------------------------------------------------------------------
// ECDH
// ----------------------------------------------------------------------
//

pub fn derive_shared_secret(
    private_key: &StaticSecret,
    receiver_public_key: &PublicKey,
) -> [u8; 32] {
    private_key.diffie_hellman(receiver_public_key).to_bytes()
}

//
// ----------------------------------------------------------------------
// HKDF
// ----------------------------------------------------------------------
//

pub fn derive_transfer_key(shared_secret: &[u8; 32]) -> Result<[u8; 32], String> {
    let hk = Hkdf::<Sha256>::new(None, shared_secret);

    let mut key = [0u8; 32];

    hk.expand(b"carsdv-transfer-key-v1", &mut key)
        .map_err(|e| format!("hkdf failed: {e}"))?;

    Ok(key)
}

//
// ----------------------------------------------------------------------
// AES-256-GCM
// ----------------------------------------------------------------------
//

pub fn encrypt_chunk(key: &[u8; 32], plaintext: &[u8]) -> Result<EncryptedChunk, String> {
    let cipher = Aes256Gcm::new(key.into());

    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);

    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|e| e.to_string())?;

    tracing::info!(
        plaintext_size = plaintext.len(),
        ciphertext_size = ciphertext.len(),
        nonce_size = nonce.len(),
        total_payload = ciphertext.len() + nonce.len(),
        "Chunk encrypted"
    );

    Ok(EncryptedChunk {
        nonce: nonce.to_vec(),
        data: ciphertext,
    })
}

pub fn decrypt_chunk(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(key.into());

    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|e| e.to_string())
}

pub fn decode_private_key(encoded: &str) -> Result<StaticSecret, String> {
    let bytes = STANDARD.decode(encoded).map_err(|e| e.to_string())?;

    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "Invalid X25519 private key length".to_string())?;

    Ok(StaticSecret::from(bytes))
}

/// Characterisation tests for the **frozen v1 wire format**.
///
/// v1.0.4 is shipped and speaks exactly this. Every assertion below is a
/// statement about bytes that are already on the wire, and the tests exist so
/// that the Phase 4 protocol work cannot change them by accident — the module
/// header says how; these are the tripwire.
///
/// Two of them pin a *defect* rather than a guarantee, and say so by name:
/// `the_v1_aead_does_not_bind_a_chunk_to_its_index` is the replay-binding gap
/// that task 4.1 closes for v2, and `the_v1_kdf_is_hkdf_sha256_without_a_salt`
/// is the missing-salt gap that task 4.2 closes for v2. Neither may be
/// "fixed" here: v1 cannot change, and the v2 fixes land in
/// `crate::transfer::crypto_v2` behind the `protocol-v2` feature.
#[cfg(test)]
mod tests {
    use super::*;

    /// The input to every KDF vector below. Fixed so the expected key can be
    /// frozen as a hex constant.
    const SHARED_SECRET: [u8; 32] = [0x42; 32];

    /*
     * ----------------------------------------------------------------------
     * Key derivation — pinned by value, because this is the wire format
     * ----------------------------------------------------------------------
     */

    /// The exact key v1 derives, as a literal.
    ///
    /// The three things this pins are all decisions the v1 wire made and that
    /// v2 deliberately changes:
    ///
    /// * the hash is SHA-256,
    /// * the **salt is empty** (`Hkdf::new(None, ..)` means a salt of 32 zero
    ///   bytes — task 4.2 adds a salt in v2),
    /// * the `info` string is the literal `b"carsdv-transfer-key-v1"`, a name
    ///   from before the product was called VilSend. It is ugly and it must not
    ///   be "tidied": changing it changes every derived key.
    ///
    /// If this test fails, a v1 peer can no longer decrypt anything this build
    /// sends. That is the entire point of freezing it.
    #[test]
    fn the_v1_kdf_is_hkdf_sha256_without_a_salt_and_a_frozen_info_string() {
        let key = derive_transfer_key(&SHARED_SECRET).unwrap();

        assert_eq!(
            hex::encode(key),
            "5ff215160c2a1daaab57b73d1a5a23541649915eb3e87be0b03f5fad88f99f66"
        );
    }

    /// A second vector, so the assertion above is a function of its input and
    /// not a constant that happens to be returned for anything.
    #[test]
    fn the_v1_kdf_varies_with_its_input() {
        let key = derive_transfer_key(&[0x00; 32]).unwrap();

        assert_eq!(
            hex::encode(key),
            "0573a909002a7f6a93e78135c300cd7fad90170a7d5997b67a7cd70482372e30"
        );
    }

    #[test]
    fn the_v1_kdf_is_deterministic() {
        assert_eq!(
            derive_transfer_key(&SHARED_SECRET).unwrap(),
            derive_transfer_key(&SHARED_SECRET).unwrap()
        );
    }

    /*
     * ----------------------------------------------------------------------
     * ECDH
     * ----------------------------------------------------------------------
     */

    #[test]
    fn both_sides_derive_the_same_shared_secret() {
        let receiver = generate_ephemeral_keypair();
        let sender = generate_ephemeral_keypair();

        assert_eq!(
            derive_shared_secret(&sender.private_key, &receiver.public_key),
            derive_shared_secret(&receiver.private_key, &sender.public_key)
        );
    }

    #[test]
    fn an_unrelated_private_key_derives_a_different_shared_secret() {
        let receiver = generate_ephemeral_keypair();
        let sender = generate_ephemeral_keypair();
        let stranger = generate_ephemeral_keypair();

        assert_ne!(
            derive_shared_secret(&sender.private_key, &receiver.public_key),
            derive_shared_secret(&stranger.private_key, &receiver.public_key)
        );
    }

    /*
     * ----------------------------------------------------------------------
     * AEAD — shape and round trip
     * ----------------------------------------------------------------------
     */

    #[test]
    fn a_chunk_round_trips_through_encrypt_and_decrypt() {
        let key = derive_transfer_key(&SHARED_SECRET).unwrap();
        let plaintext = b"the quick brown fox jumps over the lazy dog".to_vec();

        let encrypted = encrypt_chunk(&key, &plaintext).unwrap();

        let decrypted = decrypt_chunk(&key, &encrypted.nonce, &encrypted.data).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn the_v1_nonce_is_twelve_bytes_and_the_tag_is_sixteen() {
        let key = derive_transfer_key(&SHARED_SECRET).unwrap();
        let plaintext = vec![0xAB; 4096];

        let encrypted = encrypt_chunk(&key, &plaintext).unwrap();

        assert_eq!(encrypted.nonce.len(), 12, "AES-GCM nonce width");
        assert_eq!(
            encrypted.data.len(),
            plaintext.len() + 16,
            "AES-GCM appends a 16-byte tag and does not pad"
        );
    }

    /// An empty chunk is legal on the wire today. Nothing rejects it, and the
    /// receiver writes a 0-byte `.part` for it. Pinned so that a later change
    /// to reject empty chunks is a deliberate act with a failing-first test.
    #[test]
    fn an_empty_chunk_round_trips() {
        let key = derive_transfer_key(&SHARED_SECRET).unwrap();

        let encrypted = encrypt_chunk(&key, &[]).unwrap();

        assert_eq!(encrypted.data.len(), 16, "tag only");
        assert_eq!(
            decrypt_chunk(&key, &encrypted.nonce, &encrypted.data).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn every_v1_chunk_uses_a_fresh_random_nonce() {
        let key = derive_transfer_key(&SHARED_SECRET).unwrap();
        let plaintext = b"identical plaintext".to_vec();

        let first = encrypt_chunk(&key, &plaintext).unwrap();
        let second = encrypt_chunk(&key, &plaintext).unwrap();

        assert_ne!(first.nonce, second.nonce);
        assert_ne!(first.data, second.data, "same input, different nonce");
    }

    /// **This test asserts a vulnerability. It is not a guarantee.**
    ///
    /// The nonce is generated inside `encrypt_chunk` and compared against
    /// nothing, ever. There is no per-transfer state anywhere that could
    /// notice a repeat, which is the birthday-bound risk task 4.4 addresses
    /// for v2 with a bounded sliding window.
    ///
    /// The demonstration: take a captured nonce, seal a *different* plaintext
    /// under it, and decrypt both. Nonce reuse is invisible.
    #[test]
    fn the_v1_path_cannot_detect_a_reused_nonce() {
        let key = derive_transfer_key(&SHARED_SECRET).unwrap();

        let captured = encrypt_chunk(&key, b"first chunk").unwrap();

        let cipher = Aes256Gcm::new((&key).into());

        let second = cipher
            .encrypt(
                Nonce::from_slice(&captured.nonce),
                b"second chunk".as_slice(),
            )
            .unwrap();

        assert_eq!(
            decrypt_chunk(&key, &captured.nonce, &captured.data).unwrap(),
            b"first chunk"
        );

        assert_eq!(
            decrypt_chunk(&key, &captured.nonce, &second).unwrap(),
            b"second chunk"
        );
    }

    /*
     * ----------------------------------------------------------------------
     * AEAD — fail-closed behaviour
     * ----------------------------------------------------------------------
     */

    #[test]
    fn a_flipped_ciphertext_byte_fails_to_decrypt() {
        let key = derive_transfer_key(&SHARED_SECRET).unwrap();

        let mut encrypted = encrypt_chunk(&key, b"payload").unwrap();

        encrypted.data[0] ^= 0x01;

        assert!(decrypt_chunk(&key, &encrypted.nonce, &encrypted.data).is_err());
    }

    #[test]
    fn a_flipped_tag_byte_fails_to_decrypt() {
        let key = derive_transfer_key(&SHARED_SECRET).unwrap();

        let mut encrypted = encrypt_chunk(&key, b"payload").unwrap();

        let last = encrypted.data.len() - 1;
        encrypted.data[last] ^= 0x80;

        assert!(decrypt_chunk(&key, &encrypted.nonce, &encrypted.data).is_err());
    }

    #[test]
    fn a_flipped_nonce_byte_fails_to_decrypt() {
        let key = derive_transfer_key(&SHARED_SECRET).unwrap();

        let mut encrypted = encrypt_chunk(&key, b"payload").unwrap();

        encrypted.nonce[0] ^= 0x01;

        assert!(decrypt_chunk(&key, &encrypted.nonce, &encrypted.data).is_err());
    }

    #[test]
    fn a_different_key_fails_to_decrypt() {
        let encrypted =
            encrypt_chunk(&derive_transfer_key(&SHARED_SECRET).unwrap(), b"payload").unwrap();

        let other = derive_transfer_key(&[0x11; 32]).unwrap();

        assert!(decrypt_chunk(&other, &encrypted.nonce, &encrypted.data).is_err());
    }

    /// A wrong-length nonce must not panic.
    ///
    /// `Nonce::from_slice` panics on a slice that is not 12 or 16 bytes, so the
    /// *only* thing keeping a short nonce from taking the process down is the
    /// caller's `nonce.len() != 12` check in `writer::receive`. This test pins
    /// which of the two lengths is the one that would panic, so that a future
    /// refactor of the receiver's validation is a deliberate act.
    ///
    /// It documents the hazard rather than endorsing it: `decrypt_chunk` is
    /// not total, and the length check it relies on lives in another module.
    #[test]
    #[should_panic]
    fn a_wrong_length_nonce_panics_in_decrypt_chunk() {
        let key = derive_transfer_key(&SHARED_SECRET).unwrap();
        let encrypted = encrypt_chunk(&key, b"payload").unwrap();

        let _ = decrypt_chunk(&key, &encrypted.nonce[..11], &encrypted.data);
    }

    /*
     * ----------------------------------------------------------------------
     * The replay-binding gap — pinned, not endorsed
     * ----------------------------------------------------------------------
     */

    /// **This test asserts a vulnerability. It is not a guarantee.**
    ///
    /// The v1 AEAD is called with no associated data at all, so a ciphertext is
    /// bound to nothing: not the transfer, not the file, not the chunk index.
    /// A chunk captured on the wire decrypts perfectly when presented as a
    /// different chunk of a different file in a different transfer.
    ///
    /// Task 4.1 closes this for v2 by binding
    /// `transfer_id ‖ file_id ‖ chunk_index ‖ relative_path ‖ proto_v`. The v2
    /// counterpart of this test — `a_chunk_replayed_at_a_different_index_is_rejected`
    /// in `crypto_v2` — is the one that asserts rejection.
    ///
    /// When this test starts failing, someone has changed v1. That is a
    /// wire-format break, not a fix.
    #[test]
    fn the_v1_aead_does_not_bind_a_chunk_to_its_index() {
        let key = derive_transfer_key(&SHARED_SECRET).unwrap();

        // Captured from transfer "t-1", file "f-1", chunk 3.
        let captured = encrypt_chunk(&key, b"chunk three of file one").unwrap();

        // Replayed against transfer "t-9", file "f-7", chunk 99. The decrypt
        // function is not told any of that, because there is nowhere to put it.
        let replayed = decrypt_chunk(&key, &captured.nonce, &captured.data).unwrap();

        assert_eq!(replayed, b"chunk three of file one");
    }

    /*
     * ----------------------------------------------------------------------
     * Key encoding
     * ----------------------------------------------------------------------
     */

    #[test]
    fn a_public_key_round_trips_through_base64() {
        let pair = generate_ephemeral_keypair();

        let encoded = encode_public_key(&pair.public_key);
        let decoded = decode_public_key(&encoded).unwrap();

        assert_eq!(decoded.as_bytes(), pair.public_key.as_bytes());
    }

    #[test]
    fn a_public_key_that_is_not_thirty_two_bytes_is_rejected() {
        let short = STANDARD.encode([0u8; 31]);
        let long = STANDARD.encode([0u8; 33]);

        assert!(decode_public_key(&short).is_err());
        assert!(decode_public_key(&long).is_err());
    }

    #[test]
    fn an_invalid_base64_public_key_is_rejected() {
        assert!(decode_public_key("not base64!!").is_err());
    }

    #[test]
    fn a_zero_public_key_is_accepted_and_encodes_to_a_fixed_string() {
        // Pinned because it is on the wire: the all-zero X25519 public key is
        // the low-order point, and v1 does not reject it. Task 9.x may.
        assert_eq!(
            encode_public_key(&PublicKey::from([0u8; 32])),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );

        assert!(decode_public_key("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").is_ok());
    }

    #[test]
    fn a_private_key_round_trips_through_base64() {
        let pair = generate_ephemeral_keypair();

        let encoded = STANDARD.encode(pair.private_key.to_bytes());
        let decoded = decode_private_key(&encoded).unwrap();

        assert_eq!(decoded.to_bytes(), pair.private_key.to_bytes());
    }

    #[test]
    fn a_private_key_that_is_not_thirty_two_bytes_is_rejected() {
        assert!(decode_private_key(&STANDARD.encode([0u8; 31])).is_err());
        assert!(decode_private_key(&STANDARD.encode([0u8; 33])).is_err());
        assert!(decode_private_key("not base64!!").is_err());
    }
}
