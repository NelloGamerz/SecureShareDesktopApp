//! The rules the public API has to keep, checked mechanically.
//!
//! `05-migration-plan.md` § "Phase 5" and the phase brief both state three
//! invariants that are easy to state and easy to break silently:
//!
//! - **every public enum is `#[non_exhaustive]`** (§2.5 of `04`), so that
//!   adding a variant is not a breaking change;
//! - **no public type exposes `tauri`, `axum` or `sqlx`** — no public type may
//!   even name them, since a shell that links one of them cannot be a CLI, a
//!   mobile app or a Node addon;
//! - **no public type carries a lifetime parameter**, which is
//!   `04-sdk-cli-mobile-build-plan.md` §3.3 rule 2: "No lifetimes. Everything
//!   owned or `Arc`."
//!
//! A rule that is only in a document is a rule that holds until the first busy
//! afternoon. These scans are line-based on purpose: an attribute sits on the
//! line above the item it applies to, so an anchored scan is both sufficient
//! and impossible to fool with a cleverly wrapped expression. `cargo-public-api`
//! in CI (task 5.5) is the *precise* check; this one runs everywhere, including
//! where that tool is not installed, and fails loudly rather than drifting.

use std::path::{Path, PathBuf};

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

/// Every source file under `src/`, with its path.
fn sources() -> Vec<(PathBuf, String)> {
    let mut paths = Vec::new();

    rust_sources(&crate_root().join("src"), &mut paths);
    paths.sort();

    assert!(
        paths.len() > 5,
        "the scan found almost nothing, so it is not scanning: {paths:?}"
    );

    paths
        .into_iter()
        .map(|path| {
            let source = std::fs::read_to_string(&path).expect("the source is readable");

            (path, source)
        })
        .collect()
}

/// Whether the attribute block above `line` contains `attribute`.
///
/// Walks back over the `#[...]` lines; a doc comment or a blank line ends the
/// block, which is where a misplaced attribute would be anyway.
fn has_attribute_above(lines: &[&str], line: usize, attribute: &str) -> bool {
    let mut index = line;

    while index > 0 {
        index -= 1;

        let trimmed = lines[index].trim();

        if !trimmed.starts_with("#[") {
            return false;
        }

        if trimmed.starts_with(attribute) {
            return true;
        }
    }

    false
}

/// Lines that declare a public item, as `(line number, trimmed text)`.
fn public_declarations(lines: &[&str], keyword: &str) -> Vec<(usize, String)> {
    let needle = format!("pub {keyword} ");

    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim_start();

            trimmed
                .starts_with(&needle)
                .then(|| (index, trimmed.to_owned()))
        })
        .collect()
}

