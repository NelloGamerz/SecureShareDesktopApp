#[derive(Debug, Clone)]
pub struct FileInfo {
    pub id: String,
    pub path: std::path::PathBuf,
    pub relative_path: String,
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch, when the platform
    /// reported one.
    ///
    /// Recorded when the file was scanned so that a transfer restored from disk
    /// can tell whether the file it planned digests for is still the file on the
    /// disk. It is not used for the wire or for any digest: it only decides
    /// whether a persisted digest plan may be reused or has to be recomputed.
    /// `None` means "unknown", and a plan whose stamp is unknown is never
    /// trusted.
    pub modified_nanos: Option<u64>,
}

/// A file's modification time as nanoseconds since the Unix epoch.
///
/// Returns `None` when the platform does not report a modification time, or
/// reports one before 1970 — both are treated as "no stamp", which makes a
/// persisted digest plan recompute rather than be trusted.
pub fn modified_nanos(metadata: &std::fs::Metadata) -> Option<u64> {
    let modified = metadata.modified().ok()?;

    let since_epoch = modified.duration_since(std::time::UNIX_EPOCH).ok()?;

    Some(since_epoch.as_nanos().min(u64::MAX as u128) as u64)
}
