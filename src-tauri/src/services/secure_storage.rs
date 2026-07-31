// use std::{
//     fs,
//     path::Path,
//     sync::{Arc, OnceLock},
// };

// use tauri::{AppHandle, Manager};
// use tauri_plugin_stronghold::stronghold::Stronghold;
// use iota_stronghold::Client;
// // use tauri_plugin_stronghold::stronghold::{Client, Stronghold};

// const CLIENT_NAMESPACE: &str = "my-tauri-app";
// const VAULT_FILE_NAME: &str = "vault.hold";

// static SHARED_STORAGE: OnceLock<Arc<SecureStorage>> = OnceLock::new();

// #[derive(Clone)]
// pub struct SecureStorage {
//     stronghold: Arc<Stronghold>,
//     client_namespace: String,
//     vault_path: std::path::PathBuf,
// }

// impl SecureStorage {
//     pub fn initialize(app: &AppHandle) -> Result<Arc<Self>, String> {
//         let app_data_dir = app
//             .path()
//             .app_data_dir()
//             .map_err(|error| format!("failed to resolve app data directory: {error}"))?;

//         let vault_path = app_data_dir.join(VAULT_FILE_NAME);
//         let parent_dir = vault_path.parent().unwrap_or_else(|| Path::new("."));

//         if !parent_dir.exists() {
//             fs::create_dir_all(parent_dir).map_err(|error| {
//                 format!("failed to create Stronghold storage directory: {error}")
//             })?;
//         }

//         let password = Self::password();
//         let stronghold = Arc::new(
//             Stronghold::new(&vault_path, password.as_bytes().to_vec())
//                 .map_err(|error| format!("failed to initialize Stronghold vault: {error}"))?,
//         );

//         let client_namespace = CLIENT_NAMESPACE.to_string();

//         match stronghold.get_client(client_namespace.as_bytes()) {
//             Ok(_) => {
//                 tracing::info!(
//                     path = %vault_path.display(),
//                     namespace = %client_namespace,
//                     "Loaded existing Stronghold client"
//                 );
//             }
//             Err(error) => {
//                 tracing::info!(
//                     path = %vault_path.display(),
//                     namespace = %client_namespace,
//                     error = %error,
//                     "Creating Stronghold client"
//                 );
//                 stronghold
//                     .create_client(client_namespace.as_bytes())
//                     .map_err(|error| format!("failed to create Stronghold client: {error}"))?;
//             }
//         }

//         let storage = Arc::new(Self {
//             stronghold,
//             client_namespace,
//             vault_path,
//         });

//         Self::set_shared_storage(storage.clone());

//         Ok(storage)
//     }

//     pub fn set_shared_storage(storage: Arc<Self>) -> bool {
//         SHARED_STORAGE.set(storage).is_ok()
//     }

//     pub fn shared_storage() -> Result<Arc<Self>, String> {
//         SHARED_STORAGE
//             .get()
//             .cloned()
//             .ok_or_else(|| "secure storage not initialized".to_string())
//     }

//     pub fn insert_secret(&self, key: &str, value: &str) -> Result<(), String> {
//         let client = self.client()?;
//         client
//             .store()
//             .insert(key.as_bytes().to_vec(), value.as_bytes().to_vec(), None)
//             .map_err(|error| format!("secure store insert failed: {error}"))?;
//         self.save()
//     }

//     pub fn read_secret(&self, key: &str) -> Result<String, String> {
//         let client = self.client()?;
//         match client
//             .store()
//             .get(key.as_bytes())
//             .map_err(|error| format!("secure store read failed: {error}"))?
//         {
//             Some(bytes) => String::from_utf8(bytes)
//                 .map_err(|error| format!("secure store payload is invalid UTF-8: {error}")),
//             None => Err("TOKEN_NOT_FOUND".into()),
//         }
//     }

//     pub fn delete_secret(&self, key: &str) -> Result<(), String> {
//         let client = self.client()?;
//         client
//             .store()
//             .delete(key.as_bytes())
//             .map_err(|error| format!("secure store delete failed: {error}"))?;
//         self.save()
//     }

