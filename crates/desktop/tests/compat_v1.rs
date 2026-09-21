//! The wire-compatibility floor (ADR-0010, `06-testing-and-quality.md` §6.2).
//!
//! > A v2 client / v1 client receiver: **Must work** — tunnel only, no resume.
//!
//! Every test here sends through the **real** v1 sender path — the real header
//! set from `transfer::http_client`, the real chunk reader and encryptor from
//! `transfer::upload` — to a receiver that is an independent implementation of
//! the frozen v1 schema (`tests/v1_fixture`). Nothing in this file re-implements
//! the code it is testing, and the fixture does not call the product's crypto.
//!
//! # What makes this test load-bearing
//!
//! It compiles and runs in **both** feature configurations. With `protocol-v2`
//! off it is a plain v1 round trip. With `protocol-v2` on it asserts the thing
//! the whole phase is arranged around: **a build with the v2 wire compiled in
//! still speaks v1 on the wire**, because capability probing — and therefore
//! the decision to use v2 at all — is task 4.11 and does not exist yet.
//!
//! Later sub-phases must keep this green. If 4a–4e ever make a
//! `protocol-v2` build emit v2 bytes to a peer that has not negotiated them,
//! this is the file that fails.
//!
//! # The limitation, stated plainly
//!
//! The receiver here is a **fixture**, not the `v1.0.4` binary. A pinned v1
//! binary cannot be run headless: at that tag the receiver takes a
//! `tauri::AppHandle` and needs the settings store, the Downloads directory and
//! the OS keyring on the request path. `tests/v1_fixture` explains this in full
//! and is written to compensate — it re-derives the v1 KDF and re-implements
//! the v1 AEAD from the schema rather than calling the product's, so that a
//! change to v1 fails this test.

mod v1_fixture;

use server_frontend_lib::wire::{crypto, http_client::HttpClient, upload, ChunkJob};
use v1_fixture::{start, Fixture, CHUNK_HEADERS};

/// The bearer token the fixture's presence-only check accepts. v1 never reads
/// the value — `docs/SECURITY.md:5-18`, Phase 9's finding.
const TOKEN: &str = "fixture-token";

const TRANSFER_ID: &str = "compat-transfer-1";
const FILE_ID: &str = "compat-file-1";

/// The v1 destination, relative to the receiver's root. Deliberately nested, so
/// the `Relative-Path` header is exercised rather than assumed.
const DESTINATION: &str = "nested/destination.bin";

/// Small chunks, so a handful of them cross the wire and the merge path runs.
const CHUNK_SIZE: usize = 4096;

/// A deterministic payload. Random bytes from a fixed seed, so a failure is
/// reproducible, and large enough to be several chunks with a short last one.
fn payload(len: usize) -> Vec<u8> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;

    (0..len)
        .map(|_| {
            // xorshift64*, expanded to a byte. Not a CSPRNG and not meant to
            // be: the test needs the same bytes every run.
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;

            (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u8
        })
        .collect()
}

/// The chunk jobs the real chunker would produce for one file.
fn jobs_of(source: &std::path::Path, len: usize) -> Vec<ChunkJob> {
    let total = (len as u64).div_ceil(CHUNK_SIZE as u64).max(1);

    (0..total)
        .map(|index| {
            let offset = index * CHUNK_SIZE as u64;

            ChunkJob {
                transfer_id: TRANSFER_ID.into(),
                file_id: FILE_ID.into(),
                index,
                total,
                relative_path: DESTINATION.into(),
                path: source.to_path_buf(),
                offset,
                length: ((len as u64 - offset).min(CHUNK_SIZE as u64)) as usize,
            }
        })
        .collect()
}

/// A sender with the v1 key material, exactly as `UploadManager::start` builds
/// it. Returned so a test can drive individual chunks rather than only the
/// whole-transfer helper below.
struct Sender {
    client: HttpClient,
    endpoint: String,
    transfer_key: [u8; 32],
}

