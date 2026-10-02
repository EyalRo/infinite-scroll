//! Wire protocol, version 1. Normative description: `docs/ble-protocol.md`.
//!
//! Everything in this module is transport-independent so it can be tested
//! without a Bluetooth adapter. The GATT glue lives in `gatt.rs`.

use bluer::Uuid;
use serde_json::{json, Value};

pub const PROTOCOL_VERSION: u64 = 1;

pub const SERVICE_UUID: Uuid = Uuid::from_u128(0xa4f6_0001_6b8e_4c1f_9d0a_5e1f_0c1a_1001);
pub const INFO_UUID: Uuid = Uuid::from_u128(0xa4f6_0002_6b8e_4c1f_9d0a_5e1f_0c1a_1001);
pub const REQUEST_UUID: Uuid = Uuid::from_u128(0xa4f6_0003_6b8e_4c1f_9d0a_5e1f_0c1a_1001);
pub const RESPONSE_UUID: Uuid = Uuid::from_u128(0xa4f6_0004_6b8e_4c1f_9d0a_5e1f_0c1a_1001);
pub const CHANGED_UUID: Uuid = Uuid::from_u128(0xa4f6_0005_6b8e_4c1f_9d0a_5e1f_0c1a_1001);
pub const UPLOAD_UUID: Uuid = Uuid::from_u128(0xa4f6_0006_6b8e_4c1f_9d0a_5e1f_0c1a_1001);

/// Frame header: flags (1) + message id (2, little endian).
pub const FRAME_HEADER_LEN: usize = 3;
pub const FLAG_FIRST: u8 = 0b01;
pub const FLAG_LAST: u8 = 0b10;
/// Largest reassembled request or response body. Library pages are bounded
/// well below this; anything larger is a malformed or hostile peer.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// Upload data header: session id (2, LE) + byte offset (4, LE).
pub const UPLOAD_HEADER_LEN: usize = 6;

/// Bits of the `Changed` characteristic's domain mask.
pub const DOMAIN_STATUS: u16 = 1 << 0;
pub const DOMAIN_LIBRARY: u16 = 1 << 1;
pub const DOMAIN_PRINTER: u16 = 1 << 2;
pub const DOMAIN_SCHEDULER: u16 = 1 << 3;

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    TooShort,
    /// A continuation frame arrived with no message in progress, or for a
    /// different message id than the one in progress.
    Unexpected,
    TooLarge,
}

pub fn encode_frames(msg_id: u16, body: &[u8], chunk_payload: usize) -> Vec<Vec<u8>> {
    let chunk_payload = chunk_payload.max(1);
    let mut frames = Vec::new();
    let mut offset = 0;
    loop {
        let end = (offset + chunk_payload).min(body.len());
        let mut flags = 0;
        if offset == 0 {
            flags |= FLAG_FIRST;
        }
        if end == body.len() {
            flags |= FLAG_LAST;
        }
        let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + end - offset);
        frame.push(flags);
        frame.extend_from_slice(&msg_id.to_le_bytes());
        frame.extend_from_slice(&body[offset..end]);
        frames.push(frame);
        if end == body.len() {
            return frames;
        }
        offset = end;
    }
}

#[derive(Default)]
pub struct Reassembler {
    msg_id: Option<u16>,
    body: Vec<u8>,
}

impl Reassembler {
    /// Feeds one frame; returns the complete `(message id, body)` when the
    /// frame carries the LAST flag. A FIRST frame always restarts assembly,
    /// so a message abandoned half-way by a dropped connection cannot
    /// poison the next one.
    pub fn push(&mut self, frame: &[u8]) -> Result<Option<(u16, Vec<u8>)>, FrameError> {
        if frame.len() < FRAME_HEADER_LEN {
            return Err(FrameError::TooShort);
        }
        let flags = frame[0];
        let msg_id = u16::from_le_bytes([frame[1], frame[2]]);
        let payload = &frame[FRAME_HEADER_LEN..];
        if flags & FLAG_FIRST != 0 {
            self.body.clear();
            self.msg_id = Some(msg_id);
        } else if self.msg_id != Some(msg_id) {
            self.msg_id = None;
            self.body.clear();
            return Err(FrameError::Unexpected);
        }
        if self.body.len() + payload.len() > MAX_MESSAGE_BYTES {
            self.msg_id = None;
            self.body.clear();
            return Err(FrameError::TooLarge);
        }
        self.body.extend_from_slice(payload);
        if flags & FLAG_LAST != 0 {
            self.msg_id = None;
            return Ok(Some((msg_id, std::mem::take(&mut self.body))));
        }
        Ok(None)
    }
}

pub struct UploadChunk<'a> {
    pub session: u16,
    pub offset: u32,
    pub data: &'a [u8],
}

