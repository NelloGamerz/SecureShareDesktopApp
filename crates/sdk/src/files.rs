//! The in-memory file store, and the reason it is a public type.
//!
//! ADR-0011 §2 lists `InMemoryChunkSource`/`Sink` as the adapters that make a
//! transfer testable at all, and §3 makes `VilsendBuilder::in_memory()` a
//! contract: it "must touch **nothing external** — no filesystem, no keychain,
//! no network, no real clock." A backend that touches no filesystem still has
//! to get bytes from somewhere, and put them somewhere, so the something is
//! this type.
//!
//! **It is deliberately the simplest thing that can hold bytes.** It is not a
//! VFS: no permissions, no metadata, no symlinks, no streaming, no partial
//! reads. Phase 3 task 3.1 introduces the real `ChunkSource`/`ChunkSink` ports
//! and their `StdFileSource` adapters; when it does, these two types should
//! become the *in-memory implementations of those ports* rather than standing
//! beside them. Anything beyond `get`/`insert` added here before then is a
//! guess about a port nobody has written.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// A map of path to bytes, shared by cloning.
#[derive(Debug, Default, Clone)]
pub struct MemoryFiles {
    entries: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl MemoryFiles {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Puts `bytes` at `path`, returning whatever was there before.
    pub fn insert(&self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Option<Vec<u8>> {
        self.entries
            .lock()
            .expect("memory files lock poisoned")
            .insert(path.into(), bytes.into())
    }

    /// The bytes at `path`, or `None`.
    ///
    /// Clones rather than lending, so the lock is not held across a caller's
    /// work. A byte store that hands out a guarded reference is a byte store
    /// that deadlocks the first time a caller does something with it.
    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.entries
            .lock()
            .expect("memory files lock poisoned")
            .get(path)
            .cloned()
    }

    /// Whether `path` is present — including when the file it holds is empty.
    pub fn contains(&self, path: &str) -> bool {
        self.entries
            .lock()
            .expect("memory files lock poisoned")
            .contains_key(path)
    }

    /// Takes `path` out of the store, returning what it held.
    pub fn remove(&self, path: &str) -> Option<Vec<u8>> {
        self.entries
            .lock()
            .expect("memory files lock poisoned")
            .remove(path)
    }

    /// Every path present, in sorted order.
    ///
    /// Sorted, so that a test asserting on it is deterministic without having
    /// to sort what it got back.
    pub fn paths(&self) -> Vec<String> {
        self.entries
            .lock()
            .expect("memory files lock poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// How many paths are stored.
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .expect("memory files lock poisoned")
            .len()
    }

    /// Whether nothing is stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The total number of bytes held.
    pub fn total_bytes(&self) -> u64 {
        self.entries
            .lock()
            .expect("memory files lock poisoned")
            .values()
            .map(|bytes| bytes.len() as u64)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_starts_empty() {
        let files = MemoryFiles::new();

        assert!(files.is_empty());
        assert_eq!(files.len(), 0);
        assert_eq!(files.total_bytes(), 0);
        assert!(files.paths().is_empty());
    }

    #[test]
    fn inserting_and_reading_back_is_lossless() {
        let files = MemoryFiles::new();

        assert_eq!(files.insert("a.bin", vec![0u8, 1, 2, 255]), None);
        assert_eq!(files.get("a.bin"), Some(vec![0u8, 1, 2, 255]));
        assert_eq!(files.total_bytes(), 4);
    }

    #[test]
    fn inserting_over_an_existing_path_returns_what_was_there() {
        let files = MemoryFiles::new();

        files.insert("a.txt", b"old".to_vec());

        assert_eq!(
            files.insert("a.txt", b"new".to_vec()),
            Some(b"old".to_vec())
        );
        assert_eq!(files.get("a.txt"), Some(b"new".to_vec()));
        assert_eq!(files.len(), 1, "the path was replaced, not duplicated");
    }

    #[test]
    fn paths_are_sorted_so_an_assertion_on_them_is_deterministic() {
        let files = MemoryFiles::new();

        files.insert("z.txt", b"z".to_vec());
        files.insert("a.txt", b"a".to_vec());
        files.insert("m/n.txt", b"m".to_vec());

        assert_eq!(files.paths(), vec!["a.txt", "m/n.txt", "z.txt"]);
    }

    #[test]
    fn a_missing_path_is_none_rather_than_a_panic() {
        let files = MemoryFiles::new();

        assert_eq!(files.get("nope"), None);
        assert!(!files.contains("nope"));
        assert_eq!(files.remove("nope"), None);
    }

    #[test]
    fn cloning_shares_the_store_rather_than_copying_it() {
        // The builder takes the store by value and the caller keeps one; if a
        // clone copied, the caller would be looking at a different store from
        // the engine and every test would be vacuous.
        let files = MemoryFiles::new();
        let clone = files.clone();

        files.insert("a.txt", b"a".to_vec());

        assert_eq!(clone.get("a.txt"), Some(b"a".to_vec()));
        assert_eq!(clone.len(), 1);
    }

    #[test]
    fn an_empty_file_is_a_file() {
        // Zero-byte files are a real case the chunker has to handle, and a
        // store that treated "no bytes" as "no entry" would lose them.
        let files = MemoryFiles::new();

        files.insert("empty.txt", Vec::new());

        assert!(files.contains("empty.txt"));
        assert_eq!(files.get("empty.txt"), Some(Vec::new()));
        assert_eq!(files.len(), 1);
    }
}