impl Sender {
    /// Fetch the receiver's public key, derive the transfer key, and open the
    /// session — the v1 handshake, three calls in order, using the product's
    /// own crypto exactly as `UploadManager::start` does.
    async fn connect(fixture: &Fixture, len: usize) -> Self {
        let client = HttpClient::new();
        let endpoint = fixture.endpoint.clone();

        let receiver_public_key = client
            .get_public_key(&endpoint, TOKEN)
            .await
            .expect("the fixture must serve its public key");

        let receiver_public = crypto::decode_public_key(&receiver_public_key)
            .expect("the fixture sends a 32-byte key");

        let ephemeral = crypto::generate_ephemeral_keypair();

        let shared = crypto::derive_shared_secret(&ephemeral.private_key, &receiver_public);

        let transfer_key = crypto::derive_transfer_key(&shared).expect("a 32-byte HKDF output");

        client
            .start_transfer(
                &endpoint,
                TOKEN,
                TRANSFER_ID,
                &crypto::encode_public_key(&ephemeral.public_key),
                len as u64,
            )
            .await
            .expect("the fixture must accept the session");

        Self {
            client,
            endpoint,
            transfer_key,
        }
    }

    /// Upload one chunk through the real sender path.
    async fn send(&self, job: &ChunkJob) {
        upload::upload_chunk(
            &self.client,
            &self.endpoint,
            TOKEN,
            job,
            &self.transfer_key,
            0,
        )
        .await
        .expect("the v1 receiver must accept this chunk");
    }
}

/*
 * ----------------------------------------------------------------------
 * Which side is pinned, and why it is asymmetrical
 * ----------------------------------------------------------------------
 *
 * The **sender** above uses the product's own code throughout —
 * `transfer::http_client` for the requests, `transfer::upload` for reading and
 * encrypting a chunk, `transfer::crypto` for the key agreement and the KDF.
 * That is the point: the sender *is* the thing under test, and a round trip
 * through anything other than the shipped sender would prove nothing about it.
 *
 * The **receiver** in `tests/v1_fixture` is the opposite: it re-implements the
 * frozen v1 KDF and AEAD from the schema and never calls `transfer::crypto`.
 *
 * The asymmetry is deliberate and it is what makes the test load-bearing. If
 * both sides called the product, a change to the v1 wire would move them
 * together, the transfer would still round-trip, and the test would pass while
 * every installed client broke. Because only one side is pinned, any change to
 * the product's v1 crypto makes the two disagree and this file fails. That was
 * verified by changing the v1 KDF's `info` string and confirming the round trip
 * breaks — see the phase report.
 */

/*
 * ----------------------------------------------------------------------
 * The compatibility floor
 * ----------------------------------------------------------------------
 */

/// The matrix cell that must never break: a build of this repository completes
/// a transfer to a v1 receiver.
///
/// Runs in both feature configurations. With `protocol-v2` on, this is the
/// assertion that compiling the v2 wire in changes nothing about what goes out
/// on the wire.
#[tokio::test]
async fn a_transfer_to_a_v1_receiver_completes_byte_for_byte() {
    let fixture = start().await;

    let expected = payload(5 * CHUNK_SIZE + 137);

    let source = fixture.receiver.root().join("source.bin");
    std::fs::write(&source, &expected).expect("could not write the source file");

    let sender = Sender::connect(&fixture, expected.len()).await;

    for job in jobs_of(&source, expected.len()) {
        sender.send(&job).await;
    }

    let delivered = std::fs::read(fixture.receiver.root().join(DESTINATION))
        .expect("the fixture must have merged the file");

    assert_eq!(
        delivered, expected,
        "the v1 receiver must reconstruct the file exactly"
    );

    // The per-file staging directory is removed on success. v1 removes exactly
    // `incoming/{transfer}/{file}` and leaves the now-empty
    // `incoming/{transfer}` behind, so that is what is asserted — matching the
    // receiver rather than the tidier thing.
    assert!(
        !fixture
            .receiver
            .root()
            .join("incoming")
            .join(TRANSFER_ID)
            .join(FILE_ID)
            .exists(),
        "a completed transfer must leave no per-file staging directory"
    );
}

/// The same transfer, with chunks arriving **out of order**.
///
/// The default concurrency is 4, so this is the normal case rather than an edge
/// one, and it is the case that would break if a change ever made the receiver
/// depend on arrival order.
#[tokio::test]
async fn a_transfer_to_a_v1_receiver_completes_with_chunks_out_of_order() {
    let fixture = start().await;

    let expected = payload(3 * CHUNK_SIZE + 11);

    let source = fixture.receiver.root().join("source.bin");
    std::fs::write(&source, &expected).expect("could not write the source file");

    let sender = Sender::connect(&fixture, expected.len()).await;

    let mut jobs = jobs_of(&source, expected.len());
    jobs.reverse();

    for job in &jobs {
        sender.send(job).await;
    }

    assert_eq!(
        std::fs::read(fixture.receiver.root().join(DESTINATION)).expect("merged"),
        expected
    );
}

