#[derive(Debug, Clone)]
pub struct FileInfo {
    pub id: String,
    pub path: std::path::PathBuf,
    pub relative_path: String,
    pub size: u64,
}
