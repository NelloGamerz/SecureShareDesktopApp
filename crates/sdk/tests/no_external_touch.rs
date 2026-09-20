//! `VilsendBuilder::in_memory()` must touch nothing external.
//!
//! ADR-0011 §3 is unambiguous about this being a contract rather than a
//! convenience: it "must touch **nothing external** — no filesystem, no
//! keychain, no network, no real clock", and "if a test needs a real socket or
//! a real keychain, that is an architectural finding, not a testing
//! inconvenience."
//!
//! A promise like that decays silently. The moment someone reaches for
//! `std::fs::metadata` to report a file's size, or `Instant::now()` for a
//! throughput number, the contract is broken and nothing fails. So it is
//! checked two ways:
//!
//! 1. **Statistically** — the crate's dependency tree and its own source are
//!    scanned for the crates and modules that *are* the outside world.
//! 2. **Behaviourally** — a full cycle is run under a clock that never moves,
//!    and the transfer is required to report the only numbers that clock
//!    implies. A single `Instant::now()` anywhere in the measured path would
//!    produce a non-zero throughput and fail this.
//!
//! The first is a lint and the second is a test, and neither is sufficient
//! alone: the lint cannot see a `SystemTime` reached through a dependency, and
//! the test cannot see a stray `PathBuf` that nothing calls yet.

use std::path::{Path, PathBuf};
use std::process::Command;

use futures::executor::block_on;

use vilsend_sdk::{
    Destination, FileRef, MemoryFiles, PeerRef, Policy, ReceiveRequest, SendRequest,
    VilsendBuilder, LOOPBACK_PEER,
};

/// The crates `vilsend-sdk` may never depend on, and the `std` modules its own
/// source may never name.
///
/// The crate list is the architecture lint's (`docs/migration/05-migration-plan.md`,
/// shared rule 9) plus `tokio`, which the SDK's public signatures are required
/// to be free of (`01-target-architecture.md` §6.1). The `std` entries are what
/// "no filesystem, no keychain, no network, no real clock" means in a language
/// where those are modules rather than crates.
const FORBIDDEN_CRATES: &[&str] = &[
    "tauri", "reqwest", "axum", "sqlx", "keyring", "tokio", "hyper", "rustls", "ureq", "curl",
];

const FORBIDDEN_MODULES: &[&str] = &[
    // The filesystem.
    "std::fs",
    // The network.
    "std::net",
    // The clock.
    "std::time::SystemTime",
    "std::time::Instant",
    // Paths: the SDK deals in opaque handles, never in paths.
    "std::path::Path",
    "std::path::PathBuf",
    // The process's environment and its process table.
    "std::env",
    "std::process",
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_sources(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("the directory is readable") {
        let path = entry.expect("a readable entry").path();

        if path.is_dir() {
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

/// Removes comments, so that a doc comment *about* `tokio` is not read as a use
/// of it.
///
/// Two things it has to get right, because getting either wrong makes the scan
/// it feeds a false negative:
///
/// - Rust's block comments **nest**, so it counts depth rather than looking for
///   the first `*/`.
/// - A `//` inside a **string literal** is not a comment, and stripping from
///   there to end-of-line would delete real code that the scan is supposed to
///   read.
///
/// It does not understand raw strings (`r"..."`), and there are none in this
/// crate. A string literal's *contents* are kept, so a forbidden token inside
/// one is still reported — failing toward failing is the right direction for a
/// lint like this.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut depth = 0usize;

    while let Some(character) = chars.next() {
        if depth > 0 {
            match character {
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    depth += 1;
                }

                '*' if chars.peek() == Some(&'/') => {
                    chars.next();
                    depth -= 1;
                }

                // Newlines are kept so that line numbers survive.
                '\n' => out.push('\n'),

                _ => {}
            }

            continue;
        }

        match character {
            '/' if chars.peek() == Some(&'/') => {
                chars.next();

                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }

            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                depth = 1;
            }

            '"' => {
                out.push('"');

                let mut escaped = false;

                for next in chars.by_ref() {
                    out.push(next);

                    if escaped {
                        escaped = false;
                    } else if next == '\\' {
                        escaped = true;
                    } else if next == '"' {
                        break;
                    }
                }
            }

            other => out.push(other),
        }
    }

    out
}

#[test]
fn the_crates_source_never_names_the_outside_world() {
    let mut sources = Vec::new();

    rust_sources(&crate_root().join("src"), &mut sources);

    assert!(
        sources.len() > 5,
        "the scan found almost nothing, so it is not scanning: {sources:?}"
    );

    let mut offences = Vec::new();

    for path in &sources {
        let source = std::fs::read_to_string(path).expect("the source is readable");
        let code = strip_comments(&source);

        for forbidden in FORBIDDEN_MODULES {
            if code.contains(forbidden) {
                offences.push(format!("{}: {forbidden}", path.display()));
            }
        }
    }

    assert!(
        offences.is_empty(),
        "`in_memory()` may touch nothing external, and these are external:\n{}",
        offences.join("\n")
    );
}

#[test]
fn the_crates_dependency_tree_holds_none_of_the_forbidden_crates() {
    // `cargo tree` rather than a parse of `Cargo.toml`, because the rule is
    // about the whole *tree*: a dependency that pulls `tokio` in transitively
    // is a dependency that puts it in this crate's build.
    //
    // `--prefix none` prints one bare `name version` line per crate, which is
    // what makes an anchored match below a match on a package name rather than
    // on a substring of one.
    let output = Command::new(env!("CARGO"))
        .args(["tree", "-p", "vilsend-sdk", "--prefix", "none"])
        .current_dir(crate_root())
        .output()
        .expect("cargo runs");

    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let tree = String::from_utf8(output.stdout).expect("the tree is utf-8");

    assert!(
        tree.lines().any(|line| line.starts_with("vilsend-sdk ")),
        "vilsend-sdk is not in its own tree, so this check proves nothing:\n{tree}"
    );

    let found: Vec<&str> = tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| FORBIDDEN_CRATES.contains(name))
        .collect();

    assert!(found.is_empty(), "vilsend-sdk must not depend on {found:?}");
}

