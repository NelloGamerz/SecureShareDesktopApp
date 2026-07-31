use crate::services::{
    generate_device_keypair::generate_device_keypair, keyring_service::KeyringService,
};
use sysinfo::{Components, System};
use tauri::AppHandle;

#[tauri::command]
pub async fn create_device_identity(app: AppHandle) -> Result<String, String> {
    // Check if device already has keys
    if let Ok(public_key) = KeyringService::get_device_public_key(&app) {
        return Ok(public_key);
    }

    let keys = generate_device_keypair();

    KeyringService::save_device_private_key(&app, &keys.private_key)?;

    KeyringService::save_device_public_key(&app, &keys.public_key)?;

    Ok(keys.public_key)
}

// #[tauri::command]
#[tauri::command]
pub async fn detect_device_type() -> String {
    #[cfg(target_os = "windows")]
    {
        return detect_windows_device_type();
    }

    #[cfg(target_os = "macos")]
    {
        return detect_macos_device_type();
    }

    #[cfg(target_os = "linux")]
    {
        return detect_linux_device_type();
    }

    "UNKNOWN".into()
}

#[cfg(target_os = "windows")]
fn detect_windows_device_type() -> String {

    use std::process::Command;

    let output = Command::new("powershell")
        .args([
            "-Command",
            "Get-CimInstance Win32_Battery"
        ])
        .output();


    match output {

        Ok(result) => {

            let battery = String::from_utf8_lossy(
                &result.stdout
            );


            if !battery.trim().is_empty() {
                return "LAPTOP".into();
            }


            "DESKTOP".into()
        }


        Err(_) => "UNKNOWN".into()
    }
}

#[cfg(target_os = "macos")]
fn detect_macos_device_type() -> String {

    use std::process::Command;


    let output = Command::new("sh")
        .args([
            "-c",
            "system_profiler SPPowerDataType"
        ])
        .output();


    if let Ok(result) = output {

        let text =
            String::from_utf8_lossy(&result.stdout);


        if text.contains("Battery Information") {
            return "LAPTOP".into();
        }
    }


    "DESKTOP".into()
}

#[cfg(target_os = "linux")]
fn detect_linux_device_type() -> String {

    use std::path::Path;


    if Path::new("/sys/class/power_supply/BAT0")
        .exists()
    {
        return "LAPTOP".into();
    }


    "DESKTOP".into()
}