//     // fn client(&self) -> Result<iota_stronghold::Client, String> {
//     //     self.stronghold
//     //         .get_client(self.client_namespace.as_bytes())
//     //         .map_err(|error| format!("failed to access Stronghold client: {error}"))
//     // }

//     fn client(&self) -> Result<Client, String> {
//         self.stronghold
//             .get_client(self.client_namespace.as_bytes())
//             .map_err(|error| format!("failed to access Stronghold client: {error}"))
//     }

//     fn save(&self) -> Result<(), String> {
//         self.stronghold
//             .save()
//             .map_err(|error| format!("failed to save Stronghold vault: {error}"))
//     }

//     fn password() -> String {
//         std::env::var("TAURI_STRONGHOLD_PASSPHRASE")
//             .unwrap_or_else(|_| format!("my-tauri-app-stronghold::{}", env!("CARGO_PKG_NAME")))
//     }
// }

// use std::sync::{Arc, OnceLock};

// use tauri::{AppHandle, Manager};
// // use tauri_plugin_stronghold::stronghold::{Client, Stronghold};
// use iota_stronghold::Client;
// use tauri_plugin_stronghold::stronghold::Stronghold;

// const CLIENT_NAMESPACE: &str = "my-tauri-app";

// static SHARED_STORAGE: OnceLock<Arc<SecureStorage>> = OnceLock::new();

// // pub struct SecureStorage {
// //     stronghold: &'static Stronghold,
// //     client_namespace: String,
// // }

// pub struct SecureStorage {
//     stronghold: Arc<Stronghold>,
//     client_namespace: String,
// }

// impl SecureStorage {
//     pub fn initialize(app: &AppHandle) -> Result<Arc<Self>, String> {
//         let stronghold = app.state::<Stronghold>().inner();

//         let client_namespace = CLIENT_NAMESPACE.to_string();

//         match stronghold.get_client(client_namespace.as_bytes()) {
//             Ok(_) => {
//                 tracing::info!(
//                     namespace = %client_namespace,
//                     "Loaded existing Stronghold client"
//                 );
//             }

//             Err(error) => {
//                 tracing::info!(
//                     namespace = %client_namespace,
//                     error = %error,
//                     "Creating Stronghold client"
//                 );

//                 stronghold
//                     .create_client(client_namespace.as_bytes())
//                     .map_err(|error| format!("failed to create Stronghold client: {error}"))?;
//             }
//         }

//         let storage = Arc::new(Self {
//             stronghold,
//             client_namespace,
//         });

//         Self::set_shared_storage(storage.clone());

//         Ok(storage)
//     }

//     pub fn set_shared_storage(storage: Arc<Self>) -> bool {
//         SHARED_STORAGE.set(storage).is_ok()
//     }

//     pub fn shared_storage() -> Result<Arc<Self>, String> {
//         SHARED_STORAGE
//             .get()
//             .cloned()
//             .ok_or_else(|| "secure storage not initialized".into())
//     }

//     pub fn insert_secret(&self, key: &str, value: &str) -> Result<(), String> {
//         let client = self.client()?;

//         client
//             .store()
//             .insert(key.as_bytes().to_vec(), value.as_bytes().to_vec(), None)
//             .map_err(|error| format!("secure store insert failed: {error}"))?;

//         self.save()
//     }

//     pub fn read_secret(&self, key: &str) -> Result<String, String> {
//         let client = self.client()?;

//         let value = client
//             .store()
//             .get(key.as_bytes())
//             .map_err(|error| format!("secure store read failed: {error}"))?;

//         match value {
//             Some(bytes) => {
//                 String::from_utf8(bytes).map_err(|error| format!("invalid UTF-8 data: {error}"))
//             }

//             None => Err("TOKEN_NOT_FOUND".into()),
//         }
//     }

//     pub fn delete_secret(&self, key: &str) -> Result<(), String> {
//         let client = self.client()?;

//         client
//             .store()
//             .delete(key.as_bytes())
//             .map_err(|error| format!("secure store delete failed: {error}"))?;

//         self.save()
//     }

