//! Frame encoding (SPEC.md section 7.1): `frame = type:u8 | body`.
//!
//! Control bodies are CBOR. DATA uses a fixed big-endian header
//! `file_id:u32 | offset:u64 | len:u32` followed by `len` bytes, so the hot
//! path never touches CBOR. Decoding never panics on hostile input.

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::messages::*;

pub const TYPE_HELLO: u8 = 0x01;
pub const TYPE_OFFER: u8 = 0x02;
pub const TYPE_ACCEPT: u8 = 0x03;
pub const TYPE_REJECT: u8 = 0x04;
pub const TYPE_STREAM_OPEN: u8 = 0x05;
pub const TYPE_DATA: u8 = 0x06;
pub const TYPE_CHUNK_HASH: u8 = 0x07;
pub const TYPE_CHUNK_ACK: u8 = 0x08;
pub const TYPE_FILE_DONE: u8 = 0x09;
pub const TYPE_TRANSFER_COMPLETE: u8 = 0x0A;
pub const TYPE_CANCEL: u8 = 0x0B;
pub const TYPE_PING: u8 = 0x0C;
pub const TYPE_PONG: u8 = 0x0D;
pub const TYPE_CLIPBOARD: u8 = 0x10;

/// Size of the DATA header after the type byte.
pub const DATA_HEADER_LEN: usize = 4 + 8 + 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Hello(Hello),
    Offer(Offer),
    Accept(Accept),
    Reject(Reject),
    StreamOpen(StreamOpen),
    Data(Data),
    ChunkHash(ChunkHash),
    ChunkAck(ChunkAck),
    FileDone(FileDone),
    TransferComplete(TransferComplete),
    Cancel(Cancel),
    Ping(Ping),
    Pong(Ping),
    Clipboard(Clipboard),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("empty frame")]
    Empty,
    #[error("unknown frame type 0x{0:02x}")]
    UnknownType(u8),
    #[error("invalid CBOR body for frame type 0x{frame_type:02x}: {message}")]
    Cbor { frame_type: u8, message: String },
    #[error("{0} unexpected bytes after the CBOR body")]
    TrailingBytes(usize),
    #[error("DATA frame is shorter than its header")]
    DataTruncated,
    #[error("DATA length field says {declared} bytes but {actual} follow")]
    DataLengthMismatch { declared: u32, actual: usize },
    #[error("DATA payload of {0} bytes does not fit the u32 length field")]
    DataTooLong(usize),
}

impl Frame {
    pub fn frame_type(&self) -> u8 {
        match self {
            Frame::Hello(_) => TYPE_HELLO,
            Frame::Offer(_) => TYPE_OFFER,
            Frame::Accept(_) => TYPE_ACCEPT,
            Frame::Reject(_) => TYPE_REJECT,
            Frame::StreamOpen(_) => TYPE_STREAM_OPEN,
            Frame::Data(_) => TYPE_DATA,
            Frame::ChunkHash(_) => TYPE_CHUNK_HASH,
            Frame::ChunkAck(_) => TYPE_CHUNK_ACK,
            Frame::FileDone(_) => TYPE_FILE_DONE,
            Frame::TransferComplete(_) => TYPE_TRANSFER_COMPLETE,
            Frame::Cancel(_) => TYPE_CANCEL,
            Frame::Ping(_) => TYPE_PING,
            Frame::Pong(_) => TYPE_PONG,
            Frame::Clipboard(_) => TYPE_CLIPBOARD,
        }
    }

