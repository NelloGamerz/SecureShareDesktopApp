//! Assembling a file from its chunks, and checking it before anyone can see it.
//!
//! The assembled bytes are digested as they are written, so the final integrity
//! check costs no second pass over the file. The result is renamed into place
//! only once it has been checked: a file that is not what the sender described
//! is removed, and the destination is left exactly as it was. That is the whole
//! point of assembling into a temporary — a transfer that fails verification
//! must not leave its output looking finished.

use crate::transfer::errors::{Result, TransferError};
use crate::transfer::integrity::DigestingWriter;
use std::path::Path;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};

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
    let mut output = fs::File::create(temporary).await?;

    let actual = {
        let mut writer = DigestingWriter::new(&mut output, expected);

        for index in 0..total {
            let mut input = fs::File::open(parts.join(format!("{index}.part"))).await?;
            let mut buffer = vec![0; 64 * 1024];

            loop {
                let read = input.read(&mut buffer).await?;

                if read == 0 {
                    break;
                }

                writer.write(&buffer[..read]).await?;
            }
        }

        writer.digest()
    };

    output.flush().await?;
    output.sync_all().await?;

    if let Some(expected) = expected {
        let actual = actual.unwrap_or_default();

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