//     fn client(&self) -> Result<Client, String> {
//         self.stronghold
//             .get_client(self.client_namespace.as_bytes())
//             .map_err(|error| format!("failed to access Stronghold client: {error}"))
//     }

//     fn save(&self) -> Result<(), String> {
//         self.stronghold
//             .save()
//             .map_err(|error| format!("failed to save Stronghold vault: {error}"))
//     }
// }

// use std::sync::{Arc, OnceLock};

// use iota_stronghold::Client;
// use tauri::{AppHandle, Manager};
// use tauri_plugin_stronghold::stronghold::Stronghold;

// const CLIENT_NAMESPACE: &str = "my-tauri-app";

// static SHARED_STORAGE: OnceLock<Arc<SecureStorage>> = OnceLock::new();

// // pub struct SecureStorage {
// //     app: AppHandle,
// //     client_namespace: String,
// // }

// pub struct SecureStorage {
//     stronghold: Arc<Stronghold>,
//     client_namespace: String,
// }

// impl SecureStorage {
//     pub fn initialize(app: &AppHandle, stronghold: Arc<Stronghold>) -> Result<Arc<Self>, String> {
//         let client_namespace = CLIENT_NAMESPACE.to_string();

//         match stronghold.get_client(client_namespace.as_bytes()) {
//             Ok(_) => {
//                 tracing::info!(
//                     namespace = %client_namespace,
//                     "Loaded existing Stronghold client"
//                 );
//             }

//             Err(error) => {
//                 tracing::info!(
//                     namespace = %client_namespace,
//                     error = %error,
//                     "Creating Stronghold client"
//                 );

//                 stronghold
//                     .create_client(client_namespace.as_bytes())
//                     .map_err(|e| format!("failed to create Stronghold client: {e}"))?;
//             }
//         }

//         let storage = Arc::new(Self {
//             stronghold,
//             client_namespace,
//         });

//         Self::set_shared_storage(storage.clone());

//         Ok(storage)
//     }

//     pub fn set_shared_storage(storage: Arc<Self>) -> bool {
//         SHARED_STORAGE.set(storage).is_ok()
//     }

//     pub fn shared_storage() -> Result<Arc<Self>, String> {
//         SHARED_STORAGE
//             .get()
//             .cloned()
//             .ok_or_else(|| "secure storage not initialized".into())
//     }

//     fn stronghold(&self) -> &Stronghold {
//         self.stronghold.as_ref()
//     }

//     pub fn insert_secret(&self, key: &str, value: &str) -> Result<(), String> {
//         let client = self.client()?;

//         client
//             .store()
//             .insert(key.as_bytes().to_vec(), value.as_bytes().to_vec(), None)
//             .map_err(|error| format!("secure store insert failed: {error}"))?;

//         self.save()
//     }

//     pub fn read_secret(&self, key: &str) -> Result<String, String> {
//         let client = self.client()?;

//         let value = client
//             .store()
//             .get(key.as_bytes())
//             .map_err(|error| format!("secure store read failed: {error}"))?;

//         match value {
//             Some(bytes) => {
//                 String::from_utf8(bytes).map_err(|error| format!("invalid UTF-8 data: {error}"))
//             }

//             None => Err("TOKEN_NOT_FOUND".into()),
//         }
//     }

//     pub fn delete_secret(&self, key: &str) -> Result<(), String> {
//         let client = self.client()?;

//         client
//             .store()
//             .delete(key.as_bytes())
//             .map_err(|error| format!("secure store delete failed: {error}"))?;

//         self.save()
//     }

//     fn client(&self) -> Result<Client, String> {
//         self.stronghold()
//             .get_client(self.client_namespace.as_bytes())
//             .map_err(|error| format!("failed to access Stronghold client: {error}"))
//     }

//     fn save(&self) -> Result<(), String> {
//         self.stronghold()
//             .save()
//             .map_err(|error| format!("failed to save Stronghold vault: {error}"))
//     }
// }

// use std::sync::{Arc, OnceLock};

// use iota_stronghold::Client;
// use tauri::{AppHandle, Manager};
// use tauri_plugin_stronghold::stronghold::Stronghold;

