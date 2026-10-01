//! What a transfer's files are supposed to be, and the check that a
//! reconstructed file really is.
//!
//! Per-chunk hashing (see [`crate::transfer::manifest`]) proves that the bytes
//! the receiver holds are the bytes it was sent. It cannot prove that the bytes
//! it was sent are the bytes the sender meant to send: the receiver records each
//! chunk under the digest of what arrived, so a chunk corrupted in flight — or a
//! manifest damaged after the fact — verifies against itself and passes.
//!
//! What closes that gap is an expectation the receiver did not write. The
//! sender digests its own files before the first chunk moves, and sends the
//! result with the transfer's metadata. The receiver checks the assembled file
//! against it before publishing anything, and a file that does not match is not
//! reported as delivered — see [`crate::transfer::writer::assemble_and_publish`].
//!
//! Two digests are kept per file, and they do different jobs:
//!
//! * the whole-file digest is the final check, and the only one that can catch a
//!   file assembled from chunks that each verified on their own;
//! * the per-chunk digests say *which* chunks are wrong once the whole-file
//!   check has failed, so the sender can be asked for those and not for the
//!   whole file.
//!
//! Both are computed in one pass over each source file. The sender reads its
//! files once more than it otherwise would, before the first chunk goes out; the
//! alternative is a transfer that cannot tell a delivered file from a plausible
//! one.

use crate::models::file_info::FileInfo;
use crate::transfer::errors::{Result, TransferError};
use crate::transfer::manifest::sha256_hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt as _},
};

/// One chunk, as the sender intends to send it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedChunk {
    /// Its index within the file.
    pub index: u64,
    /// How many bytes it holds.
    pub length: u64,
    /// Hex SHA-256 of those bytes.
    pub sha256: String,
}

/// One file, as the sender intends to send it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedFile {
    /// The file's id, which is what a chunk's `File-Id` header names.
    pub file_id: String,
    /// The path the sender will ask the receiver to write it to.
    pub relative_path: String,
    /// The size the transfer will carry for it.
    pub size: u64,
    /// Hex SHA-256 of the whole file, in the order its chunks are sent.
    pub sha256: String,
    /// The digests of the chunks that make it up, by index.
    ///
    /// Absent from a sender that only describes whole files; the receiver then
    /// knows a file is wrong but not which chunk to ask for.
    #[serde(default)]
    pub chunks: Vec<ExpectedChunk>,
}

impl ExpectedFile {
    /// The expectation for one chunk of this file, if the sender described it.
    pub fn chunk(&self, index: u64) -> Option<&ExpectedChunk> {
        self.chunks.iter().find(|chunk| chunk.index == index)
    }
}

/// The digests a whole transfer is checked against.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedTransfer {
    /// The transfer these digests belong to, so a manifest that arrives for the
    /// wrong one can be noticed rather than acted on.
    pub transfer_id: String,
    /// One entry per file being sent.
    #[serde(default)]
    pub files: Vec<ExpectedFile>,
}

impl ExpectedTransfer {
    /// The expectation for one file of this transfer.
    pub fn file(&self, file_id: &str) -> Option<&ExpectedFile> {
        self.files.iter().find(|file| file.file_id == file_id)
    }
}

/// Read every file once, digesting it whole and chunk by chunk.
///
/// The chunk boundaries are [`crate::transfer::chunker::jobs`]'s, so the digests
/// describe exactly the bytes that will be sent — an empty file is one empty
/// chunk, and the last chunk of a file is its tail. Only the first `file.size`
/// bytes are read, which is all the transfer will carry.
///
/// Fails rather than guesses when a file is shorter than the transfer says it
/// is: a plan built from bytes that are not there would describe a file the
/// sender cannot send.
pub async fn plan(
    transfer_id: &str,
    files: &[FileInfo],
    chunk_size: usize,
) -> Result<ExpectedTransfer> {
    let chunk_size = chunk_size.max(1) as u64;
    let mut expected = Vec::with_capacity(files.len());

    for file in files {
        expected.push(plan_file(transfer_id, file, chunk_size).await?);
    }

    Ok(ExpectedTransfer {
        transfer_id: transfer_id.to_string(),
        files: expected,
    })
}

