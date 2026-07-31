// use aes_gcm::{
//     aead::{Aead, KeyInit},
//     Aes256Gcm, Nonce,
// };
// use rand::RngCore;

// pub struct EncryptedChunk {
//     pub nonce: Vec<u8>,
//     pub data: Vec<u8>,
// }

// pub fn encrypt_chunk(key: &[u8; 32], plaintext: &[u8]) -> Result<EncryptedChunk, String> {
//     let cipher = Aes256Gcm::new(key.into());

//     let mut nonce_bytes = [0u8; 12];

//     rand::thread_rng().fill_bytes(&mut nonce_bytes);

//     let nonce = Nonce::from_slice(&nonce_bytes);

//     let ciphertext = cipher
//         .encrypt(nonce, plaintext)
//         .map_err(|e| e.to_string())?;

//     Ok(EncryptedChunk {
//         nonce: nonce_bytes.to_vec(),
//         data: ciphertext,
//     })
// }

// pub fn decrypt_chunk(
//     key: &[u8; 32],
//     nonce_bytes: &[u8],
//     ciphertext: &[u8],
// ) -> Result<Vec<u8>, String> {
//     let cipher = Aes256Gcm::new(key.into());

//     let nonce = Nonce::from_slice(nonce_bytes);

//     cipher.decrypt(nonce, ciphertext).map_err(|e| e.to_string())
// }

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

// pub fn encrypt_chunk(key: &[u8; 32], plaintext: &[u8]) -> Result<EncryptedChunk, String> {
//     let cipher = Aes256Gcm::new(key.into());

//     let mut nonce = [0u8; 12];

//     rand::thread_rng().fill_bytes(&mut nonce);

//     let ciphertext = cipher
//         .encrypt(Nonce::from_slice(&nonce), plaintext)
//         .map_err(|e| e.to_string())?;

//     Ok(EncryptedChunk {
//         nonce: nonce.to_vec(),
//         data: ciphertext,
//     })
// }

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
