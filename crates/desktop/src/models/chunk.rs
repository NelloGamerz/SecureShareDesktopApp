#[derive(Debug, Clone)]
pub struct ChunkJob {
    pub transfer_id: String,
    pub file_id: String,
    pub index: u64,
    pub total: u64,
    pub relative_path: String,
    pub path: std::path::PathBuf,
    pub offset: u64,
    pub length: usize,
}
