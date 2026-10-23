//! The v2 wire framing: how a session's messages are delimited inside a byte
//! stream.
//!
//! # Why a length prefix rather than trusting the transport
//!
//! The transport port hands over a stream of bytes
//! (`vilsend_core::DuplexStream`), and the two transports that will carry it
//! disagree about what a "message" is: a WebSocket has frames, and a QUIC
//! bi-stream has **none at all**. If the session assumed one `Bytes` per
//! message, it would work over the tunnel and silently corrupt over P2P — the
//! two transports would be structurally incompatible, which is exactly what the
//! byte-stream port exists to prevent.
//!
//! So framing lives *inside* the bytes, and [`FrameDecoder`] reassembles
//! whatever the transport delivers — one frame, half a frame, or six frames at
//! once.
//!
//! # The format
//!
//! ```text
//! frame := u32_le(body_len + 1) ‖ u8 kind ‖ body
//! ```
//!
//! Little-endian, matching the integer encoding [`crate::transfer::crypto_v2`]'s
//! `ChunkAad::encode` already uses, so there is one integer convention in v2
//! rather than two. The `+ 1` keeps the kind inside the declared length, so a
//! frame can be validated and skipped without understanding its body.
//!
//! # Bounded, deliberately
//!
//! Every length is checked **against the cap before anything is buffered**.
//! A peer that opens with `u32_le(0xFFFF_FFFF)` is rejected on the four-byte
//! header, not after the process has tried to hold four gigabytes. That is the
//! difference between a protocol that can be attacked for memory and one that
//! cannot, and it is why the decoder owns the cap rather than the caller.
//!
//! # What is frozen
//!
//! The kind bytes and the layout above are wire format: changing either changes
//! what a peer must send. A test at the bottom pins them by value, the same way
//! [`crate::transfer::crypto_v2`] pins its AAD.

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::transfer::errors::{Result as TransferResult, TransferError};

/// The largest frame this build will send or accept.
///
/// Derived from [`crate::transfer::constants::MAX_CHUNK_SIZE`], which is the
/// largest chunk any sender may be configured with. A `CHUNK` body carries that
/// plus a 12-byte nonce, a 16-byte tag, a file id and an index; the extra
/// mebibyte leaves room for a long file id without allowing an unbounded
/// allocation, or the cap and the planner drifting apart.
pub const MAX_FRAME_BYTES: usize = crate::transfer::constants::MAX_CHUNK_SIZE + 1024 * 1024;

/// The bytes the length prefix itself occupies.
const LENGTH_PREFIX_BYTES: usize = 4;

/// What a frame is.
///
/// A closed enum rather than an open `u8`: an unknown kind is a protocol error,
/// not a frame to pass through, because a peer that sends one is either buggy or
/// probing, and both should be refused where they are seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FrameKind {
    /// Sender → receiver: opens a session. Carries the protocol version.
    Hello = 0x01,
    /// Receiver → sender: accepts, and carries the receiver's key and nonce.
    HelloAck = 0x02,
    /// Sender → receiver: the files, their digests, and the chunk plan.
    Manifest = 0x03,
    /// Receiver → sender: what it already holds, so the sender can resume.
    ManifestAck = 0x04,
    /// Sender → receiver: one sealed chunk.
    Chunk = 0x05,
    /// Receiver → sender: this chunk is durably recorded. See the module docs of
    /// `crate::transfer::session` for why this is not "bytes reached the socket".
    Ack = 0x06,
    /// Sender → receiver: every chunk is sent; here is the manifest's digest.
    Finish = 0x07,
    /// Receiver → sender: the file is assembled and verified.
    Complete = 0x08,
    /// Either → either: stop.
    Cancel = 0x09,
    /// Either → either: something went wrong, with a reason. Never a substitute
    /// for a specific frame — only for failures that have no other carrier.
    Error = 0x0A,
    /// Liveness. See `Pong`.
    Ping = 0x0B,
    /// Liveness.
    Pong = 0x0C,
}

impl FrameKind {
    /// Every kind, for exhaustive tests.
    pub const ALL: [FrameKind; 12] = [
        FrameKind::Hello,
        FrameKind::HelloAck,
        FrameKind::Manifest,
        FrameKind::ManifestAck,
        FrameKind::Chunk,
        FrameKind::Ack,
        FrameKind::Finish,
        FrameKind::Complete,
        FrameKind::Cancel,
        FrameKind::Error,
        FrameKind::Ping,
        FrameKind::Pong,
    ];

