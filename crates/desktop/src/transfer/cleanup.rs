//! Removing what a transfer leaves behind on the receiver.
//!
//! A transfer that arrives in pieces leaves pieces behind: a `.part` per chunk,
//! a `manifest.json` recording them, and a directory holding both. They are
//! deliberate — that is what makes a transfer resumable — but they are only
//! worth keeping while somebody might come back for them. Two things end that:
//!
//! * **A cancellation.** The sender says it is done with the transfer, and the
//!   receiver drops everything it holds for it. See
//!   [`crate::transfer::writer::cancel_transfer`].
//! * **A timeout.** Nobody said anything, and nothing has touched the directory
//!   for longer than [`DEFAULT_TTL`]. Every partial transfer is then disk nobody
//!   is coming back for.
//!
//! The rule both share is that **a live transfer is never touched**, however
//! old its directory looks. Age is what an abandoned transfer and a slow one
//! have in common, and only the receiver knows which is which — so it passes the
//! set of transfers it has a live session for, and this module leaves those
//! alone whatever the clock says.
//!
//! Everything here works on paths, and nothing here needs a running app; the
//! receiver's side of the wiring lives in [`crate::transfer::writer`].

use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tokio::fs;

/// How long a transfer's directory is kept after the last time it was touched.
///
/// A day is long enough that a transfer interrupted overnight can still be
/// resumed in the morning, and short enough that a transfer nobody is coming
/// back for does not hold a gigabyte of `.part` files indefinitely.
pub const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// The environment variable that overrides [`DEFAULT_TTL`], in seconds.
///
/// Read once at startup, in the style of the shell's other knobs — see
/// `utils::config`. An unparseable value is logged and ignored rather than
/// taken as zero, because "delete every transfer older than nothing" is a very
/// expensive way to misread a configuration file.
pub const TTL_ENV: &str = "VILSEND_TRANSFER_TTL_SECS";

/// How often the sweep looks for transfers to remove.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// The TTL to sweep with, from the environment if it says anything.
pub fn ttl_from_env() -> Duration {
    let Ok(value) = std::env::var(TTL_ENV) else {
        return DEFAULT_TTL;
    };

    match value.trim().parse::<u64>() {
        Ok(seconds) => Duration::from_secs(seconds),

        Err(error) => {
            tracing::warn!(
                variable = TTL_ENV,
                value = %value,
                %error,
                "ignoring an unreadable transfer TTL; using the default"
            );

            DEFAULT_TTL
        }
    }
}

/// The directory a transfer's data lives in under `root`.
///
/// Refuses an id that could name anything outside `root`. The id arrives in a
/// request header, and this is the module that deletes things, so it is the one
/// place that cannot afford to trust it: `../../` in a transfer id would
/// otherwise be a request to remove a directory of the caller's choosing.
pub fn transfer_dir(root: &Path, transfer_id: &str) -> io::Result<PathBuf> {
    if transfer_id.is_empty()
        || transfer_id.contains('/')
        || transfer_id.contains('\\')
        || transfer_id.contains("..")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("transfer id cannot name a directory: {transfer_id:?}"),
        ));
    }

    Ok(root.join("incoming").join(transfer_id))
}

/// Remove everything one transfer holds: its parts, its manifest, its directory.
///
/// Returns whether there was anything to remove. A transfer whose directory is
/// already gone is not an error — the caller's intent is "this transfer should
/// not be here", and it is not — so cancelling twice, or cancelling a transfer
/// the sweep has already taken, behaves the same as cancelling once.
pub async fn remove_transfer(root: &Path, transfer_id: &str) -> io::Result<bool> {
    let dir = transfer_dir(root, transfer_id)?;

    match fs::remove_dir_all(&dir).await {
        Ok(()) => Ok(true),

        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),

        // A part of the tree that cannot be removed — a file held open, or a
        // permission that changed underneath us — is reported rather than
        // retried here. The sweep will come back to it.
        Err(error) => Err(error),
    }
}

