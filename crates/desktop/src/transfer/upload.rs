use crate::{
    models::chunk::ChunkJob,
    transfer::{
        errors::{Result, TransferError},
        retry,
        session::ProtocolSession,
    },
};

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// One chunk's measured phases, summed over its attempts.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChunkTimings {
    /// Compressing the plaintext, on the blocking pool.
    pub compress: std::time::Duration,
    /// Plaintext bytes the successful attempt carried.
    pub plaintext_bytes: u64,
    /// Body bytes the successful attempt put on the wire.
    pub body_bytes: u64,
    /// Reading the bytes from the source file, on the blocking pool.
    pub read: std::time::Duration,
    /// Sealing them, as the session measured it.
    pub encrypt: std::time::Duration,
    /// The part of `encrypt` spent in the cipher rather than queueing for a
    /// blocking thread.
    pub encrypt_cpu: std::time::Duration,
    /// The request and the wait for the receiver's acknowledgement.
    pub http: std::time::Duration,
}

/// How many source files are kept open across one run.
///
/// A transfer may hold thousands of files; keeping every one open would exhaust
/// the process's handles. Past this many, a file is opened per chunk as it was
/// before, which is slower but bounded.
const MAX_OPEN_FILES: usize = 128;

/// Source files opened once per run instead of once per chunk.
///
/// A chunk read is a seek and a read; the open that preceded them is repeated
/// for every chunk and every retry of a transfer that is otherwise a stream of
/// sequential reads from a handful of files.
#[derive(Default)]
pub struct FileHandles {
    files: Mutex<HashMap<PathBuf, Arc<std::fs::File>>>,
}

impl FileHandles {
    /// The handle for `path`, opening it on first use.
    async fn handle(&self, path: &Path) -> Result<Arc<std::fs::File>> {
        if let Some(file) = self
            .files
            .lock()
            .expect("file handle lock poisoned")
            .get(path)
        {
            return Ok(Arc::clone(file));
        }

        let opened = path.to_path_buf();

        let file = tokio::task::spawn_blocking(move || std::fs::File::open(&opened))
            .await
            .map_err(|error| {
                TransferError::Invalid(format!("the chunk open task failed: {error}"))
            })??;

        let file = Arc::new(file);
        let mut files = self.files.lock().expect("file handle lock poisoned");

        if files.len() < MAX_OPEN_FILES {
            files.insert(path.to_path_buf(), Arc::clone(&file));
        }

        Ok(file)
    }
}

/// Read `length` bytes at `offset` from a shared handle.
///
/// A **positional** read, not a seek. `try_clone` does not give a clone its own
/// cursor — on Unix `dup` and on Windows `DuplicateHandle` share the file
/// position with the original — so a seek and a read on two "clones" of the
/// same handle race, and concurrent chunks read from each other's offsets. That
/// is a short read at best ("failed to fill whole buffer") and the wrong bytes
/// for a chunk at worst. `pread`/`seek_read` take the offset as an argument and
/// leave the shared cursor alone, which is what makes one handle safe to read
/// from many tasks.
#[cfg(unix)]
fn read_at(file: &std::fs::File, offset: u64, length: usize) -> std::io::Result<Vec<u8>> {
    use std::os::unix::fs::FileExt;

    let mut bytes = vec![0u8; length];

    file.read_exact_at(&mut bytes, offset)?;

    Ok(bytes)
}

#[cfg(windows)]
fn read_at(file: &std::fs::File, offset: u64, length: usize) -> std::io::Result<Vec<u8>> {
    use std::os::windows::fs::FileExt;

    let mut bytes = vec![0u8; length];
    let mut filled = 0usize;

    while filled < length {
        let read = file.seek_read(&mut bytes[filled..], offset + filled as u64)?;

        if read == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }

        filled += read;
    }

    Ok(bytes)
}

/// Platform without positional reads: a seek under a lock this process owns,
/// so two chunks cannot interleave their offsets. Correct, just serialised.
#[cfg(not(any(unix, windows)))]
fn read_at(file: &std::fs::File, offset: u64, length: usize) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};

    let mut handle = file.try_clone()?;

    handle.seek(SeekFrom::Start(offset))?;

    let mut bytes = vec![0u8; length];

    handle.read_exact(&mut bytes)?;

    Ok(bytes)
}