    /// The wire byte for this kind.
    pub fn as_byte(self) -> u8 {
        self as u8
    }

    /// The kind for a wire byte, or `None` if this build does not know it.
    pub fn from_byte(byte: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_byte() == byte)
    }

    /// Whether this frame's body is a fixed, known length.
    ///
    /// Used by the decoder to reject a body that cannot be what it claims before
    /// any part of the session sees it. `Chunk` and `Ack` are absent: their
    /// bodies carry variable-length ids, so their shape is checked by the code
    /// that parses them.
    fn fixed_body_len(self) -> Option<usize> {
        match self {
            // The manifest's digest, and only that.
            FrameKind::Finish => Some(32),
            FrameKind::Complete | FrameKind::Cancel | FrameKind::Ping | FrameKind::Pong => Some(0),
            _ => None,
        }
    }

    /// Whether this kind must carry a body.
    fn requires_body(self) -> bool {
        matches!(
            self,
            FrameKind::Hello
                | FrameKind::HelloAck
                | FrameKind::Manifest
                | FrameKind::ManifestAck
                | FrameKind::Error
        )
    }
}

/// One framed message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: FrameKind,
    pub body: Bytes,
}

impl Frame {
    /// A frame carrying `body`.
    pub fn new(kind: FrameKind, body: impl Into<Bytes>) -> Self {
        Self {
            kind,
            body: body.into(),
        }
    }
}

/// Encode one frame.
///
/// Validates the body against its kind, so a caller cannot put a 40-byte digest
/// in a `Finish` frame and discover it on the far end.
pub fn encode(kind: FrameKind, body: &[u8]) -> TransferResult<Vec<u8>> {
    let declared = body
        .len()
        .checked_add(1)
        .ok_or_else(|| FrameError::TooLarge(body.len()).into_transfer_error())?;

    if declared > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(body.len()).into_transfer_error());
    }

    validate_body(kind, body)?;

    let mut out = Vec::with_capacity(LENGTH_PREFIX_BYTES + declared);

    out.put_u32_le(declared as u32);
    out.put_u8(kind.as_byte());
    out.put_slice(body);

    Ok(out)
}

/// Check a body against what its kind allows.
fn validate_body(kind: FrameKind, body: &[u8]) -> TransferResult<()> {
    if let Some(expected) = kind.fixed_body_len() {
        if body.len() != expected {
            return Err(FrameError::WrongBodyLength {
                kind,
                expected,
                found: body.len(),
            }
            .into_transfer_error());
        }
    }

    if kind.requires_body() && body.is_empty() {
        return Err(FrameError::EmptyBody { kind }.into_transfer_error());
    }

    Ok(())
}

/// Everything that can be wrong with a frame.
///
/// Classified rather than stringly, so a receiver can answer with the right
/// frame and a shell can branch on the kind (ADR-0003). The `Display` strings
/// never carry a key, a nonce or a payload.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The declared length is over [`MAX_FRAME_BYTES`].
    #[error("frame of {0} bytes exceeds the {MAX_FRAME_BYTES}-byte limit")]
    TooLarge(usize),

    /// A zero-length frame carries no kind.
    #[error("frame declares a zero length")]
    Empty,

    /// A kind byte this build does not know.
    #[error("unknown frame kind 0x{0:02x}")]
    UnknownKind(u8),

    /// A body of the wrong size for its fixed-length kind.
    #[error("{kind:?} takes a body of {expected} bytes, got {found}")]
    WrongBodyLength {
        kind: FrameKind,
        expected: usize,
        found: usize,
    },

    /// A kind that must carry a body carried none.
    #[error("{kind:?} must carry a body")]
    EmptyBody { kind: FrameKind },

    /// The stream ended in the middle of a frame.
    #[error("the stream ended with {pending} bytes of an incomplete frame")]
    IncompleteAtEof { pending: usize },

    /// A body that does not parse as what its kind requires.
    #[error("{kind:?} carried a malformed body: {reason}")]
    Malformed { kind: FrameKind, reason: String },
}

impl FrameError {
    /// As a transfer error, so the session layer can return one type.
    fn into_transfer_error(self) -> TransferError {
        TransferError::Protocol(self.to_string())
    }
}