// const CLIENT_NAMESPACE: &str = "my-tauri-app";

// static SHARED_STORAGE: OnceLock<Arc<SecureStorage>> = OnceLock::new();

// pub struct SecureStorage {
//     app: AppHandle,
//     client_namespace: String,
// }

// impl SecureStorage {
//     pub fn initialize(app: &AppHandle) -> Result<Arc<Self>, String> {
//         let client_namespace = CLIENT_NAMESPACE.to_string();

//         // Only use plugin managed Stronghold here
//         let stronghold = app
//             .try_state::<Stronghold>()
//             .ok_or_else(|| "Stronghold plugin is not initialized".to_string())?;

//         match stronghold.get_client(client_namespace.as_bytes()) {
//             Ok(_) => {
//                 tracing::info!(
//                     namespace = %client_namespace,
//                     "Loaded existing Stronghold client"
//                 );
//             }

//             Err(_) => {
//                 stronghold
//                     .create_client(client_namespace.as_bytes())
//                     .map_err(|e| format!("failed to create Stronghold client: {e}"))?;

//                 tracing::info!(
//                     namespace = %client_namespace,
//                     "Created Stronghold client"
//                 );
//             }
//         }

//         let storage = Arc::new(Self {
//             app: app.clone(),
//             client_namespace,
//         });

//         SHARED_STORAGE
//             .set(storage.clone())
//             .map_err(|_| "SecureStorage already initialized".to_string())?;

//         Ok(storage)
//     }

//     pub fn shared_storage() -> Result<Arc<Self>, String> {
//         SHARED_STORAGE
//             .get()
//             .cloned()
//             .ok_or_else(|| "secure storage not initialized".into())
//     }

//     fn stronghold(&self) -> Result<tauri::State<'_, Stronghold>, String> {
//         self.app
//             .try_state::<Stronghold>()
//             .ok_or_else(|| "Stronghold plugin is not initialized".to_string())
//     }

//     pub fn insert_secret(&self, key: &str, value: &str) -> Result<(), String> {
//         let client = self.client()?;

//         client
//             .store()
//             .insert(key.as_bytes().to_vec(), value.as_bytes().to_vec(), None)
//             .map_err(|e| format!("insert failed: {e}"))?;

//         self.save()
//     }

//     pub fn read_secret(&self, key: &str) -> Result<String, String> {
//         let client = self.client()?;

//         let value = client
//             .store()
//             .get(key.as_bytes())
//             .map_err(|e| format!("read failed: {e}"))?;

//         match value {
//             Some(bytes) => String::from_utf8(bytes).map_err(|e| format!("utf8 error: {e}")),
//             None => Err("TOKEN_NOT_FOUND".into()),
//         }
//     }

//     pub fn delete_secret(&self, key: &str) -> Result<(), String> {
//         let client = self.client()?;

//         client
//             .store()
//             .delete(key.as_bytes())
//             .map_err(|e| format!("delete failed: {e}"))?;

//         self.save()
//     }

//     fn client(&self) -> Result<Client, String> {
//         self.stronghold()?
//             .get_client(self.client_namespace.as_bytes())
//             .map_err(|e| format!("client error: {e}"))
//     }

//     fn save(&self) -> Result<(), String> {
//         self.stronghold()?
//             .save()
//             .map_err(|e| format!("save failed: {e}"))
//     }
// }

// use iota_stronghold::Client;
// use tauri::{AppHandle, Manager};
// use tauri_plugin_stronghold::stronghold::Stronghold;

// const CLIENT_NAMESPACE: &str = "my-tauri-app";

// pub struct SecureStorage {
//     app: AppHandle,
//     client_namespace: String,
// }

// impl SecureStorage {
//     // pub fn initialize(app: &AppHandle) -> Result<Self, String> {
//     //     // Verify that the Stronghold plugin has already initialized Stronghold
//     //     app.try_state::<Stronghold>()
//     //         .ok_or_else(|| "Stronghold plugin is not initialized".to_string())?;

