use crate::services::generate_device_keypair::derive_public_key;
use tauri::AppHandle;
use tauri_plugin_secure_storage::{OptionsRequest, SecureStorageExt};

const TOKEN_KEY: &str = "tunnel_token";
const HOSTNAME_KEY: &str = "tunnel_hostname";
const DEVICE_PRIVATE_KEY: &str = "device_private_key";
const DEVICE_PUBLIC_KEY: &str = "device_public_key";

/*
 * Desktop session, persisted so a restart does not sign the user out.
 *
 * The credentials are split across separate keys rather than stored as one JSON
 * blob: the Windows credential store caps a single secret at 2560 bytes, and an
 * access and refresh token together can exceed that.
 */
const AUTH_ACCESS_TOKEN: &str = "auth_access_token";
const AUTH_REFRESH_TOKEN: &str = "auth_refresh_token";
const AUTH_EXPIRES_AT: &str = "auth_expires_at";
const AUTH_USER_ID: &str = "auth_user_id";
const AUTH_TOKEN_ENDPOINT: &str = "auth_token_endpoint";
const AUTH_CLIENT_ID: &str = "auth_client_id";

const AUTH_KEYS: [&str; 6] = [
    AUTH_ACCESS_TOKEN,
    AUTH_REFRESH_TOKEN,
    AUTH_EXPIRES_AT,
    AUTH_USER_ID,
    AUTH_TOKEN_ENDPOINT,
    AUTH_CLIENT_ID,
];

/// The credential-bearing state of a desktop session, as written to the OS
/// keychain.
#[derive(Debug, Clone, Default)]
pub struct StoredAuthSession {
    pub access_token: String,
    /// Absent when the issuer did not grant `offline_access`.
    pub refresh_token: Option<String>,
    /// Unix epoch seconds at which `access_token` expires.
    pub expires_at: Option<i64>,
    pub user_id: Option<String>,
    /// Retained so a refresh can run without the frontend re-supplying config.
    pub token_endpoint: Option<String>,
    pub client_id: Option<String>,
}

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

    /// Reads a key, resolving `Ok(None)` when it was never written.
    ///
    /// Distinct from [`Self::get`], which reports a missing key as an error.
    fn get_optional(app: &AppHandle, key: &str) -> Result<Option<String>, String> {
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

        Ok(result.data)
    }

    /// Writes a key, or clears it when the value is absent.
    fn set_optional(app: &AppHandle, key: &str, value: Option<&str>) -> Result<(), String> {
        match value {
            Some(value) => Self::set(app, key, value),
            // The key may never have been written, so a failed delete is not an
            // error — the desired end state (absent) already holds.
            None => {
                let _ = Self::remove(app, key);
                Ok(())
            }
        }
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

    // Desktop session

    /// Persists the desktop session so a restart can resume it.
    ///
    /// The access token is the marker of a stored session: without it there is
    /// nothing to resume, so this is the only field that must be written.
    pub fn save_auth_session(app: &AppHandle, session: &StoredAuthSession) -> Result<(), String> {
        if session.access_token.is_empty() {
            return Err("refusing to persist an empty access token".to_string());
        }

        Self::set(app, AUTH_ACCESS_TOKEN, &session.access_token)?;
        Self::set_optional(app, AUTH_REFRESH_TOKEN, session.refresh_token.as_deref())?;
        Self::set_optional(
            app,
            AUTH_EXPIRES_AT,
            session.expires_at.map(|value| value.to_string()).as_deref(),
        )?;
        Self::set_optional(app, AUTH_USER_ID, session.user_id.as_deref())?;
        Self::set_optional(app, AUTH_TOKEN_ENDPOINT, session.token_endpoint.as_deref())?;
        Self::set_optional(app, AUTH_CLIENT_ID, session.client_id.as_deref())?;

        Ok(())
    }

    /// Reads a persisted desktop session.
    ///
    /// Resolves `Ok(None)` when the user has never signed in, or has since
    /// logged out. A partially written session is treated as absent unless the
    /// access token is present, since nothing can be resumed without it.
    pub fn load_auth_session(app: &AppHandle) -> Result<Option<StoredAuthSession>, String> {
        let access_token = match Self::get_optional(app, AUTH_ACCESS_TOKEN)? {
            Some(value) if !value.is_empty() => value,
            _ => return Ok(None),
        };

        Ok(Some(StoredAuthSession {
            access_token,
            refresh_token: Self::get_optional(app, AUTH_REFRESH_TOKEN)?,
            // A value that cannot be parsed is treated as unknown rather than
            // fatal; the token is then refreshed lazily on first use.
            expires_at: Self::get_optional(app, AUTH_EXPIRES_AT)?.and_then(|value| value.parse().ok()),
            user_id: Self::get_optional(app, AUTH_USER_ID)?,
            token_endpoint: Self::get_optional(app, AUTH_TOKEN_ENDPOINT)?,
            client_id: Self::get_optional(app, AUTH_CLIENT_ID)?,
        }))
    }

    /// Removes every persisted session field. Safe to call when none exist.
    pub fn delete_auth_session(app: &AppHandle) -> Result<(), String> {
        for key in AUTH_KEYS {
            // Best-effort: the key may never have been written, and a failure to
            // delete one must not leave the others behind.
            let _ = Self::remove(app, key);
        }

        Ok(())
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