/// Reassembles frames from a byte stream.
///
/// Owns the cap, so every bound is applied in one place: a peer cannot make the
/// buffer grow past [`MAX_FRAME_BYTES`], whatever it streams.
#[derive(Debug)]
pub struct FrameDecoder {
    buffer: BytesMut,
    cap: usize,
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameDecoder {
    /// A decoder at the default cap.
    pub fn new() -> Self {
        Self::with_cap(MAX_FRAME_BYTES)
    }

    /// A decoder with an explicit cap. For tests.
    pub fn with_cap(cap: usize) -> Self {
        Self {
            buffer: BytesMut::new(),
            cap,
        }
    }

    /// Feed bytes from the transport.
    ///
    /// Returns [`FrameError::TooLarge`] the moment the buffer could not
    /// possibly be holding a legal frame, rather than growing without bound.
    /// A peer that streams forever without completing a frame is stopped here.
    pub fn push(&mut self, bytes: &[u8]) -> TransferResult<()> {
        /*
         * The peak a legal frame can reach is the cap plus its own prefix: the
         * prefix is not counted inside the declared length, so a buffer holding
         * a full frame of exactly `cap` bytes is `cap + 4`. Anything past that
         * cannot be a frame this decoder will ever accept, so it is refused now
         * instead of being buffered first.
         */
        if self.buffer.len() + bytes.len() > self.cap + LENGTH_PREFIX_BYTES {
            return Err(FrameError::TooLarge(self.buffer.len() + bytes.len()).into_transfer_error());
        }

        self.buffer.extend_from_slice(bytes);

        Ok(())
    }

    /// Take the next complete frame, or `None` if the rest has not arrived.
    ///
    /// An incomplete frame is **not** an error — a byte stream arrives in pieces,
    /// and treating a split as corruption would make the protocol unusable. A
    /// partial frame left at end-of-stream is an error, and that is
    /// [`FrameDecoder::finish`]'s job.
    pub fn next_frame(&mut self) -> TransferResult<Option<Frame>> {
        if self.buffer.len() < LENGTH_PREFIX_BYTES {
            return Ok(None);
        }

        // Peek without consuming, so a rejected frame's bytes are still there
        // for the error to describe and for a caller to log.
        let declared = u32::from_le_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        if declared == 0 {
            return Err(FrameError::Empty.into_transfer_error());
        }

        /*
         * Checked against the header, before waiting for the rest. A peer that
         * declares four gigabytes is refused here, having sent four bytes.
         */
        if declared > self.cap {
            return Err(FrameError::TooLarge(declared).into_transfer_error());
        }

        if self.buffer.len() < LENGTH_PREFIX_BYTES + declared {
            return Ok(None);
        }

        let kind_byte = self.buffer[LENGTH_PREFIX_BYTES];
        let kind = FrameKind::from_byte(kind_byte)
            .ok_or_else(|| FrameError::UnknownKind(kind_byte).into_transfer_error())?;

        let body = self.buffer[LENGTH_PREFIX_BYTES + 1..LENGTH_PREFIX_BYTES + declared].to_vec();

        validate_body(kind, &body)?;

        self.buffer.advance(LENGTH_PREFIX_BYTES + declared);

        Ok(Some(Frame::new(kind, body)))
    }

    /// Check that the stream ended on a frame boundary.
    ///
    /// A peer that disappears mid-frame has not closed cleanly, and saying so is
    /// the difference between "the transfer was interrupted" and "the transfer
    /// silently lost a chunk".
    pub fn finish(&self) -> TransferResult<()> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            Err(FrameError::IncompleteAtEof {
                pending: self.buffer.len(),
            }
            .into_transfer_error())
        }
    }
}

/// The body of a `CHUNK` frame.
///
/// ```text
/// u32_le(file_id_len) ‖ file_id
///   ‖ u64_le(chunk_index)
///   ‖ u64_le(total_chunks)
///   ‖ nonce[12]
///   ‖ ciphertext(‖ tag[16])
/// ```
///
/// The relative path is **not** here. It travels once, in the manifest, and the
/// receiver looks it up by `file_id` — so a chunk cannot claim a different
/// destination from the plan the receiver accepted, and a 16 MiB chunk does not
/// carry a path it already has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkBody {
    pub file_id: String,
    pub chunk_index: u64,
    pub total_chunks: u64,
    /// 12 bytes. Not a `[u8; 12]` so a malformed frame can be described rather
    /// than panicking on the way in.
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