    /// Short human-readable name, used in protocol error messages.
    pub fn name(&self) -> &'static str {
        match self {
            Frame::Hello(_) => "HELLO",
            Frame::Offer(_) => "OFFER",
            Frame::Accept(_) => "ACCEPT",
            Frame::Reject(_) => "REJECT",
            Frame::StreamOpen(_) => "STREAM_OPEN",
            Frame::Data(_) => "DATA",
            Frame::ChunkHash(_) => "CHUNK_HASH",
            Frame::ChunkAck(_) => "CHUNK_ACK",
            Frame::FileDone(_) => "FILE_DONE",
            Frame::TransferComplete(_) => "TRANSFER_COMPLETE",
            Frame::Cancel(_) => "CANCEL",
            Frame::Ping(_) => "PING",
            Frame::Pong(_) => "PONG",
            Frame::Clipboard(_) => "CLIPBOARD",
        }
    }

    /// Appends the encoded frame (type byte + body) to `out`.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), FrameError> {
        if let Frame::Data(data) = self {
            return encode_data(data.file_id, data.offset, &data.bytes, out);
        }
        out.push(self.frame_type());
        match self {
            Frame::Hello(m) => encode_cbor(m, out),
            Frame::Offer(m) => encode_cbor(m, out),
            Frame::Accept(m) => encode_cbor(m, out),
            Frame::Reject(m) => encode_cbor(m, out),
            Frame::StreamOpen(m) => encode_cbor(m, out),
            Frame::ChunkHash(m) => encode_cbor(m, out),
            Frame::ChunkAck(m) => encode_cbor(m, out),
            Frame::FileDone(m) => encode_cbor(m, out),
            Frame::TransferComplete(m) => encode_cbor(m, out),
            Frame::Cancel(m) => encode_cbor(m, out),
            Frame::Ping(m) | Frame::Pong(m) => encode_cbor(m, out),
            Frame::Clipboard(m) => encode_cbor(m, out),
            Frame::Data(_) => unreachable!("handled above"),
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, FrameError> {
        let mut out = Vec::new();
        self.encode(&mut out)?;
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Frame, FrameError> {
        let (&frame_type, body) = bytes.split_first().ok_or(FrameError::Empty)?;
        Ok(match frame_type {
            TYPE_HELLO => Frame::Hello(decode_cbor(frame_type, body)?),
            TYPE_OFFER => Frame::Offer(decode_cbor(frame_type, body)?),
            TYPE_ACCEPT => Frame::Accept(decode_cbor(frame_type, body)?),
            TYPE_REJECT => Frame::Reject(decode_cbor(frame_type, body)?),
            TYPE_STREAM_OPEN => Frame::StreamOpen(decode_cbor(frame_type, body)?),
            TYPE_DATA => Frame::Data(decode_data(body)?),
            TYPE_CHUNK_HASH => Frame::ChunkHash(decode_cbor(frame_type, body)?),
            TYPE_CHUNK_ACK => Frame::ChunkAck(decode_cbor(frame_type, body)?),
            TYPE_FILE_DONE => Frame::FileDone(decode_cbor(frame_type, body)?),
            TYPE_TRANSFER_COMPLETE => Frame::TransferComplete(decode_cbor(frame_type, body)?),
            TYPE_CANCEL => Frame::Cancel(decode_cbor(frame_type, body)?),
            TYPE_PING => Frame::Ping(decode_cbor(frame_type, body)?),
            TYPE_PONG => Frame::Pong(decode_cbor(frame_type, body)?),
            TYPE_CLIPBOARD => Frame::Clipboard(decode_cbor(frame_type, body)?),
            other => return Err(FrameError::UnknownType(other)),
        })
    }
}

/// Encodes a DATA frame straight from a borrowed buffer, so the sender does
/// not have to copy file bytes into a [`Data`] first.
pub fn encode_data(
    file_id: u32,
    offset: u64,
    bytes: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), FrameError> {
    let len = u32::try_from(bytes.len()).map_err(|_| FrameError::DataTooLong(bytes.len()))?;
    out.reserve(1 + DATA_HEADER_LEN + bytes.len());
    out.push(TYPE_DATA);
    out.extend_from_slice(&file_id.to_be_bytes());
    out.extend_from_slice(&offset.to_be_bytes());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn decode_data(body: &[u8]) -> Result<Data, FrameError> {
    if body.len() < DATA_HEADER_LEN {
        return Err(FrameError::DataTruncated);
    }
    let (header, payload) = body.split_at(DATA_HEADER_LEN);
    let file_id = u32::from_be_bytes(header[0..4].try_into().expect("4 bytes"));
    let offset = u64::from_be_bytes(header[4..12].try_into().expect("8 bytes"));
    let declared = u32::from_be_bytes(header[12..16].try_into().expect("4 bytes"));
    if declared as usize != payload.len() {
        return Err(FrameError::DataLengthMismatch {
            declared,
            actual: payload.len(),
        });
    }
    Ok(Data {
        file_id,
        offset,
        bytes: payload.to_vec(),
    })
}

fn encode_cbor<T: Serialize>(value: &T, out: &mut Vec<u8>) {
    // Writing into a Vec cannot fail, and every message type serializes cleanly.
    ciborium::into_writer(value, out).expect("CBOR encoding into a Vec cannot fail");
}

fn decode_cbor<T: DeserializeOwned>(frame_type: u8, body: &[u8]) -> Result<T, FrameError> {
    let mut rest = body;
    let value = ciborium::from_reader(&mut rest).map_err(|e| FrameError::Cbor {
        frame_type,
        message: e.to_string(),
    })?;
    if !rest.is_empty() {
        return Err(FrameError::TrailingBytes(rest.len()));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_frame(declared_len: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![TYPE_DATA];
        bytes.extend_from_slice(&7u32.to_be_bytes());
        bytes.extend_from_slice(&1024u64.to_be_bytes());
        bytes.extend_from_slice(&declared_len.to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn data_frame_layout_is_fixed_big_endian() {
        let frame = Frame::Data(Data {
            file_id: 7,
            offset: 1024,
            bytes: b"abc".to_vec(),
        });
        assert_eq!(frame.to_bytes().unwrap(), data_frame(3, b"abc"));
    }

    #[test]
    fn data_frame_with_wrong_length_is_rejected() {
        assert_eq!(
            Frame::decode(&data_frame(4, b"abc")),
            Err(FrameError::DataLengthMismatch {
                declared: 4,
                actual: 3
            })
        );
        assert_eq!(
            Frame::decode(&data_frame(u32::MAX, b"")),
            Err(FrameError::DataLengthMismatch {
                declared: u32::MAX,
                actual: 0
            })
        );
    }

    #[test]
    fn truncated_data_header_is_rejected() {
        assert_eq!(
            Frame::decode(&[TYPE_DATA, 0, 0]),
            Err(FrameError::DataTruncated)
        );
    }

    #[test]
    fn empty_and_unknown_frames_are_rejected() {
        assert_eq!(Frame::decode(&[]), Err(FrameError::Empty));
        assert_eq!(Frame::decode(&[0xEE]), Err(FrameError::UnknownType(0xEE)));
    }

    /// Pins the control-message encoding quoted in docs/PROTOCOL.md: a CBOR
    /// map with text keys.
    #[test]
    fn ping_wire_bytes_match_the_protocol_doc() {
        let bytes = Frame::Ping(Ping { nonce: 1 }).to_bytes().unwrap();
        assert_eq!(
            bytes,
            [0x0C, 0xA1, 0x65, b'n', b'o', b'n', b'c', b'e', 0x01]
        );
    }

    #[test]
    fn trailing_bytes_after_cbor_are_rejected() {
        let mut bytes = Frame::Ping(Ping { nonce: 1 }).to_bytes().unwrap();
        bytes.push(0);
        assert_eq!(Frame::decode(&bytes), Err(FrameError::TrailingBytes(1)));
    }

    #[test]
    fn garbage_cbor_is_an_error_not_a_panic() {
        for body in [
            &[][..],
            &[0xff],
            &[0xbf, 0x61],
            &[0x9b, 0xff, 0xff, 0xff, 0xff],
        ] {
            let mut bytes = vec![TYPE_OFFER];
            bytes.extend_from_slice(body);
            assert!(matches!(
                Frame::decode(&bytes),
                Err(FrameError::Cbor { .. })
            ));
        }
    }

    #[test]
    fn device_id_displays_as_four_hex_groups() {
        let id = DeviceId([0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
        assert_eq!(id.to_string(), "0123-4567-89ab-cdef");
    }
}