/// The transfers under `root` that have been left alone for longer than `ttl`.
///
/// `active` is the set of transfers the receiver has a live session for; they
/// are never returned, however long they have been going. `now` is passed in
/// rather than read so that what counts as "old" can be tested without waiting.
///
/// A directory that cannot be read or stat'd is skipped rather than reported:
/// there is nothing the caller could do about it, and the next sweep will look
/// again.
pub async fn abandoned(
    root: &Path,
    ttl: Duration,
    active: &HashSet<String>,
    now: SystemTime,
) -> Vec<String> {
    let incoming = root.join("incoming");

    let mut entries = match fs::read_dir(&incoming).await {
        Ok(entries) => entries,

        // Nothing has ever been received, which is not a problem to report.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),

        Err(error) => {
            tracing::warn!(path = %incoming.display(), %error, "could not read the transfer directory");

            return Vec::new();
        }
    };

    let mut abandoned = Vec::new();

    loop {
        match entries.next_entry().await {
            Ok(Some(entry)) => {
                let id = entry.file_name().to_string_lossy().into_owned();

                if active.contains(&id) {
                    continue;
                }

                let Ok(metadata) = entry.metadata().await else {
                    continue;
                };

                if !metadata.is_dir() {
                    continue;
                }

                let Ok(modified) = metadata.modified() else {
                    continue;
                };

                // A clock that has gone backwards, or an mtime in the future,
                // reads as brand new. Sweeping something that might still be in
                // use is the failure that matters here; keeping it a day longer
                // is not.
                let age = now.duration_since(modified).unwrap_or_default();

                if age >= ttl {
                    abandoned.push(id);
                }
            }

            Ok(None) => break,

            Err(error) => {
                tracing::warn!(path = %incoming.display(), %error, "could not read an entry of the transfer directory");

                break;
            }
        }
    }

    // Sorted so that a sweep removes, logs and reports the same order every
    // time, which is what makes a failure reproducible.
    abandoned.sort();

    abandoned
}