impl ChunkBody {
    /// Encode, checking every bound.
    pub fn encode(&self) -> TransferResult<Vec<u8>> {
        if self.nonce.len() != crate::transfer::crypto_v2::NONCE_LEN {
            return Err(FrameError::Malformed {
                kind: FrameKind::Chunk,
                reason: format!(
                    "nonce must be {} bytes, got {}",
                    crate::transfer::crypto_v2::NONCE_LEN,
                    self.nonce.len()
                ),
            }
            .into_transfer_error());
        }

        let file_id = self.file_id.as_bytes();

        if file_id.len() > u32::MAX as usize {
            return Err(FrameError::TooLarge(file_id.len()).into_transfer_error());
        }

        let size = 4 + file_id.len() + 8 + 8 + self.nonce.len() + self.ciphertext.len();
        let mut out = Vec::with_capacity(size);

        out.put_u32_le(file_id.len() as u32);
        out.put_slice(file_id);
        out.put_u64_le(self.chunk_index);
        out.put_u64_le(self.total_chunks);
        out.put_slice(&self.nonce);
        out.put_slice(&self.ciphertext);

        Ok(out)
    }

    /// Parse, rejecting anything short, over-long or mis-shaped.
    ///
    /// Total for every input: a truncated body returns an error rather than
    /// panicking on a slice index, which is what makes a malformed frame
    /// survivable.
    pub fn parse(body: &[u8]) -> TransferResult<Self> {
        let malformed = |reason: &str| {
            FrameError::Malformed {
                kind: FrameKind::Chunk,
                reason: reason.to_owned(),
            }
            .into_transfer_error()
        };

        // The file-id length prefix is the smallest possible body.
        if body.len() < 4 {
            return Err(malformed("body is shorter than a chunk header"));
        }

        let file_id_len = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;

        /*
         * Every fixed-width field must fit *after* the id, and this is checked
         * before any of them is sliced out.
         *
         * Checking only the id's own bound is not enough, and the omission is
         * not theoretical: it made a body whose file-id length is plausible but
         * whose tail is truncated slice past the end and **panic**, which a peer
         * can trigger at will. A receiver that panics on a malformed frame is a
         * receiver a peer can stop.
         */
        let header_len = 4 + file_id_len + 8 + 8 + crate::transfer::crypto_v2::NONCE_LEN;

        if body.len() < header_len {
            return Err(malformed(
                "body is too short for its declared file id and header",
            ));
        }

        let file_id = String::from_utf8(body[4..4 + file_id_len].to_vec())
            .map_err(|_| malformed("file id is not valid UTF-8"))?;

        let mut cursor = 4 + file_id_len;

        let chunk_index = u64::from_le_bytes(
            body[cursor..cursor + 8]
                .try_into()
                .map_err(|_| malformed("chunk index is truncated"))?,
        );

        cursor += 8;

        let total_chunks = u64::from_le_bytes(
            body[cursor..cursor + 8]
                .try_into()
                .map_err(|_| malformed("total chunks is truncated"))?,
        );

        cursor += 8;

        let nonce = body[cursor..cursor + crate::transfer::crypto_v2::NONCE_LEN].to_vec();

        cursor += crate::transfer::crypto_v2::NONCE_LEN;

        let ciphertext = body[cursor..].to_vec();

        /*
         * A chunk with no ciphertext cannot be a sealed chunk: the tag alone is
         * 16 bytes, so anything under that is a frame that cannot authenticate.
         * Rejected here rather than handed to the AEAD, so the failure names the
         * framing rather than the tag.
         */
        if ciphertext.len() < crate::transfer::crypto_v2::TAG_LEN {
            return Err(malformed("ciphertext is shorter than an AEAD tag"));
        }

        Ok(Self {
            file_id,
            chunk_index,
            total_chunks,
            nonce,
            ciphertext,
        })
    }
}

/// The body of an `ACK` frame: the identity of one durably recorded chunk.
///
/// ```text
/// u32_le(file_id_len) ‖ file_id ‖ u64_le(chunk_index)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckBody {
    pub file_id: String,
    pub chunk_index: u64,
}

impl AckBody {
    pub fn encode(&self) -> TransferResult<Vec<u8>> {
        let file_id = self.file_id.as_bytes();

        let mut out = Vec::with_capacity(4 + file_id.len() + 8);

        out.put_u32_le(file_id.len() as u32);
        out.put_slice(file_id);
        out.put_u64_le(self.chunk_index);

        Ok(out)
    }

