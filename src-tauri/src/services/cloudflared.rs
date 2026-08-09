use crate::utils::config::AppConfig;
use std::env;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, State};
use tracing::{error, info, warn};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use crate::services::keyring_service::KeyringService;
// use crate::services::SecureStorage;
use crate::state::cloudflared_state::Cloudflared;

pub struct CloudflaredService;

impl CloudflaredService {
    pub fn start(
        app: &AppHandle,
        state: State<Cloudflared>,
        // storage: State<SecureStorage>,
        hostname: String,
    ) -> Result<String, String> {
        info!("========== CLOUDFLARED START ==========");

        let token = KeyringService::get_tunnel_token(app)?;
        info!("Tunnel token loaded");

        // let path = Self::get_binary_path(app)?;
        let path = match Self::get_binary_path(app) {
            Ok(path) => path,
            Err(e) => {
                error!("get_binary_path failed: {}", e);
                return Err(e);
            }
        };
        info!("Resolved cloudflared path: {:?}", path);

        info!("Exists: {}", path.exists());

        match std::fs::metadata(&path) {
            Ok(meta) => {
                info!("Binary size: {} bytes", meta.len());
            }
            Err(e) => {
                error!("Unable to read binary metadata: {}", e);
            }
        }

        info!("Launching cloudflared process...");

        // let mut child = Command::new(&path)
        //     .args(["tunnel", "--no-autoupdate", "run", "--token", token.trim()])
        //     .stdout(Stdio::piped())
        //     .stderr(Stdio::piped())
        //     .spawn()
        //     .map_err(|e| {
        //         error!("Failed to spawn cloudflared: {}", e);
        //         e.to_string()
        //     })?;

        let mut command = Command::new(&path);

        command
            .args(["tunnel", "--no-autoupdate", "run", "--token", token.trim()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let mut child = command.spawn().map_err(|e| {
            error!("Failed to spawn cloudflared: {}", e);
            e.to_string()
        })?;

        info!("cloudflared process started");
        info!("PID = {}", child.id());

        // let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;

        // let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;

        // *state.process.lock().unwrap() = Some(child);

        let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;

        info!("stdout captured");

        let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;

        info!("stderr captured");

        *state.process.lock().unwrap() = Some(child);

        info!("Process stored in state");

        let tunnel_live = Arc::new(Mutex::new(false));

        // stdout
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);

            for line in reader.lines().flatten() {
                println!("cloudflared: {}", line);
            }
        });

