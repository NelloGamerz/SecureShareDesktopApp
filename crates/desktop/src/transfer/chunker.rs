use crate::models::{chunk::ChunkJob, file_info::FileInfo};
pub fn jobs(transfer_id: &str, files: &[FileInfo], chunk_size: usize) -> Vec<ChunkJob> {
    let mut jobs = Vec::new();
    for f in files {
        let total = ((f.size + chunk_size as u64 - 1) / chunk_size as u64).max(1);
        for index in 0..total {
            let offset = index * chunk_size as u64;
            jobs.push(ChunkJob {
                transfer_id: transfer_id.into(),
                file_id: f.id.clone(),
                index,
                total,
                relative_path: f.relative_path.clone(),
                path: f.path.clone(),
                offset,
                length: ((f.size - offset).min(chunk_size as u64)) as usize,
            })
        }
    }
    jobs
}