#[test]
fn every_public_enum_is_non_exhaustive() {
    // `04-sdk-cli-mobile-build-plan.md` §2.5: "`#[non_exhaustive]` on all
    // public enums." Adding a variant must not be a breaking change, which is
    // the whole reason a shell is allowed to have a catch-all arm.
    let mut offenders = Vec::new();
    let mut checked = 0usize;

    for (path, source) in sources() {
        let lines: Vec<&str> = source.lines().collect();

        for (index, declaration) in public_declarations(&lines, "enum") {
            checked += 1;

            if !has_attribute_above(&lines, index, "#[non_exhaustive]") {
                offenders.push(format!("{}:{}: {declaration}", path.display(), index + 1));
            }
        }
    }

    assert!(checked >= 4, "only {checked} public enums were found");
    assert!(
        offenders.is_empty(),
        "these public enums are not #[non_exhaustive]:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn no_public_item_names_a_shell_or_a_database() {
    // The rule is about the *public* surface, but the crate cannot name these
    // anywhere at all — `no_external_touch.rs` holds the whole source to that.
    // This test is the one that says why: a public type that mentions them is a
    // public type a non-desktop shell cannot use.
    let forbidden = ["tauri", "axum", "sqlx", "keyring", "reqwest"];

    let mut offenders = Vec::new();

    for (path, source) in sources() {
        for line in source.lines() {
            let trimmed = line.trim_start();

            if !trimmed.starts_with("pub ") {
                continue;
            }

            for name in forbidden {
                if trimmed.contains(name) {
                    offenders.push(format!("{}: {trimmed}", path.display()));
                }
            }
        }
    }

    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}

#[test]
fn no_public_type_carries_a_lifetime_parameter() {
    // §3.3 rule 2: "No lifetimes. Everything owned or `Arc`." A lifetime on a
    // public type is a lifetime that UniFFI and napi-rs both have to be taught
    // about, and a host language has no way to express.
    let mut offenders = Vec::new();

    for (path, source) in sources() {
        let lines: Vec<&str> = source.lines().collect();

        for keyword in ["struct", "enum", "trait", "type", "fn"] {
            for (_, declaration) in public_declarations(&lines, keyword) {
                if declaration.contains("<'") {
                    offenders.push(format!("{}: {declaration}", path.display()));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these public items carry a lifetime:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_scan_actually_works() {
    // A scan that silently matches nothing passes for the wrong reason, so the
    // two primitives it is built from get their own check.
    let lines = vec!["#[non_exhaustive]", "#[derive(Debug)]", "pub enum E { A }"];

    assert!(has_attribute_above(&lines, 2, "#[non_exhaustive]"));
    assert!(has_attribute_above(&lines, 2, "#[derive(Debug)]"));
    assert!(!has_attribute_above(&lines, 2, "#[serde(rename_all)]"));

    // A doc comment ends the attribute block: an attribute below one does not
    // apply.
    let separated = vec!["#[non_exhaustive]", "", "/// Docs.", "pub struct S;"];

    assert!(!has_attribute_above(&separated, 3, "#[non_exhaustive]"));

    assert_eq!(
        public_declarations(&lines, "enum"),
        vec![(2, "pub enum E { A }".to_owned())]
    );
    assert!(public_declarations(&lines, "struct").is_empty());
}

#[test]
fn every_enum_the_sdk_re_exports_is_non_exhaustive_where_it_is_defined() {
    // A re-export is still this crate's public API: a caller matching on
    // `Progress::status` is matching on `vilsend_core::TransferStatus`, and if
    // that enum were closed an added variant would break them — no matter what
    // this crate's own enums do.
    //
    // So the check reaches into `vilsend-core`'s source. `TransferStatus` was
    // the one that failed it; Phase 5 marked it, which changes nothing on the
    // wire and is pinned against the golden fixtures.
    let re_exports = std::fs::read_to_string(crate_root().join("src").join("lib.rs"))
        .expect("lib.rs is readable");

    let core_sources: Vec<String> = {
        let mut paths = Vec::new();

        rust_sources(
            &crate_root().join("..").join("core").join("src"),
            &mut paths,
        );
        paths.sort();

        paths
            .into_iter()
            .map(|path| std::fs::read_to_string(&path).expect("core source is readable"))
            .collect()
    };

    let mut checked = 0usize;

    for name in ["DomainEvent", "ErrorKind", "TransferStatus", "VilsendError"] {
        assert!(
            re_exports.contains(name),
            "{name} should be re-exported by the SDK"
        );

        let declared = core_sources
            .iter()
            .find_map(|source| {
                let lines: Vec<&str> = source.lines().collect();

                public_declarations(&lines, "enum")
                    .into_iter()
                    .find(|(_, declaration)| declaration.contains(&format!("enum {name}")))
                    .map(|(index, _)| has_attribute_above(&lines, index, "#[non_exhaustive]"))
            })
            .unwrap_or_else(|| panic!("{name} is not declared in vilsend-core"));

        assert!(declared, "{name} is re-exported but not #[non_exhaustive]");
        checked += 1;
    }

    assert_eq!(checked, 4);

    // And the deliberate exception stays an exception: `ConnectionStatus` is
    // the shell's own vocabulary and is not part of the SDK's surface, so it is
    // left alone. If it ever appears here, that decision has been reversed by
    // accident.
    assert!(
        !re_exports.contains("ConnectionStatus"),
        "ConnectionStatus is not re-exported, so it is not covered by the SDK's \
         semver policy — if that changes, mark it `#[non_exhaustive]` in core too"
    );
}
