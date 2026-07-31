// use base64::{engine::general_purpose::STANDARD, Engine};
// use ed25519_dalek::{SigningKey, VerifyingKey};
// use rand_core::OsRng;

// pub struct DeviceKeyPair {
//     pub private_key: String,
//     pub public_key: String,
// }

// pub fn generate_device_keypair() -> DeviceKeyPair {
//     let signing_key = SigningKey::generate(&mut OsRng);

//     let verifying_key: VerifyingKey = signing_key.verifying_key();

//     let private_key = STANDARD.encode(signing_key.to_bytes());

//     let public_key = STANDARD.encode(verifying_key.to_bytes());

//     DeviceKeyPair {
//         private_key,
//         public_key,
//     }
// }

use base64::{engine::general_purpose::STANDARD, Engine};
use rand_core::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};

pub struct DeviceKeyPair {
    pub private_key: String,
    pub public_key: String,
}

pub fn generate_device_keypair() -> DeviceKeyPair {
    // Generate private key
    let private_key = StaticSecret::random_from_rng(OsRng);

    // Derive public key from private key
    let public_key = PublicKey::from(&private_key);

    DeviceKeyPair {
        private_key: STANDARD.encode(private_key.to_bytes()),
        public_key: STANDARD.encode(public_key.as_bytes()),
    }
}

pub fn derive_public_key(private_key_b64: &str) -> Result<String, String> {
    let bytes = STANDARD
        .decode(private_key_b64)
        .map_err(|e| format!("invalid private key: {e}"))?;

    if bytes.len() != 32 {
        return Err("private key must be 32 bytes".into());
    }

    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);

    let private_key = StaticSecret::from(key);
    let public_key = PublicKey::from(&private_key);

    Ok(STANDARD.encode(public_key.as_bytes()))
}