/// Remove every transfer under `root` that has been abandoned.
///
/// Returns the ids that were removed, in order. A transfer that cannot be
/// removed is logged and left for the next sweep: a cleanup that gave up on the
/// first locked file, or that brought the receiver down with it, would leave
/// more behind than it removed.
pub async fn sweep(
    root: &Path,
    ttl: Duration,
    active: &HashSet<String>,
    now: SystemTime,
) -> Vec<String> {
    let mut removed = Vec::new();

    for id in abandoned(root, ttl, active, now).await {
        match remove_transfer(root, &id).await {
            Ok(true) => {
                tracing::info!(transfer_id = %id, "removed an abandoned transfer");

                removed.push(id);
            }

            Ok(false) => {}

            Err(error) => {
                tracing::warn!(
                    transfer_id = %id,
                    %error,
                    "could not remove an abandoned transfer; leaving it for the next sweep"
                );
            }
        }
    }

    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::manifest::test_support::TempDir;

    /// A transfer directory holding a manifest and two chunk files, as a
    /// transfer in progress would leave it.
    async fn received(scratch: &TempDir, transfer_id: &str) -> PathBuf {
        let dir = transfer_dir(scratch.path(), transfer_id).expect("a usable id");
        let parts = dir.join("f-1");

        fs::create_dir_all(&parts).await.expect("creating parts");
        fs::write(dir.join("manifest.json"), b"{\"chunks\":[]}")
            .await
            .expect("writing a manifest");
        fs::write(parts.join("0.part"), b"abcd")
            .await
            .expect("writing a part");
        fs::write(parts.join("1.part"), b"efgh")
            .await
            .expect("writing a part");

        dir
    }

    fn no_active() -> HashSet<String> {
        HashSet::new()
    }

    /// A moment far enough in the future that anything on disk is older than
    /// `ttl`.
    fn later(ttl: Duration) -> SystemTime {
        SystemTime::now() + ttl + Duration::from_secs(1)
    }

    /*
     * ----------------------------------------------------------------------
     * Cancelling removes everything a transfer holds
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn cancelling_removes_the_manifest_and_every_part() {
        let scratch = TempDir::new();
        let dir = received(&scratch, "t-1").await;

        assert!(
            remove_transfer(scratch.path(), "t-1")
                .await
                .expect("removing"),
            "there was something to remove"
        );

        assert!(
            !dir.exists(),
            "the whole directory goes, manifest and parts with it"
        );
    }

    #[tokio::test]
    async fn cancelling_a_transfer_that_holds_nothing_is_not_an_error() {
        let scratch = TempDir::new();

        assert!(
            !remove_transfer(scratch.path(), "t-nobody")
                .await
                .expect("a transfer that is already gone is not a failure"),
            "and it reports that there was nothing to do"
        );
    }

    #[tokio::test]
    async fn cancelling_twice_is_the_same_as_cancelling_once() {
        let scratch = TempDir::new();
        received(&scratch, "t-1").await;

        assert!(remove_transfer(scratch.path(), "t-1").await.expect("first"));
        assert!(
            !remove_transfer(scratch.path(), "t-1")
                .await
                .expect("second"),
            "the second leaves nothing to remove"
        );
    }

    #[tokio::test]
    async fn a_transfer_id_that_would_escape_the_root_is_refused() {
        let scratch = TempDir::new();

        let outside = scratch.path().join("keep-me");
        fs::create_dir_all(&outside)
            .await
            .expect("creating a neighbour");

        for id in ["../keep-me", "..", "a/b", "a\\b", ""] {
            assert!(
                remove_transfer(scratch.path(), id).await.is_err(),
                "{id:?} must not name a directory"
            );
        }

        assert!(outside.exists(), "and nothing outside the root is touched");
    }

    /*
     * ----------------------------------------------------------------------
     * The TTL sweep
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_transfer_left_alone_past_the_ttl_is_removed() {
        let scratch = TempDir::new();
        let ttl = Duration::from_secs(60 * 60);

        let dir = received(&scratch, "t-old").await;

        let removed = sweep(scratch.path(), ttl, &no_active(), later(ttl)).await;

        assert_eq!(removed, vec!["t-old"]);
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn a_transfer_touched_inside_the_ttl_is_kept() {
        let scratch = TempDir::new();
        let ttl = Duration::from_secs(60 * 60);

        let dir = received(&scratch, "t-fresh").await;

        assert!(
            sweep(scratch.path(), ttl, &no_active(), SystemTime::now())
                .await
                .is_empty(),
            "a transfer inside its TTL is not abandoned"
        );
        assert!(dir.exists());
    }

    #[tokio::test]
    async fn a_transfer_with_a_live_session_is_never_swept() {
        let scratch = TempDir::new();
        let ttl = Duration::from_secs(60 * 60);

        /*
         * The directory has not been written to for longer than the TTL — a
         * transfer paused overnight, or one whose remaining chunks are large —
         * and it is still being worked on. Only the receiver knows that, which
         * is why it says so.
         */
        let dir = received(&scratch, "t-running").await;
        let active: HashSet<String> = ["t-running".to_string()].into_iter().collect();

        let removed = sweep(scratch.path(), ttl, &active, later(ttl)).await;

        assert!(removed.is_empty(), "an active transfer is not abandoned");
        assert!(dir.exists());
    }

    #[tokio::test]
    async fn the_sweep_leaves_what_it_does_not_take() {
        let scratch = TempDir::new();
        let ttl = Duration::from_secs(60 * 60);

        let old = received(&scratch, "t-old").await;
        let active = received(&scratch, "t-active").await;
        let running: HashSet<String> = ["t-active".to_string()].into_iter().collect();

        let removed = sweep(scratch.path(), ttl, &running, later(ttl)).await;

        assert_eq!(removed, vec!["t-old"]);
        assert!(!old.exists());
        assert!(active.exists(), "the live one is still there");
    }

    #[tokio::test]
    async fn sweeping_a_directory_that_was_never_created_is_quiet() {
        let scratch = TempDir::new();

        assert!(
            sweep(
                scratch.path(),
                Duration::from_secs(1),
                &no_active(),
                SystemTime::now()
            )
            .await
            .is_empty(),
            "nothing received means nothing to remove, not an error"
        );
    }

    #[tokio::test]
    async fn only_the_transfer_directories_are_considered() {
        let scratch = TempDir::new();

        let incoming = scratch.path().join("incoming");
        fs::create_dir_all(&incoming)
            .await
            .expect("creating incoming");

        let loose = incoming.join("not-a-directory.txt");
        fs::write(&loose, b"scratch").await.expect("writing a file");

        let removed = sweep(
            scratch.path(),
            Duration::from_secs(1),
            &no_active(),
            later(Duration::from_secs(1)),
        )
        .await;

        assert!(removed.is_empty(), "a stray file is not a transfer");
        assert!(loose.exists());
    }

    /*
     * ----------------------------------------------------------------------
     * The TTL is configuration, and bad configuration is not a purge
     * ----------------------------------------------------------------------
     */

    #[test]
    fn an_unreadable_ttl_falls_back_to_the_default() {
        // The environment is process-wide and these tests share a process, so
        // the variable is set and cleared around the read.
        let previous = std::env::var(TTL_ENV).ok();

        std::env::set_var(TTL_ENV, "not a number");
        assert_eq!(ttl_from_env(), DEFAULT_TTL);

        std::env::set_var(TTL_ENV, "90");
        assert_eq!(ttl_from_env(), Duration::from_secs(90));

        match previous {
            Some(value) => std::env::set_var(TTL_ENV, value),
            None => std::env::remove_var(TTL_ENV),
        }
    }
}
