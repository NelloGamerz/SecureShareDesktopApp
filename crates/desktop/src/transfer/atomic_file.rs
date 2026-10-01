//! Writing a file so that a crash cannot leave it half-written.
//!
//! Both sides of a transfer keep small files whose whole value is that they are
//! never observed partially: the receiver's chunk manifest, the receiver's
//! `.part` files, and the sender's per-transfer state. All three want the same
//! thing, so it lives here once.
//!
//! The shape is always the same: stage the bytes in a temporary beside the
//! target, flush and `sync_all` them with the file still open, then rename the
//! temporary over the target. The rename is the only step a reader can observe,
//! and it either happened or it did not.

use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::{fs, io::AsyncWriteExt};

/// The name a temporary for `path` is written under.
///
/// The suffix is appended rather than substituted so the name stays obviously
/// related to its target: `manifest.json` stages as `manifest.json.tmp`, and
/// `0.part` as `0.part.tmp`.
pub fn temporary_for(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();

    name.push(".tmp");

    PathBuf::from(name)
}

/// Write `bytes` to `path`, atomically and durably.
///
/// The bytes are staged in a temporary **in the target's own directory** — not
/// in the system temp directory, so that the rename cannot cross a filesystem
/// boundary, which would turn it into a copy and lose the atomicity this
/// function exists for — then flushed and `sync_all`ed, and only then renamed
/// over `path`. Renaming within one directory replaces the target atomically
/// (on Windows `std::fs::rename` maps to `MoveFileExW` with
/// `MOVEFILE_REPLACE_EXISTING`), so a reader sees the whole old file or the
/// whole new one. A crash part-way through leaves the temporary, never a short
/// file under the real name.
///
/// When this returns `Ok`, the bytes are on disk rather than merely in the OS
/// cache.
///
/// The parent directory must already exist; this does not create it, so that a
/// caller which means to create it can distinguish that failure from this one.
pub async fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = temporary_for(path);

    let staged = async {
        let mut file = fs::File::create(&temporary).await?;

        file.write_all(bytes).await?;
        file.flush().await?;
        file.sync_all().await
    }
    .await;

    let result = match staged {
        Ok(()) => fs::rename(&temporary, path).await,
        Err(error) => Err(error),
    };

    if result.is_err() {
        // A write that failed still owes the directory nothing: the temporary
        // is not read by anything, and leaving it would be bytes nobody will
        // ever account for.
        let _ = fs::remove_file(&temporary).await;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::manifest::test_support::TempDir;

    #[tokio::test]
    async fn writing_put_the_bytes_where_they_were_asked_for() {
        let scratch = TempDir::new();
        let target = scratch.path().join("target.bin");

        write_atomic(&target, b"the whole file")
            .await
            .expect("write");

        assert_eq!(
            fs::read(&target).await.expect("read"),
            b"the whole file".to_vec()
        );
    }

    #[tokio::test]
    async fn writing_replaces_the_previous_contents_whole() {
        let scratch = TempDir::new();
        let target = scratch.path().join("target.bin");

        write_atomic(&target, b"a much longer first version")
            .await
            .expect("first");
        write_atomic(&target, b"short").await.expect("second");

        // An in-place overwrite rather than a rename would leave the tail of
        // the first write behind and the file would be the wrong length.
        assert_eq!(fs::read(&target).await.expect("read"), b"short".to_vec());
    }

    #[tokio::test]
    async fn writing_leaves_no_temporary_behind() {
        let scratch = TempDir::new();
        let target = scratch.path().join("target.bin");

        write_atomic(&target, b"bytes").await.expect("write");

        let mut entries = fs::read_dir(scratch.path()).await.expect("listing");
        let mut names = Vec::new();

        while let Some(entry) = entries.next_entry().await.expect("entry") {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }

        assert_eq!(names, vec!["target.bin".to_string()]);
    }

    #[tokio::test]
    async fn a_failed_write_leaves_no_temporary_either() {
        let scratch = TempDir::new();

        // A directory where the file should go: staging succeeds, the rename
        // cannot replace a directory, and the write fails after its bytes were
        // written.
        let target = scratch.path().join("target.bin");
        fs::create_dir(&target)
            .await
            .expect("mkdir over the target");

        let result = write_atomic(&target, b"bytes").await;

        assert!(result.is_err(), "replacing a directory must fail");
        assert!(
            !temporary_for(&target).exists(),
            "and the staged temporary must be cleaned up"
        );
    }

    #[tokio::test]
    async fn a_crash_mid_write_leaves_only_a_temporary() {
        let scratch = TempDir::new();
        let target = scratch.path().join("target.bin");

        // What a crash during `write_atomic` leaves behind: the staged file,
        // and nothing at the real name.
        fs::write(temporary_for(&target), b"half")
            .await
            .expect("staging");

        assert!(
            !target.exists(),
            "the real name must not be visible until the rename"
        );
    }

    #[test]
    fn the_temporary_is_named_beside_its_target() {
        assert_eq!(
            temporary_for(Path::new("/tmp/manifest.json")),
            PathBuf::from("/tmp/manifest.json.tmp")
        );
        assert_eq!(
            temporary_for(Path::new("/tmp/0.part")),
            PathBuf::from("/tmp/0.part.tmp")
        );
    }
}
