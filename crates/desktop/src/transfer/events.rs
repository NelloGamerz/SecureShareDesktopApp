use crate::models::progress::TransferProgress;
use tauri::{AppHandle, Emitter};
pub fn emit_progress(app: &AppHandle, name: &str, p: TransferProgress) {
    if let Err(e) = app.emit(name, p) {
        tracing::warn!(error=%e,"failed to emit transfer event")
    }
}
