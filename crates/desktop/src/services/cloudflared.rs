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

            // `flatten()` drops each `Err` and asks for the next line. A line
            // that is not valid UTF-8 is an `Err`, and `read_line` has already
            // consumed it by the time it reports one, so each error costs a
            // line rather than the loop — `flatten()` reads the stream to the
            // end. The tests below pin that, because the `allow` depends on it.
            //
            // Clippy's suggestion, `map_while(Result::ok)`, would stop at the
            // first undecodable byte and silently truncate cloudflared's output.
            // Wrong for a log reader, so the lint is allowed deliberately.
            #[allow(clippy::lines_filter_map_ok)]
            for line in reader.lines().flatten() {
                println!("cloudflared: {}", line);
            }
        });

        // stderr
        let live = tunnel_live.clone();

        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);

            // Same as the stdout reader above — and here it matters more than
            // truncation: `tunnel_live` is set from a line that may arrive
            // after one that failed to decode, and a reader that stopped early
            // would never see it, so the tunnel would be declared dead.
            #[allow(clippy::lines_filter_map_ok)]
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
        let path = resource_dir.join("resources").join(relative_path);
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

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Cursor, ErrorKind};

    /// The premise the `allow(clippy::lines_filter_map_ok)` on the two output
    /// readers rests on, checked rather than assumed.
    ///
    /// `read_line` rejects a line that is not valid UTF-8 — and, this asserts,
    /// consumes it from the reader before doing so. That is what makes
    /// `lines().flatten()` safe here: each `Err` costs one bad line off the
    /// front of the stream and the next call reads the line after it, so the
    /// loop always makes progress.
    #[test]
    fn read_line_consumes_a_bad_line_before_reporting_it() {
        let bytes = b"first\n\xff\xfe not utf-8\nthird\n".to_vec();
        let mut reader = BufReader::new(Cursor::new(bytes));
        let mut line = String::new();

        assert_eq!(reader.read_line(&mut line).expect("valid line"), 6);
        assert_eq!(line, "first\n");

        line.clear();

        let error = reader
            .read_line(&mut line)
            .expect_err("the second line is not valid UTF-8");

        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(line.is_empty(), "a rejected line must not be appended to");

        // The one that matters: the bad line is behind us, not still in front.
        assert_eq!(reader.read_line(&mut line).expect("valid line"), 6);
        assert_eq!(line, "third\n");

        assert_eq!(reader.read_line(&mut line).expect("end of input"), 0);
    }

    /// The other half of the same premise, and the reason `flatten()` is kept
    /// over clippy's suggestion.
    ///
    /// The lint is allowed here for the same reason it is allowed in the
    /// readers: this test exists to show that the input it warns about does
    /// not, in fact, run forever.
    #[test]
    #[allow(clippy::lines_filter_map_ok)]
    fn flatten_keeps_draining_where_map_while_would_stop() {
        // The line the stderr reader watches for sits *after* a line that does
        // not decode, which is the whole point.
        let bytes =
            b"one line\n\xff\xfe undecodable\nRegistered tunnel connection\nlast\n".to_vec();

        let drained: Vec<String> = BufReader::new(Cursor::new(bytes.clone()))
            .lines()
            .flatten()
            .collect();

        assert_eq!(
            drained,
            vec!["one line", "Registered tunnel connection", "last"],
            "flatten() must skip the bad line and read to the end"
        );
        assert!(
            drained
                .iter()
                .any(|line| line.contains("Registered tunnel connection")),
            "the bad line must not hide the one that flips tunnel_live"
        );

        // Clippy's suggestion, on the same input, for contrast: correct for a
        // reader whose errors are terminal, wrong for this one, where a read
        // error costs a line rather than the stream.
        let stopped: Vec<String> = BufReader::new(Cursor::new(bytes))
            .lines()
            .map_while(Result::ok)
            .collect();

        assert_eq!(stopped, vec!["one line"]);
        assert!(
            !stopped
                .iter()
                .any(|line| line.contains("Registered tunnel connection")),
            "map_while(Result::ok) would truncate cloudflared's output here"
        );
    }
}