        // stderr
        let live = tunnel_live.clone();

        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);

            for line in reader.lines().flatten() {
                eprintln!("cloudflared: {}", line);

                if line.contains("Registered tunnel connection") {
                    *live.lock().unwrap() = true;
                }
            }
        });

        let start = Instant::now();

        while start.elapsed() < Duration::from_secs(15) {
            if *tunnel_live.lock().unwrap() {
                return Ok(format!("Tunnel LIVE → {}", hostname));
            }

            std::thread::sleep(Duration::from_millis(200));
        }

        Err("Tunnel failed to start".into())
    }

    pub fn stop(state: State<Cloudflared>) -> Result<(), String> {
        let mut guard = state.process.lock().map_err(|_| "State lock poisoned")?;

        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();

            println!("cloudflared tunnel stopped");

            Ok(())
        } else {
            Err("Tunnel is not running".into())
        }
    }

    pub fn is_active(state: State<Cloudflared>) -> Result<bool, String> {
        let mut guard = state.process.lock().map_err(|_| "State lock poisoned")?;

        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    *guard = None;

                    Ok(false)
                }

                Ok(None) => Ok(true),

                Err(e) => Err(e.to_string()),
            }
        } else {
            Ok(false)
        }
    }

    // fn get_binary_path(app: &AppHandle) -> Result<PathBuf, String> {
    //     let os = env::consts::OS;
    //     let arch = env::consts::ARCH;

    //     info!("========== RESOLVING CLOUDFLARED BINARY ==========");
    //     info!("OS: {}", os);
    //     info!("ARCH: {}", arch);

    //     let relative_path = match (os, arch) {
    //         ("windows", "x86_64") => "cloudflared/windows-x64/cloudflared.exe",
    //         ("macos", "x86_64") => "cloudflared/macos-x64/cloudflared",
    //         ("macos", "aarch64") => "cloudflared/macos-arm64/cloudflared",
    //         ("linux", "x86_64") => "cloudflared/linux-x64/cloudflared",
    //         ("linux", "aarch64") => "cloudflared/linux-arm64/cloudflared",
    //         _ => return Err(format!("Unsupported platform {} {}", os, arch)),
    //     };

    //     info!("Expected relative resource path: {}", relative_path);

    //     let config = if cfg!(debug_assertions) {
    //         AppConfig::development()
    //     } else {
    //         AppConfig::production()
    //     };

    //     // let path = if cfg!(debug_assertions) {
    //     //     info!("Running in DEBUG mode");
    //     let path = if config.environment == "development" {
    //         info!("Running in DEVELOPMENT");

    //         let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    //         info!("Current working directory: {:?}", cwd);

    //         cwd.join("resources").join(relative_path)
    //     } else {
    //         // info!("Running in RELEASE mode");

    //         // let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;

    //         // info!("Resource directory: {:?}", resource_dir);

    //         // resource_dir.join(relative_path)
    //         info!("Running in PRODUCTION");

    //         // resources/cloudflared/windows-x64/cloudflared.exe
    //         let exe_dir = std::env::current_exe()
    //             .map_err(|e| e.to_string())?
    //             .parent()
    //             .ok_or("Failed to get executable directory")?
    //             .to_path_buf();

    //         exe_dir.join("resources").join(relative_path)
    //     };

    //     info!("Looking for cloudflared binary at:");
    //     info!("{:?}", path);

    //     match std::fs::canonicalize(&path) {
    //         Ok(real) => info!("Canonical path: {:?}", real),
    //         Err(e) => warn!("Could not canonicalize path: {}", e),
    //     }

    //     info!("File exists: {}", path.exists());

    //     match std::fs::metadata(&path) {
    //         Ok(meta) => {
    //             info!("File size: {} bytes", meta.len());
    //             info!("Readonly: {}", meta.permissions().readonly());
    //         }
    //         Err(e) => {
    //             warn!("Metadata unavailable: {}", e);
    //         }
    //     }

    //     if !path.exists() {
    //         error!("cloudflared binary NOT FOUND at {:?}", path);
    //         return Err(format!("cloudflared not found at {:?}", path));
    //     }

    //     info!("Using cloudflared binary: {:?}", path);

    //     Ok(path)
    // }

    fn get_binary_path(app: &AppHandle) -> Result<PathBuf, String> {
        let os = env::consts::OS;
        let arch = env::consts::ARCH;
        info!("========== RESOLVING CLOUDFLARED BINARY ==========");
        info!("OS: {}", os);
        info!("ARCH: {}", arch);
        let relative_path = match (os, arch) {
            ("windows", "x86_64") => "cloudflared/windows-x64/cloudflared.exe",
            ("macos", "x86_64") => "cloudflared/macos-x64/cloudflared",
            ("macos", "aarch64") => "cloudflared/macos-arm64/cloudflared",
            ("linux", "x86_64") => "cloudflared/linux-x64/cloudflared",
            ("linux", "aarch64") => "cloudflared/linux-arm64/cloudflared",
            _ => return Err(format!("Unsupported platform {} {}", os, arch)),
        };
        info!("Expected relative resource path: {}", relative_path);
        let resource_dir = app.path().resource_dir().map_err(|e| {
            error!("Failed to resolve Tauri resource directory: {}", e);
            e.to_string()
        })?;
        info!("Tauri resource directory: {:?}", resource_dir);
        let path = resource_dir.join(relative_path);
        info!("Looking for cloudflared binary at:");
        info!("{:?}", path);
        match std::fs::canonicalize(&path) {
            Ok(real) => info!("Canonical path: {:?}", real),
            Err(e) => warn!("Could not canonicalize path: {}", e),
        }
        info!("File exists: {}", path.exists());
        match std::fs::metadata(&path) {
            Ok(meta) => {
                info!("File size: {} bytes", meta.len());
                info!("Readonly: {}", meta.permissions().readonly());
            }
            Err(e) => {
                warn!("Metadata unavailable: {}", e);
            }
        }
        if !path.exists() {
            error!("cloudflared binary NOT FOUND at {:?}", path);
            return Err(format!("cloudflared not found at {:?}", path));
        }
        info!("Using cloudflared binary: {:?}", path);
        Ok(path)
    }
}