pub fn parse_upload_chunk(bytes: &[u8]) -> Option<UploadChunk<'_>> {
    if bytes.len() <= UPLOAD_HEADER_LEN {
        return None;
    }
    Some(UploadChunk {
        session: u16::from_le_bytes([bytes[0], bytes[1]]),
        offset: u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
        data: &bytes[UPLOAD_HEADER_LEN..],
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    BadRequest,
    UnsupportedVersion,
    UnknownOp,
    InvalidArgument,
    NotFound,
    Conflict,
    /// A backing service (printer/uploader) is not reachable.
    Unavailable,
    /// A backing service answered with an unexpected failure.
    BackendError,
    UploadIncomplete,
    /// The operation or setting is not available on this installation.
    Unsupported,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::BadRequest => "bad_request",
            ErrorCode::UnsupportedVersion => "unsupported_version",
            ErrorCode::UnknownOp => "unknown_op",
            ErrorCode::InvalidArgument => "invalid_argument",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Conflict => "conflict",
            ErrorCode::Unavailable => "unavailable",
            ErrorCode::BackendError => "backend_error",
            ErrorCode::UploadIncomplete => "upload_incomplete",
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::Internal => "internal",
        }
    }
}

/// Whether the Pi finished the work or merely took ownership of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// The operation is finished; the result is final.
    Completed,
    /// The Pi has durably accepted the operation and will carry it out on
    /// its own. The result describes the pending work (e.g. a job).
    Accepted,
}

pub fn ok_response(id: u64, disposition: Disposition, result: Value) -> Value {
    json!({
        "v": PROTOCOL_VERSION,
        "id": id,
        "ok": true,
        "disposition": match disposition {
            Disposition::Completed => "completed",
            Disposition::Accepted => "accepted",
        },
        "result": result,
    })
}

pub fn error_response(id: u64, code: ErrorCode, message: &str, details: Option<Value>) -> Value {
    let mut error = json!({"code": code.as_str(), "message": message});
    if let Some(details) = details {
        error["details"] = details;
    }
    json!({"v": PROTOCOL_VERSION, "id": id, "ok": false, "error": error})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_frame_round_trip() {
        let frames = encode_frames(7, b"hello", 100);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0][0], FLAG_FIRST | FLAG_LAST);
        let mut r = Reassembler::default();
        assert_eq!(r.push(&frames[0]).unwrap(), Some((7, b"hello".to_vec())));
    }

    #[test]
    fn multi_frame_round_trip_with_odd_chunking() {
        let body: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let frames = encode_frames(0xBEEF, &body, 17);
        assert!(frames.len() > 50);
        assert!(frames.iter().all(|f| f.len() <= FRAME_HEADER_LEN + 17));
        let mut r = Reassembler::default();
        let mut out = None;
        for f in &frames {
            out = r.push(f).unwrap();
        }
        assert_eq!(out, Some((0xBEEF, body)));
    }

    #[test]
    fn empty_body_is_one_frame() {
        let frames = encode_frames(1, b"", 10);
        assert_eq!(frames.len(), 1);
        let mut r = Reassembler::default();
        assert_eq!(r.push(&frames[0]).unwrap(), Some((1, vec![])));
    }

    #[test]
    fn first_frame_restarts_an_abandoned_message() {
        let mut r = Reassembler::default();
        let abandoned = encode_frames(1, b"aaaaaaaaaa", 4);
        r.push(&abandoned[0]).unwrap();
        let fresh = encode_frames(2, b"bb", 4);
        assert_eq!(r.push(&fresh[0]).unwrap(), Some((2, b"bb".to_vec())));
    }

    #[test]
    fn continuation_without_a_start_is_rejected() {
        let mut r = Reassembler::default();
        let frames = encode_frames(1, b"aaaaaaaa", 4);
        assert_eq!(r.push(&frames[1]), Err(FrameError::Unexpected));
    }

    #[test]
    fn continuation_for_another_message_id_is_rejected() {
        let mut r = Reassembler::default();
        r.push(&encode_frames(1, b"aaaaaaaa", 4)[0]).unwrap();
        assert_eq!(r.push(&encode_frames(2, b"bbbbbbbb", 4)[1]), Err(FrameError::Unexpected));
    }

    #[test]
    fn oversized_messages_are_rejected() {
        let mut r = Reassembler::default();
        let big = vec![0u8; MAX_MESSAGE_BYTES + 1];
        let frames = encode_frames(1, &big, 500);
        let result = frames.iter().map(|f| r.push(f)).find(|res| res.is_err());
        assert_eq!(result, Some(Err(FrameError::TooLarge)));
    }

    #[test]
    fn short_frames_are_rejected() {
        assert_eq!(Reassembler::default().push(&[1, 2]), Err(FrameError::TooShort));
    }

    #[test]
    fn upload_chunk_parses_header_and_data() {
        let bytes = [0x02, 0x00, 0x10, 0x00, 0x00, 0x00, 9, 8, 7];
        let chunk = parse_upload_chunk(&bytes).unwrap();
        assert_eq!((chunk.session, chunk.offset, chunk.data), (2, 16, &[9u8, 8, 7][..]));
        assert!(parse_upload_chunk(&bytes[..6]).is_none());
    }
}