#[test]
fn a_whole_cycle_runs_under_a_clock_that_never_moves() {
    // The behavioural half. `FakeClock` is the default for `in_memory()`, and
    // it starts stopped, so a transfer that measures its own throughput from
    // the real clock would report a large number here rather than zero.
    block_on(async {
        let source = MemoryFiles::new();
        source.insert("a.bin", vec![9u8; 8_192]);

        let sink = MemoryFiles::new();

        let client = VilsendBuilder::in_memory()
            .with_memory_source(source)
            .with_memory_sink(sink.clone())
            .with_policy(Policy {
                chunk_size: 1_024,
                ..Policy::default()
            })
            .build()
            .expect("builds");

        let peer = PeerRef::from(LOOPBACK_PEER);

        let receiving = client
            .receive(ReceiveRequest::from(peer.clone(), Destination::root()))
            .await
            .expect("listener");

        let sending = client
            .send(SendRequest::to(peer, vec![FileRef::from("a.bin")]))
            .await
            .expect("send");

        let sample = sending.progress().await;

        assert_eq!(
            sample.throughput_bps, 0,
            "no time passed on the injected clock, so no throughput can exist"
        );
        assert_eq!(sample.eta, None);
        assert_eq!(sample.degraded, None);

        sending.wait().await.expect("completed");
        receiving.wait().await.expect("completed");

        assert_eq!(sink.total_bytes(), 8_192);
    });
}

#[test]
fn a_destination_that_looks_like_a_path_stays_inside_the_store() {
    // The SDK deals in opaque destination names, not paths, and `in_memory()`
    // has no filesystem to escape into. A destination that *looks* like a
    // traversal is therefore stored verbatim rather than resolved — which is
    // the property that makes "no filesystem" more than an absence: there is no
    // path resolution here for one to reach.
    block_on(async {
        let source = MemoryFiles::new();
        source.insert("a.txt", b"x".to_vec());

        let sink = MemoryFiles::new();

        let client = VilsendBuilder::in_memory()
            .with_memory_source(source)
            .with_memory_sink(sink.clone())
            .build()
            .expect("builds");

        let peer = PeerRef::from(LOOPBACK_PEER);

        let receiving = client
            .receive(ReceiveRequest::from(
                peer.clone(),
                Destination::named("../../etc"),
            ))
            .await
            .expect("listener");

        client
            .send(SendRequest::to(peer, vec![FileRef::from("a.txt")]))
            .await
            .expect("send")
            .wait()
            .await
            .expect("completed");

        receiving.wait().await.expect("completed");

        assert_eq!(sink.paths(), vec!["../../etc/a.txt"]);
        assert_eq!(sink.get("../../etc/a.txt"), Some(b"x".to_vec()));
    });
}

#[test]
fn the_comment_stripper_ignores_prose_and_keeps_code() {
    // The scan above is only as good as this, so it gets its own test rather
    // than being trusted.
    let source = r#"
// std::fs in a line comment
/* std::net in a block comment, and /* a nested one: std::process */ */
let a = 1;
"#;

    let stripped = strip_comments(source);

    assert!(!stripped.contains("std::fs"));
    assert!(!stripped.contains("std::net"));
    assert!(!stripped.contains("std::process"));
    assert!(stripped.contains("let a = 1;"));

    // And code is kept, including code that merely looks like a comment.
    assert!(strip_comments("let url = \"a//b\";").contains("a//b"));
}
