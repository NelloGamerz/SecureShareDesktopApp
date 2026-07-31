use crate::services::generate_device_keypair::derive_public_key;
use tauri::AppHandle;
use tauri_plugin_secure_storage::{OptionsRequest, SecureStorageExt};

const TOKEN_KEY: &str = "tunnel_token";
const HOSTNAME_KEY: &str = "tunnel_hostname";
const DEVICE_PRIVATE_KEY: &str = "device_private_key";
const DEVICE_PUBLIC_KEY: &str = "device_public_key";

pub struct KeyringService;

impl KeyringService {
    fn set(app: &AppHandle, key: &str, value: &str) -> Result<(), String> {
        let request = OptionsRequest {
            prefixed_key: Some(key.to_string()),
            data: Some(value.to_string()),
            sync: None,
            keychain_access: None,
        };

        app.secure_storage()
            .set_item(app.clone(), request)
            .map_err(|e| e.to_string())
            .map(|_| ())
    }

    fn get(app: &AppHandle, key: &str) -> Result<String, String> {
        let request = OptionsRequest {
            prefixed_key: Some(key.to_string()),
            data: None,
            sync: None,
            keychain_access: None,
        };

        let result = app
            .secure_storage()
            .get_item(app.clone(), request)
            .map_err(|e| e.to_string())?;

        result.data.ok_or_else(|| format!("{} not found", key))
    }

    fn remove(app: &AppHandle, key: &str) -> Result<(), String> {
        let request = OptionsRequest {
            prefixed_key: Some(key.to_string()),
            data: None,
            sync: None,
            keychain_access: None,
        };

        app.secure_storage()
            .remove_item(app.clone(), request)
            .map_err(|e| e.to_string())
            .map(|_| ())
    }

    // Tunnel Token

    pub fn save_tunnel_token(app: &AppHandle, token: &str) -> Result<(), String> {
        Self::set(app, TOKEN_KEY, token)
    }

    pub fn get_tunnel_token(app: &AppHandle) -> Result<String, String> {
        Self::get(app, TOKEN_KEY)
    }

    pub fn delete_tunnel_token(app: &AppHandle) -> Result<(), String> {
        Self::remove(app, TOKEN_KEY)
    }

    // Hostname

    pub fn save_hostname(app: &AppHandle, hostname: &str) -> Result<(), String> {
        Self::set(app, HOSTNAME_KEY, hostname)
    }

    pub fn get_hostname(app: &AppHandle) -> Result<String, String> {
        Self::get(app, HOSTNAME_KEY)
    }

    pub fn delete_hostname(app: &AppHandle) -> Result<(), String> {
        Self::remove(app, HOSTNAME_KEY)
    }

    // Clear everything

    pub fn clear_all(app: &AppHandle) -> Result<(), String> {
        let _ = Self::delete_tunnel_token(app);
        let _ = Self::delete_hostname(app);

        Ok(())
    }

    pub fn save_device_private_key(app: &AppHandle, key: &str) -> Result<(), String> {
        Self::set(app, DEVICE_PRIVATE_KEY, key)
    }

    pub fn get_device_private_key(app: &AppHandle) -> Result<String, String> {
        Self::get(app, DEVICE_PRIVATE_KEY)
    }

    pub fn save_device_public_key(app: &AppHandle, key: &str) -> Result<(), String> {
        Self::set(app, DEVICE_PUBLIC_KEY, key)
    }

    // pub fn get_device_public_key(app: &AppHandle) -> Result<String, String> {
    //     Self::get(app, DEVICE_PUBLIC_KEY)
    // }

    pub fn get_device_public_key(app: &AppHandle) -> Result<String, String> {
        let private_key = Self::get(app, DEVICE_PRIVATE_KEY)?;
        derive_public_key(&private_key)
    }
}
