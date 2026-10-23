//! Assembling a file from its chunks, and checking it before anyone can see it.
//!
//! The assembled bytes are digested as they are written, so the final integrity
//! check costs no second pass over the file. The result is renamed into place
//! only once it has been checked: a file that is not what the sender described
//! is removed, and the destination is left exactly as it was. That is the whole
//! point of assembling into a temporary — a transfer that fails verification
//! must not leave its output looking finished.

use crate::transfer::errors::{Result, TransferError};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::fs;

/// The buffer a merge reads and writes in.
///
/// The copy runs on the blocking pool, so this is not a per-call allocation
/// concern: one read and one write per buffer, with no async round trip in
/// between. 4 MiB is large enough that the per-call cost is invisible beside
/// the bytes moved, and small enough that a merge does not hold a distracting
/// amount of memory.
const COPY_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// Assemble a file from its parts, check it, and publish it.
///
/// `expected` is the hex SHA-256 the sender says the whole file hashes to, or
/// `None` when the sender described none — an older client that has no digests
/// to give. A transfer with no expectation is assembled exactly as it always
/// was; there is nothing to check it against, and inventing a check from the
/// receiver's own records would only prove the receiver consistent with itself.
///
/// Fails with [`TransferError::IntegrityMismatch`] when the assembled bytes do
/// not match, having removed the assembled file. Nothing is written to
/// `destination` in that case.
pub async fn merge(
    parts: &Path,
    destination: &Path,
    total: u64,
    expected: Option<&str>,
) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).await?;
    }

    let temporary = destination.with_extension("transfer-tmp");

    match assemble(&temporary, parts, total, expected).await {
        Ok(()) => {
            fs::rename(&temporary, destination).await?;
            Ok(())
        }

        Err(error) => {
            // Whatever is in the temporary is not a file anybody asked for.
            let _ = fs::remove_file(&temporary).await;

            Err(error)
        }
    }
}

async fn assemble(
    temporary: &Path,
    parts: &Path,
    total: u64,
    expected: Option<&str>,
) -> Result<()> {
    /*
     * The whole copy is one blocking task.
     *
     * The previous loop awaited a separate read and a separate write for every
     * 64 KiB, which on `tokio::fs` is two blocking-pool dispatches per buffer:
     * a 10 GiB file paid for roughly 320,000 round trips before it was even
     * written. Here one thread owns both files for the duration, reads and
     * writes in `COPY_BUFFER_BYTES` steps, and hashes inline — so the async
     * runtime is never blocked on the bytes and the per-call overhead is
     * amortised over megabytes instead of kilobytes.
     */
    let temporary_for_copy = temporary.to_path_buf();
    let parts_for_copy = parts.to_path_buf();
    let expected_for_copy = expected.map(str::to_owned);

    let actual = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        use std::io::{Read as _, Write as _};

        let mut output = std::fs::File::create(&temporary_for_copy)?;
        let mut hasher = expected_for_copy.as_ref().map(|_| Sha256::new());
        let mut buffer = vec![0u8; COPY_BUFFER_BYTES];

        for index in 0..total {
            let mut input = std::fs::File::open(parts_for_copy.join(format!("{index}.part")))?;

            loop {
                let read = input.read(&mut buffer)?;

                if read == 0 {
                    break;
                }

                if let Some(hasher) = hasher.as_mut() {
                    hasher.update(&buffer[..read]);
                }

                output.write_all(&buffer[..read])?;
            }
        }

        output.flush()?;
        output.sync_all()?;

        Ok(hasher
            .map(|hasher| hex::encode(hasher.finalize()))
            .unwrap_or_default())
    })
    .await
    .map_err(|error| TransferError::Invalid(format!("the file merge task failed: {error}")))?
    .map_err(TransferError::Io)?;

    if let Some(expected) = expected {
        if actual != expected {
            return Err(TransferError::IntegrityMismatch {
                path: temporary.to_string_lossy().into_owned(),
                expected: expected.to_string(),
                actual,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::manifest;
    use crate::transfer::manifest::test_support::TempDir;

    /// Lay out `chunks` as `.part` files under the parts directory.
    async fn parts(scratch: &TempDir, chunks: &[&[u8]]) -> std::path::PathBuf {
        let parts = scratch.path().join("f-1");
        fs::create_dir_all(&parts).await.expect("mkdir");

        for (index, bytes) in chunks.iter().enumerate() {
            manifest::write_part(&manifest::part_path(&parts, index as u64), bytes)
                .await
                .expect("writing a part");
        }

        parts
    }

    fn destination(scratch: &TempDir) -> std::path::PathBuf {
        scratch.path().join("out").join("a.bin")
    }

    #[tokio::test]
    async fn a_file_that_matches_its_digest_is_published() {
        let scratch = TempDir::new();
        let parts = parts(&scratch, &[b"abcd", b"efgh", b"ij"]).await;
        let destination = destination(&scratch);

        let digest = manifest::sha256_hex(b"abcdefghij");

        merge(&parts, &destination, 3, Some(&digest))
            .await
            .expect("a matching file must be published");

        assert_eq!(
            fs::read(&destination).await.expect("reading the result"),
            b"abcdefghij"
        );
        assert!(
            !destination.with_extension("transfer-tmp").exists(),
            "the temporary must not be left behind"
        );
    }

    #[tokio::test]
    async fn a_file_that_does_not_match_its_digest_is_never_published() {
        let scratch = TempDir::new();
        let parts = parts(&scratch, &[b"abcd", b"efgh"]).await;
        let destination = destination(&scratch);

        let error = merge(
            &parts,
            &destination,
            2,
            Some(&manifest::sha256_hex(b"something else")),
        )
        .await
        .expect_err("a file that does not match must not be published");

        assert!(matches!(error, TransferError::IntegrityMismatch { .. }));

        assert!(
            !destination.exists(),
            "the destination must be left as it was"
        );
        assert!(
            !destination.with_extension("transfer-tmp").exists(),
            "and the assembled bytes taken back out of the way"
        );
    }

    #[tokio::test]
    async fn a_transfer_with_no_expectation_is_assembled_as_before() {
        let scratch = TempDir::new();
        let parts = parts(&scratch, &[b"abcd", b"efgh"]).await;
        let destination = destination(&scratch);

        merge(&parts, &destination, 2, None)
            .await
            .expect("an old sender gives nothing to check against");

        assert_eq!(
            fs::read(&destination).await.expect("reading the result"),
            b"abcdefgh"
        );
    }

    #[tokio::test]
    async fn a_missing_chunk_cannot_be_assembled() {
        let scratch = TempDir::new();
        let parts = parts(&scratch, &[b"abcd", b"efgh"]).await;
        let destination = destination(&scratch);

        fs::remove_file(manifest::part_path(&parts, 1))
            .await
            .expect("removing a part");

        assert!(
            merge(
                &parts,
                &destination,
                2,
                Some(&manifest::sha256_hex(b"abcdefgh"))
            )
            .await
            .is_err(),
            "a short file must not appear at the destination"
        );

        assert!(!destination.exists());
        assert!(!destination.with_extension("transfer-tmp").exists());
    }
}
