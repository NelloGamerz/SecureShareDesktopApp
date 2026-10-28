use crate::{
    models::{chunk::ChunkJob, file_info::FileInfo},
    transfer::constants::{MAX_PLANNED_CHUNK_SIZE, MIN_CHUNK_SIZE},
};

/// The chunk size to plan a transfer of `total_bytes` with.
///
/// Small transfers use 256 KiB so the window fills and a stall costs little;
/// past 16 MiB the size doubles to 512 KiB so per-request cost does not
/// dominate. Nothing larger is ever planned.
pub fn chunk_size_for(total_bytes: u64) -> usize {
    const MIB: u64 = 1024 * 1024;

    if total_bytes <= 16 * MIB {
        MIN_CHUNK_SIZE
    } else {
        MAX_PLANNED_CHUNK_SIZE
    }
}

pub fn jobs(transfer_id: &str, files: &[FileInfo], chunk_size: usize) -> Vec<ChunkJob> {
    let mut jobs = Vec::new();
    for f in files {
        let total = f.size.div_ceil(chunk_size as u64).max(1);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chunk_size_grows_with_the_transfer_and_never_exceeds_the_cap() {
        const MIB: u64 = 1024 * 1024;

        assert_eq!(chunk_size_for(0), MIN_CHUNK_SIZE);
        assert_eq!(chunk_size_for(16 * MIB), MIN_CHUNK_SIZE);
        assert_eq!(chunk_size_for(16 * MIB + 1), MAX_PLANNED_CHUNK_SIZE);
        assert_eq!(chunk_size_for(1024 * MIB), MAX_PLANNED_CHUNK_SIZE);

        for mib in [1, 16, 17, 64, 65, 1024, 100_000] {
            let size = chunk_size_for(mib * MIB);

            assert!(
                (MIN_CHUNK_SIZE..=MAX_PLANNED_CHUNK_SIZE).contains(&size),
                "{mib} MiB planned a {size}-byte chunk"
            );
        }
    }
}