//     //     Ok(Self {
//     //         app: app.clone(),
//     //         client_namespace: CLIENT_NAMESPACE.to_string(),
//     //     })
//     // }

//     pub fn initialize(app: &AppHandle) -> Result<Self, String> {
//         let stronghold = app.state::<Stronghold>();

//         tracing::info!("Stronghold plugin loaded");

//         Ok(Self {
//             app: app.clone(),
//             client_namespace: CLIENT_NAMESPACE.to_string(),
//         })
//     }

//     fn stronghold(&self) -> Result<tauri::State<'_, Stronghold>, String> {
//         self.app
//             .try_state::<Stronghold>()
//             .ok_or_else(|| "Stronghold plugin is not initialized".to_string())
//     }

//     fn client(&self) -> Result<Client, String> {
//         let stronghold = self.stronghold()?;

//         match stronghold.get_client(self.client_namespace.as_bytes()) {
//             Ok(client) => Ok(client),

//             Err(_) => {
//                 tracing::info!(
//                     namespace = %self.client_namespace,
//                     "Creating Stronghold client"
//                 );

//                 stronghold
//                     .create_client(self.client_namespace.as_bytes())
//                     .map_err(|e| format!("failed to create Stronghold client: {e}"))
//             }
//         }
//     }

//     pub fn insert_secret(&self, key: &str, value: &str) -> Result<(), String> {
//         let client = self.client()?;

//         client
//             .store()
//             .insert(key.as_bytes().to_vec(), value.as_bytes().to_vec(), None)
//             .map_err(|e| format!("insert failed: {e}"))?;

//         self.save()
//     }

//     pub fn read_secret(&self, key: &str) -> Result<String, String> {
//         let client = self.client()?;

//         let value = client
//             .store()
//             .get(key.as_bytes())
//             .map_err(|e| format!("read failed: {e}"))?;

//         match value {
//             Some(bytes) => {
//                 String::from_utf8(bytes).map_err(|e| format!("utf8 conversion failed: {e}"))
//             }

//             None => Err("SECRET_NOT_FOUND".into()),
//         }
//     }

//     pub fn delete_secret(&self, key: &str) -> Result<(), String> {
//         let client = self.client()?;

//         client
//             .store()
//             .delete(key.as_bytes())
//             .map_err(|e| format!("delete failed: {e}"))?;

//         self.save()
//     }

//     fn save(&self) -> Result<(), String> {
//         self.stronghold()?
//             .save()
//             .map_err(|e| format!("stronghold save failed: {e}"))
//     }
// }


// use tauri::{AppHandle, Manager};
// use tauri_plugin_secure_storage::desktop::SecureStorage as TauriSecureStorage;

// pub struct SecureStorage {
//     app: AppHandle,
// }

// impl SecureStorage {
//     pub fn initialize(app: &AppHandle) -> Result<Self, String> {
//         // Verify secure storage plugin exists
//         app.try_state::<TauriSecureStorage>()
//             .ok_or_else(|| "Secure storage plugin is not initialized".to_string())?;

//         tracing::info!("Secure storage plugin loaded");

//         Ok(Self { app: app.clone() })
//     }

//     fn storage(&self) -> Result<tauri::State<'_, TauriSecureStorage>, String> {
//         self.app
//             .try_state::<TauriSecureStorage>()
//             .ok_or_else(|| "Secure storage plugin is not initialized".to_string())
//     }

//     pub fn insert_secret(&self, key: &str, value: &str) -> Result<(), String> {
//         self.storage()?
//             .set(key, value.to_string())
//             .map_err(|e| format!("insert failed: {e}"))
//     }

//     pub fn read_secret(&self, key: &str) -> Result<String, String> {
//         let value = self
//             .storage()?
//             .get(key)
//             .map_err(|e| format!("read failed: {e}"))?;

//         value.ok_or_else(|| "SECRET_NOT_FOUND".to_string())
//     }

//     pub fn delete_secret(&self, key: &str) -> Result<(), String> {
//         self.storage()?
//             .delete(key)
//             .map_err(|e| format!("delete failed: {e}"))
//     }
// }