/// A chunk that is already present is not rewritten, which is the v1 behaviour
/// (`fs::metadata(&part).is_err()`) that makes a v1 receiver accidentally
/// resumable at file granularity. `02-transport-layer.md` §5.4 promises exactly
/// this and no more, so it is pinned rather than left implicit.
#[tokio::test]
async fn re_sending_a_chunk_is_a_no_op_on_a_v1_receiver() {
    let fixture = start().await;

    let expected = payload(CHUNK_SIZE + 5);

    let source = fixture.receiver.root().join("source.bin");
    std::fs::write(&source, &expected).expect("could not write the source file");

    let sender = Sender::connect(&fixture, expected.len()).await;

    let jobs = jobs_of(&source, expected.len());

    sender.send(&jobs[0]).await;
    sender.send(&jobs[0]).await;

    for job in &jobs[1..] {
        sender.send(job).await;
    }

    assert_eq!(
        std::fs::read(fixture.receiver.root().join(DESTINATION)).expect("merged"),
        expected,
        "a duplicate chunk must not be written twice"
    );
}

/*
 * ----------------------------------------------------------------------
 * Guarding the fixture
 * ----------------------------------------------------------------------
 */

/// The fixture must be as strict as the receiver it stands for, or the three
/// tests above prove nothing.
///
/// If the fixture ever stopped decrypting — or stopped requiring a header — the
/// round trips would still pass while testing a receiver that accepts anything.
#[tokio::test]
async fn the_fixture_rejects_a_chunk_it_cannot_decrypt() {
    let fixture = start().await;

    let expected = payload(64);

    let source = fixture.receiver.root().join("source.bin");
    std::fs::write(&source, &expected).expect("could not write the source file");

    let sender = Sender::connect(&fixture, expected.len()).await;

    let jobs = jobs_of(&source, expected.len());

    // Seal the chunk correctly, then flip a ciphertext byte in flight. The real
    // `HttpClient::send` reports the receiver's rejection as an error.
    let job = &jobs[0];

    let mut bytes = vec![0u8; job.length];
    std::fs::File::open(&source)
        .and_then(|mut file| {
            use std::io::{Read, Seek, SeekFrom};
            file.seek(SeekFrom::Start(job.offset))?;
            file.read_exact(&mut bytes)
        })
        .expect("could not read the source");

    let encrypted = crypto::encrypt_chunk(&sender.transfer_key, &bytes).expect("sealing");

    let mut tampered = encrypted.data;
    tampered[0] ^= 0x01;

    let result = sender
        .client
        .send(&sender.endpoint, TOKEN, job, tampered, encrypted.nonce)
        .await;

    assert!(
        result.is_err(),
        "a receiver that accepted this is not decrypting anything"
    );
}

/// Every header in the frozen set is required, and a complete set is accepted.
///
/// The sender emits all of them, so this asserts the fixture would notice if it
/// stopped requiring one — a fixture that became lenient would make the three
/// round trips above pass against a receiver that is not the v1 receiver.
///
/// Driven with `reqwest` directly rather than through `HttpClient`, because the
/// point is to control the header set, and `HttpClient`'s job is to emit the
/// one correct set.
#[tokio::test]
async fn the_fixture_requires_every_v1_chunk_header() {
    let fixture = start().await;

    let sender = Sender::connect(&fixture, 9).await;

    let plaintext = b"a v1 chunk";
    let encrypted = crypto::encrypt_chunk(&sender.transfer_key, plaintext).expect("sealing");

    let nonce_b64 = {
        use base64::{engine::general_purpose::STANDARD, Engine};

        STANDARD.encode(&encrypted.nonce)
    };

    let value_of = |header: &str| -> String {
        match header {
            "Authorization" => format!("Bearer {TOKEN}"),
            "Transfer-Id" => TRANSFER_ID.into(),
            "File-Id" => FILE_ID.into(),
            "Chunk-Index" => "0".into(),
            "Total-Chunks" => "1".into(),
            "Relative-Path" => DESTINATION.into(),
            "Chunk-Nonce" => nonce_b64.clone(),
            "Encryption" => "AES-256-GCM".into(),
            other => panic!("the fixture requires a header this test cannot supply: {other}"),
        }
    };

    let url = format!("{}/transfer/chunk", sender.endpoint);

    let post = |omit: Option<&str>| {
        let mut request = reqwest::Client::new().post(&url);

        for name in CHUNK_HEADERS {
            if Some(name) == omit {
                continue;
            }

            request = request.header(name, value_of(name));
        }

        request.body(encrypted.data.clone())
    };

    let complete = post(None).send().await.expect("request");

    assert_eq!(
        complete.status(),
        reqwest::StatusCode::OK,
        "the complete frozen header set must be accepted"
    );

    for name in CHUNK_HEADERS {
        let response = post(Some(name)).send().await.expect("request");

        // v1 answers 401 for a missing Authorization and 400 for every other
        // missing header. That asymmetry is `writer::receive`'s, and the
        // fixture reproduces it rather than tidying it.
        let expected = if name == "Authorization" {
            reqwest::StatusCode::UNAUTHORIZED
        } else {
            reqwest::StatusCode::BAD_REQUEST
        };

        assert_eq!(
            response.status(),
            expected,
            "omitting {name} must be {}, not {}",
            expected.as_u16(),
            response.status()
        );

        // The body names the missing header, so this is the header check and
        // not some other rejection for the same omission.
        let body = response.text().await.unwrap_or_default().to_lowercase();

        assert!(
            body.contains(&name.to_lowercase()),
            "the rejection should name {name}, got: {body}"
        );
    }
}