async fn plan_file(transfer_id: &str, file: &FileInfo, chunk_size: u64) -> Result<ExpectedFile> {
    let mut source = fs::File::open(&file.path).await.map_err(|error| {
        TransferError::Invalid(format!(
            "cannot read {} to work out what transfer {transfer_id} will send: {error}",
            file.path.display()
        ))
    })?;

    let mut whole = Sha256::new();
    let mut chunks = Vec::new();

    for index in 0..file.size.div_ceil(chunk_size).max(1) {
        let length = (file.size - index * chunk_size).min(chunk_size) as usize;
        let mut bytes = vec![0u8; length];

        source.read_exact(&mut bytes).await.map_err(|error| {
            TransferError::Invalid(format!(
                "{} is shorter than the transfer says it is: {error}",
                file.path.display()
            ))
        })?;

        whole.update(&bytes);

        chunks.push(ExpectedChunk {
            index,
            length: length as u64,
            sha256: sha256_hex(&bytes),
        });
    }

    Ok(ExpectedFile {
        file_id: file.id.clone(),
        relative_path: file.relative_path.clone(),
        size: file.size,
        sha256: hex::encode(whole.finalize()),
        chunks,
    })
}

/// A writer that digests what it is given while writing it out.
///
/// Used by the merger, so that the file it assembles is checked as it is built
/// rather than read back a second time. The digest is only maintained when there
/// is an expectation to compare it against.
pub struct DigestingWriter<'a> {
    output: &'a mut fs::File,
    hasher: Option<Sha256>,
}

impl<'a> DigestingWriter<'a> {
    /// Wrap `output`, digesting everything written through it if `expected` is
    /// `Some`.
    pub fn new(output: &'a mut fs::File, expected: Option<&str>) -> Self {
        Self {
            output,
            hasher: expected.map(|_| Sha256::new()),
        }
    }

    /// Write `bytes`, digesting them as they go.
    pub async fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(hasher) = self.hasher.as_mut() {
            hasher.update(bytes);
        }

        self.output.write_all(bytes).await
    }

    /// The digest of everything written so far, or `None` if this writer was not
    /// asked to keep one.
    pub fn digest(&mut self) -> Option<String> {
        self.hasher
            .take()
            .map(|hasher| hex::encode(hasher.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::chunker;
    use crate::transfer::manifest::test_support::TempDir;

    /// A file of `size` bytes with a reproducible pattern.
    async fn source(scratch: &TempDir, name: &str, size: usize) -> FileInfo {
        let path = scratch.path().join(name);
        let bytes: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();

        fs::write(&path, &bytes).await.expect("writing the source");

        FileInfo {
            id: format!("f-{name}"),
            path,
            relative_path: name.to_string(),
            size: size as u64,
        }
    }

    #[tokio::test]
    async fn a_plan_describes_the_bytes_the_chunker_will_send() {
        let scratch = TempDir::new();
        let file = source(&scratch, "a.bin", 10).await;

        let expected = plan("t-1", std::slice::from_ref(&file), 4)
            .await
            .expect("planning");

        let planned = &expected.files[0];

        assert_eq!(planned.file_id, "f-a.bin");
        assert_eq!(planned.relative_path, "a.bin");
        assert_eq!(planned.size, 10);

        let jobs = chunker::jobs("t-1", std::slice::from_ref(&file), 4);

        assert_eq!(planned.chunks.len(), jobs.len());

        for (chunk, job) in planned.chunks.iter().zip(&jobs) {
            assert_eq!(chunk.index, job.index);
            assert_eq!(chunk.length, job.length as u64);
            assert_eq!(
                chunk.sha256,
                crate::transfer::resume::chunk_digest(job)
                    .await
                    .expect("digesting the job"),
                "the plan must describe the chunk the uploader will read"
            );
        }
    }

    #[tokio::test]
    async fn the_whole_file_digest_is_the_file_s() {
        let scratch = TempDir::new();
        let file = source(&scratch, "a.bin", 10).await;

        let expected = plan("t-1", std::slice::from_ref(&file), 4)
            .await
            .expect("planning");

        let bytes = fs::read(&file.path).await.expect("reading the source");

        assert_eq!(
            expected.files[0].sha256,
            sha256_hex(&bytes),
            "the final check has to be against the file the sender holds"
        );
    }

    #[tokio::test]
    async fn an_empty_file_is_one_empty_chunk() {
        let scratch = TempDir::new();
        let file = source(&scratch, "empty.bin", 0).await;

        let expected = plan("t-1", std::slice::from_ref(&file), 4)
            .await
            .expect("planning");

        let planned = &expected.files[0];

        assert_eq!(planned.chunks.len(), 1, "the chunker sends one chunk");
        assert_eq!(planned.chunks[0].length, 0);
        assert_eq!(planned.chunks[0].sha256, sha256_hex(b""));
        assert_eq!(planned.sha256, sha256_hex(b""));
    }

    #[tokio::test]
    async fn planning_a_file_that_is_shorter_than_it_claims_fails() {
        let scratch = TempDir::new();
        let mut file = source(&scratch, "short.bin", 4).await;

        // The scan said ten bytes; the file has four.
        file.size = 10;

        assert!(
            plan("t-1", std::slice::from_ref(&file), 4).await.is_err(),
            "a plan must not describe bytes the sender does not have"
        );
    }
}