/// Read, encrypt and send one chunk, retrying the whole attempt on failure.
///
/// Takes a [`ProtocolSession`] rather than an [`crate::transfer::http_client::HttpClient`]
/// plus an endpoint plus a token, so that which wire carries the chunk is the
/// session's business and not this function's. The retry loop and the read are
/// unchanged — only *where the bytes go*, and *how they are sealed*, moved
/// behind the seam.
///
/// The read itself runs on the blocking pool: a chunk is a file syscall and a
/// full-chunk allocation, neither of which belongs on the shared runtime.
pub async fn upload_chunk(
    session: &dyn ProtocolSession,
    job: &ChunkJob,
    max_retries: u32,
    files: &FileHandles,
    first: Option<std::time::Instant>,
) -> Result<(u32, ChunkTimings)> {
    let mut attempt = 0;

    /*
     * Summed over every attempt, so a chunk that needed retries shows the real
     * cost of getting it accepted rather than the cost of the attempt that
     * finally worked.
     */
    let mut timings = ChunkTimings::default();

    loop {
        let result = async {
            let open_started = std::time::Instant::now();
            let file = files.handle(&job.path).await?;
            let open = open_started.elapsed();

            let offset = job.offset;
            let length = job.length;

            let read_started = std::time::Instant::now();

            let bytes = tokio::task::spawn_blocking(move || read_at(&file, offset, length))
                .await
                .map_err(|error| {
                    TransferError::Invalid(format!("the chunk read task failed: {error}"))
                })??;

            let read = read_started.elapsed();

            timings.read += read;

            if let Some(origin) = first {
                tracing::info!(
                    transfer_id = %job.transfer_id,
                    chunk = job.index,
                    open_ms = open.as_millis() as u64,
                    read_ms = read.as_millis() as u64,
                    startup_ms = origin.elapsed().as_millis() as u64,
                    "startup: first chunk opened and read"
                );
            }

            /*
             * Plaintext, not ciphertext.
             *
             * The session seals it, because the AEAD's associated data is a
             * property of the protocol — v2 binds the transfer, file, index,
             * path and protocol version into the tag, and v1 passes none — and
             * because the two versions derive their key differently. Sealing
             * here would mean the engine had to know which protocol it was on.
             */
            if let Some(origin) = first {
                tracing::info!(
                    transfer_id = %job.transfer_id,
                    chunk = job.index,
                    startup_ms = origin.elapsed().as_millis() as u64,
                    "startup: first chunk sealing and chunk request initiated"
                );
            }

            let sent = session.send_chunk(job, bytes).await?;

            timings.compress += sent.compress;
            timings.plaintext_bytes = sent.plaintext_bytes;
            timings.body_bytes = sent.body_bytes;
            timings.encrypt += sent.encrypt;
            timings.encrypt_cpu += sent.encrypt_cpu;
            timings.http += sent.http;

            if let Some(origin) = first {
                tracing::info!(
                    transfer_id = %job.transfer_id,
                    chunk = job.index,
                    encrypt_ms = sent.encrypt.as_millis() as u64,
                    http_ms = sent.http.as_millis() as u64,
                    startup_ms = origin.elapsed().as_millis() as u64,
                    "startup: first chunk acknowledged"
                );
            }

            Ok::<(), TransferError>(())
        }
        .await;

        match result {
            Ok(()) => {
                return Ok((attempt, timings));
            }

            /*
             * Not a transient failure: the receiver named chunks it no longer
             * holds. Retrying this same chunk would be refused again, so the
             * error goes back to the scheduler to requeue exactly those.
             */
            Err(error @ TransferError::ChunksMissing { .. }) => {
                return Err(error);
            }

            Err(error) if attempt < max_retries => {
                attempt += 1;

                tokio::time::sleep(retry::delay(attempt - 1)).await;

                tracing::warn!(
                    transfer_id=%job.transfer_id,
                    chunk=job.index,
                    attempt,
                    error=%error,
                    "retrying chunk upload"
                );
            }

            Err(error) => {
                return Err(error);
            }
        }
    }
}