/*
 * ----------------------------------------------------------------------
 * v1 is not v2 — only meaningful once the v2 wire is compiled in
 * ----------------------------------------------------------------------
 */

/// The other half of the compatibility story: v1 and v2 are **different bytes**
/// on the same endpoint, and a v2 chunk sent to a v1 receiver is refused.
///
/// Without this, everything above could pass for a build whose v2 path silently
/// produced v1 ciphertext — which would mean the AAD in task 4.1 was never
/// actually applied.
#[cfg(feature = "protocol-v2")]
#[tokio::test]
async fn a_v2_sealed_chunk_is_rejected_by_a_v1_receiver() {
    use server_frontend_lib::wire::crypto_v2;

    let fixture = start().await;

    let sender = Sender::connect(&fixture, 64).await;

    // A v2 key from a v2 derivation, and a chunk bound to its slot by an AAD.
    let v2_key = crypto_v2::derive_transfer_key_v2(&[0x42; 32], &[0x01; 16], &[0x02; 16])
        .expect("a 32-byte HKDF output");

    let aad = crypto_v2::ChunkAad::new(TRANSFER_ID, FILE_ID, 0, DESTINATION).encode();

    let sealed =
        crypto_v2::seal(&v2_key, &aad, &[0x0A; 12], b"a v2 chunk").expect("sealing a 9-byte chunk");

    let source = fixture.receiver.root().join("source.bin");
    std::fs::write(&source, b"a v2 chunk").expect("could not write the source file");

    let job = ChunkJob {
        transfer_id: TRANSFER_ID.into(),
        file_id: FILE_ID.into(),
        index: 0,
        total: 1,
        relative_path: DESTINATION.into(),
        path: source,
        offset: 0,
        length: 9,
    };

    let result = sender
        .client
        .send(
            &sender.endpoint,
            TOKEN,
            &job,
            sealed.ciphertext,
            sealed.nonce.to_vec(),
        )
        .await;

    assert!(
        result.is_err(),
        "a v1 receiver has no AAD and must not accept a chunk sealed with one"
    );
}

/// And the reverse direction, which is the one that would break the installed
/// base: with `protocol-v2` compiled in, the **v1 sender path** still emits v1
/// bytes.
///
/// The `protocol-v2` build's `PROTOCOL_VERSION` is 2, so a sender that keyed
/// its behaviour off that constant rather than off a capability probe would
/// send v2 here. This asserts it does not.
#[cfg(feature = "protocol-v2")]
#[tokio::test]
async fn a_protocol_v2_build_speaks_v1_and_reports_it_as_the_fallback() {
    assert_eq!(
        vilsend_core::PROTOCOL_VERSION,
        vilsend_core::PROTOCOL_V2,
        "this build speaks v2"
    );

    // ...and still completes against a v1 receiver, which the round trip above
    // proves with the same code path. What this test adds is that the version
    // it falls back to is nameable.
    assert_eq!(vilsend_core::PROTOCOL_V1, 1);

    let fixture = start().await;
    let expected = payload(CHUNK_SIZE + 3);

    let source = fixture.receiver.root().join("source.bin");
    std::fs::write(&source, &expected).expect("could not write the source file");

    let sender = Sender::connect(&fixture, expected.len()).await;

    for job in jobs_of(&source, expected.len()) {
        sender.send(&job).await;
    }

    assert_eq!(
        std::fs::read(fixture.receiver.root().join(DESTINATION)).expect("merged"),
        expected
    );
}