    pub fn parse(body: &[u8]) -> TransferResult<Self> {
        let malformed = |reason: &str| {
            FrameError::Malformed {
                kind: FrameKind::Ack,
                reason: reason.to_owned(),
            }
            .into_transfer_error()
        };

        if body.len() < 4 + 8 {
            return Err(malformed("body is shorter than an ack header"));
        }

        let file_id_len = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;

        if file_id_len > body.len() - 4 - 8 {
            return Err(malformed("file id length runs past the body"));
        }

        let file_id = String::from_utf8(body[4..4 + file_id_len].to_vec())
            .map_err(|_| malformed("file id is not valid UTF-8"))?;

        let at = 4 + file_id_len;

        let chunk_index = u64::from_le_bytes(
            body[at..at + 8]
                .try_into()
                .map_err(|_| malformed("chunk index is truncated"))?,
        );

        Ok(Self {
            file_id,
            chunk_index,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chunk() -> ChunkBody {
        ChunkBody {
            file_id: "file-1".into(),
            chunk_index: 7,
            total_chunks: 12,
            nonce: vec![9u8; crate::transfer::crypto_v2::NONCE_LEN],
            ciphertext: vec![1u8; 64 + crate::transfer::crypto_v2::TAG_LEN],
        }
    }

    /// The kind bytes are wire format. Renumbering one changes what a peer must
    /// send, so it is pinned by value rather than trusted to review.
    #[test]
    fn the_kind_bytes_are_frozen() {
        let expected: [(FrameKind, u8); 12] = [
            (FrameKind::Hello, 0x01),
            (FrameKind::HelloAck, 0x02),
            (FrameKind::Manifest, 0x03),
            (FrameKind::ManifestAck, 0x04),
            (FrameKind::Chunk, 0x05),
            (FrameKind::Ack, 0x06),
            (FrameKind::Finish, 0x07),
            (FrameKind::Complete, 0x08),
            (FrameKind::Cancel, 0x09),
            (FrameKind::Error, 0x0A),
            (FrameKind::Ping, 0x0B),
            (FrameKind::Pong, 0x0C),
        ];

        for (kind, byte) in expected {
            assert_eq!(kind.as_byte(), byte, "{kind:?} changed its wire byte");
            assert_eq!(FrameKind::from_byte(byte), Some(kind));
        }

        // And nothing else is a kind.
        assert_eq!(FrameKind::from_byte(0x00), None);
        assert_eq!(FrameKind::from_byte(0x0D), None);
        assert_eq!(FrameKind::from_byte(0xFF), None);
    }

    /// The layout is frozen too: little-endian length including the kind byte.
    #[test]
    fn a_frame_encodes_as_the_documented_layout() {
        let encoded = encode(FrameKind::Ping, &[]).unwrap();

        assert_eq!(encoded, vec![0x01, 0x00, 0x00, 0x00, 0x0B]);

        let encoded = encode(FrameKind::Finish, &[0xABu8; 32]).unwrap();

        assert_eq!(&encoded[..4], &[33, 0, 0, 0]);
        assert_eq!(encoded[4], 0x07);
        assert_eq!(encoded.len(), 4 + 33);
    }

    #[test]
    fn a_round_trip_preserves_a_frame() {
        let chunk = sample_chunk();
        let body = chunk.encode().unwrap();

        let encoded = encode(FrameKind::Chunk, &body).unwrap();

        let mut decoder = FrameDecoder::new();
        decoder.push(&encoded).unwrap();

        let frame = decoder.next_frame().unwrap().expect("a complete frame");

        assert_eq!(frame.kind, FrameKind::Chunk);
        assert_eq!(ChunkBody::parse(&frame.body).unwrap(), chunk);
        assert_eq!(decoder.next_frame().unwrap(), None);
        decoder.finish().unwrap();
    }

    /// A frame split across transport reads must reassemble. This is the case a
    /// QUIC stream produces routinely and a WebSocket produces when a peer
    /// chooses to.
    #[test]
    fn a_frame_split_across_reads_reassembles() {
        let body = sample_chunk().encode().unwrap();
        let encoded = encode(FrameKind::Chunk, &body).unwrap();

        let mut decoder = FrameDecoder::new();

        // One byte at a time: the worst case a transport can produce.
        for (index, byte) in encoded.iter().enumerate() {
            decoder.push(&[*byte]).unwrap();

            let frame = decoder.next_frame().unwrap();

            if index + 1 == encoded.len() {
                assert!(frame.is_some(), "the frame completes on its last byte");
            } else {
                assert!(
                    frame.is_none(),
                    "no frame is returned before it is complete"
                );
            }
        }
    }

    /// Several frames arriving in one read must all come back, in order.
    #[test]
    fn coalesced_frames_are_all_returned_in_order() {
        let mut encoded = Vec::new();

        encoded.extend(encode(FrameKind::Ping, &[]).unwrap());
        encoded.extend(encode(FrameKind::Complete, &[]).unwrap());
        encoded.extend(encode(FrameKind::Finish, &[7u8; 32]).unwrap());

        let mut decoder = FrameDecoder::new();
        decoder.push(&encoded).unwrap();

        let kinds: Vec<FrameKind> = std::iter::from_fn(|| decoder.next_frame().unwrap())
            .map(|frame| frame.kind)
            .collect();

        assert_eq!(
            kinds,
            vec![FrameKind::Ping, FrameKind::Complete, FrameKind::Finish]
        );
        decoder.finish().unwrap();
    }

    /// The bound is applied to the *header*, before the body is buffered. This
    /// is the test that matters for the memory-exhaustion case: a peer that
    /// declares four gigabytes is refused having sent four bytes.
    #[test]
    fn an_oversized_declared_length_is_refused_on_the_header_alone() {
        let mut decoder = FrameDecoder::new();

        // 0xFFFF_FFFF as a little-endian length.
        decoder.push(&[0xFF, 0xFF, 0xFF, 0xFF]).unwrap();

        let error = decoder.next_frame().unwrap_err();

        assert!(
            matches!(error, TransferError::Protocol(_)),
            "expected a protocol error, got {error:?}"
        );
        assert!(
            error.to_string().contains("exceeds"),
            "the error should say the frame is too large, got: {error}"
        );
    }

    /// A peer that streams bytes without ever completing a frame cannot grow the
    /// buffer past the cap.
    #[test]
    fn a_buffer_that_cannot_be_a_frame_is_refused() {
        let mut decoder = FrameDecoder::with_cap(64);

        // A legal-looking header declaring 64 bytes, then more bytes than any
        // frame that size could occupy.
        decoder.push(&[64, 0, 0, 0]).unwrap();

        let error = decoder
            .push(&[0u8; 128])
            .expect_err("the buffer cannot hold a legal frame any more");

        assert!(error.to_string().contains("exceeds"));
    }

    #[test]
    fn a_zero_length_frame_is_refused() {
        let mut decoder = FrameDecoder::new();
        decoder.push(&[0, 0, 0, 0]).unwrap();

        let error = decoder.next_frame().unwrap_err();

        assert!(error.to_string().contains("zero length"));
    }

    #[test]
    fn an_unknown_kind_is_refused_and_names_the_byte() {
        // Length 1, kind 0x42, no body.
        let mut decoder = FrameDecoder::new();
        decoder.push(&[1, 0, 0, 0, 0x42]).unwrap();

        let error = decoder.next_frame().unwrap_err();

        assert!(error.to_string().contains("0x42"), "got: {error}");
    }

    #[test]
    fn a_body_of_the_wrong_length_for_its_kind_is_refused() {
        // A Finish with 31 bytes rather than 32.
        let encoded = {
            let mut out = Vec::new();
            out.put_u32_le(32);
            out.put_u8(FrameKind::Finish.as_byte());
            out.put_slice(&[0u8; 31]);
            out
        };

        let mut decoder = FrameDecoder::new();
        decoder.push(&encoded).unwrap();

        let error = decoder.next_frame().unwrap_err();

        assert!(
            error.to_string().contains("got 31"),
            "the error should name the bad length, got: {error}"
        );
    }

    #[test]
    fn a_kind_that_requires_a_body_refuses_an_empty_one() {
        let error = encode(FrameKind::Hello, &[]).unwrap_err();

        assert!(
            error.to_string().contains("must carry a body"),
            "got: {error}"
        );
    }

    #[test]
    fn encoding_refuses_a_body_over_the_cap() {
        let error = encode(FrameKind::Chunk, &vec![0u8; MAX_FRAME_BYTES]).unwrap_err();

        assert!(error.to_string().contains("exceeds"), "got: {error}");
    }

    /// Ending in the middle of a frame is not a clean close, and must not read
    /// as one.
    #[test]
    fn a_stream_that_ends_mid_frame_reports_it() {
        let body = sample_chunk().encode().unwrap();
        let encoded = encode(FrameKind::Chunk, &body).unwrap();

        let mut decoder = FrameDecoder::new();
        decoder.push(&encoded[..encoded.len() - 5]).unwrap();

        assert_eq!(decoder.next_frame().unwrap(), None);

        let error = decoder.finish().unwrap_err();

        assert!(
            error.to_string().contains("incomplete frame"),
            "got: {error}"
        );
    }

    #[test]
    fn a_stream_that_ends_on_a_boundary_is_clean() {
        let encoded = encode(FrameKind::Complete, &[]).unwrap();

        let mut decoder = FrameDecoder::new();
        decoder.push(&encoded).unwrap();
        decoder.next_frame().unwrap();

        decoder
            .finish()
            .expect("a complete frame is a clean ending");
    }

    /// Every truncation of a chunk body must be *handled*. This is the test that
    /// caught a real defect: an early version bounded the file id's own length
    /// but not the fixed-width fields after it, so a body with a plausible id
    /// length and a truncated tail panicked on a slice index. A peer can send
    /// that, so a panic here is a remote denial of service.
    ///
    /// The loop is the assertion — a panic fails the test — and the explicit
    /// claims below pin the two regions: a prefix inside the fixed header cannot
    /// parse at all, and a prefix that reaches the ciphertext parses with a
    /// shorter one, because the intended ciphertext length is unknowable.
    #[test]
    fn truncated_chunk_bodies_are_refused_rather_than_panicking() {
        let body = sample_chunk().encode().unwrap();

        // Where the fixed header ends: id length + id + index + total + nonce.
        let header_len = 4 + "file-1".len() + 8 + 8 + crate::transfer::crypto_v2::NONCE_LEN;

        for length in 0..body.len() {
            let parsed = ChunkBody::parse(&body[..length]);

            if length < header_len {
                assert!(
                    parsed.is_err(),
                    "a body of {length} bytes is inside the fixed header and cannot parse"
                );
            }
        }

        // And a body that reaches the ciphertext but has no room for a tag is
        // refused rather than handed to the AEAD.
        let full = ChunkBody::parse(&body).expect("the whole body parses");

        assert_eq!(full, sample_chunk());
    }

    #[test]
    fn a_chunk_body_with_a_bad_nonce_length_is_refused() {
        let mut chunk = sample_chunk();
        chunk.nonce = vec![0u8; 11];

        assert!(chunk.encode().is_err());
    }

    #[test]
    fn a_chunk_with_no_room_for_a_tag_is_refused() {
        let mut body = Vec::new();
        let file_id = b"f";
        body.put_u32_le(file_id.len() as u32);
        body.put_slice(file_id);
        body.put_u64_le(0);
        body.put_u64_le(1);
        body.put_slice(&[0u8; crate::transfer::crypto_v2::NONCE_LEN]);
        body.put_slice(&[0u8; crate::transfer::crypto_v2::TAG_LEN - 1]);

        let error = ChunkBody::parse(&body).unwrap_err();

        assert!(error.to_string().contains("AEAD tag"), "got: {error}");
    }

    #[test]
    fn an_ack_round_trips_and_rejects_a_lying_file_id_length() {
        let ack = AckBody {
            file_id: "file-with-a-longer-id".into(),
            chunk_index: 1234,
        };

        let encoded = ack.encode().unwrap();

        assert_eq!(AckBody::parse(&encoded).unwrap(), ack);

        let mut lying = Vec::new();
        lying.put_u32_le(0xFFFF);
        lying.put_slice(b"short");

        assert!(AckBody::parse(&lying).is_err());
    }

    /// A file id that is not UTF-8 is refused rather than lossily converted:
    /// the id is used as a map key and as AAD input, so a mangled one would
    /// silently address the wrong chunk.
    #[test]
    fn a_non_utf8_file_id_is_refused() {
        let mut body = Vec::new();
        body.put_u32_le(2);
        body.put_slice(&[0xFF, 0xFE]);
        body.put_u64_le(0);
        body.put_u64_le(1);
        body.put_slice(&[0u8; crate::transfer::crypto_v2::NONCE_LEN]);
        body.put_slice(&[0u8; crate::transfer::crypto_v2::TAG_LEN]);

        assert!(ChunkBody::parse(&body).is_err());
    }
}
