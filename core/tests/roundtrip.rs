//! Property tests: decode(encode(frame)) == frame for every message type, and
//! the decoder never panics on arbitrary or corrupted input.

use beam_core::frame::Frame;
use beam_core::messages::*;
use proptest::prelude::*;
use proptest::strategy::Union;
use uuid::Uuid;

fn text() -> impl Strategy<Value = String> {
    ".{0,40}"
}

fn uuid() -> impl Strategy<Value = Uuid> {
    any::<u128>().prop_map(Uuid::from_u128)
}

fn hash() -> impl Strategy<Value = Hash32> {
    any::<[u8; 32]>().prop_map(Hash32)
}

fn device_type() -> impl Strategy<Value = DeviceType> {
    prop_oneof![Just(DeviceType::Phone), Just(DeviceType::Laptop)]
}

fn file_entry() -> impl Strategy<Value = FileEntry> {
    (any::<u32>(), text(), any::<u64>(), text(), any::<i64>()).prop_map(
        |(file_id, name, size, mime, modified_unix)| FileEntry {
            file_id,
            name,
            size,
            mime,
            modified_unix,
        },
    )
}

fn frame() -> impl Strategy<Value = Frame> {
    Union::new(vec![
        (
            any::<(u16, u16)>(),
            any::<[u8; 8]>(),
            text(),
            device_type(),
            prop::collection::vec(text(), 0..4),
        )
            .prop_map(|((major, minor), id, name, device_type, capabilities)| {
                Frame::Hello(Hello {
                    version: ProtocolVersion { major, minor },
                    device_id: DeviceId(id),
                    name,
                    device_type,
                    capabilities,
                })
            })
            .boxed(),
        (
            uuid(),
            prop::collection::vec(file_entry(), 0..20),
            any::<u64>(),
        )
            .prop_map(|(transfer_id, files, total_size)| {
                Frame::Offer(Offer {
                    transfer_id,
                    files,
                    total_size,
                })
            })
            .boxed(),
        (
            uuid(),
            prop::collection::vec(
                (any::<u32>(), prop::collection::vec(any::<u8>(), 0..64)),
                0..5,
            ),
        )
            .prop_map(|(transfer_id, have)| {
                Frame::Accept(Accept {
                    transfer_id,
                    have: have
                        .into_iter()
                        .map(|(file_id, chunk_bitmap)| FileHave {
                            file_id,
                            chunk_bitmap,
                        })
                        .collect(),
                })
            })
            .boxed(),
        (uuid(), text())
            .prop_map(|(transfer_id, reason)| {
                Frame::Reject(Reject {
                    transfer_id,
                    reason,
                })
            })
            .boxed(),
        (uuid(), any::<[u8; 32]>(), any::<u16>())
            .prop_map(|(transfer_id, session_token, stream_index)| {
                Frame::StreamOpen(StreamOpen {
                    transfer_id,
                    session_token,
                    stream_index,
                })
            })
            .boxed(),
        (
            any::<u32>(),
            any::<u64>(),
            prop::collection::vec(any::<u8>(), 0..2048),
        )
            .prop_map(|(file_id, offset, bytes)| {
                Frame::Data(Data {
                    file_id,
                    offset,
                    bytes,
                })
            })
            .boxed(),
        (any::<u32>(), any::<u64>(), hash())
            .prop_map(|(file_id, chunk_index, blake3)| {
                Frame::ChunkHash(ChunkHash {
                    file_id,
                    chunk_index,
                    blake3,
                })
            })
            .boxed(),
        (any::<u32>(), prop::collection::vec(any::<u64>(), 0..50))
            .prop_map(|(file_id, chunk_indices)| {
                Frame::ChunkAck(ChunkAck {
                    file_id,
                    chunk_indices,
                })
            })
            .boxed(),
        (any::<u32>(), any::<u64>())
            .prop_map(|(file_id, chunk_index)| {
                Frame::ChunkRetry(ChunkRetry {
                    file_id,
                    chunk_index,
                })
            })
            .boxed(),
        (any::<u32>(), hash())
            .prop_map(|(file_id, blake3_full)| {
                Frame::FileDone(FileDone {
                    file_id,
                    blake3_full,
                })
            })
            .boxed(),
        (uuid(), any::<(u64, u32, u64)>())
            .prop_map(|(transfer_id, (bytes, files, duration_ms))| {
                Frame::TransferComplete(TransferComplete {
                    transfer_id,
                    stats: TransferStats {
                        bytes,
                        files,
                        duration_ms,
                    },
                })
            })
            .boxed(),
        (uuid(), text())
            .prop_map(|(transfer_id, reason)| {
                Frame::Cancel(Cancel {
                    transfer_id,
                    reason,
                })
            })
            .boxed(),
        any::<u64>()
            .prop_map(|nonce| Frame::Ping(Ping { nonce }))
            .boxed(),
        any::<u64>()
            .prop_map(|nonce| Frame::Pong(Ping { nonce }))
            .boxed(),
        any::<[u8; 16]>()
            .prop_map(|token| Frame::PairRequest(PairRequest { token }))
            .boxed(),
        (any::<bool>(), text())
            .prop_map(|(accepted, reason)| Frame::PairResult(PairResult { accepted, reason }))
            .boxed(),
        (text(), ".{0,500}")
            .prop_map(|(mime, text)| Frame::Clipboard(Clipboard { mime, text }))
            .boxed(),
    ])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2_000))]

    #[test]
    fn every_frame_round_trips(frame in frame()) {
        let bytes = frame.to_bytes().unwrap();
        prop_assert_eq!(bytes[0], frame.frame_type());
        prop_assert_eq!(Frame::decode(&bytes).unwrap(), frame);
    }

    #[test]
    fn decoding_random_bytes_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = Frame::decode(&bytes);
    }

    #[test]
    fn decoding_corrupted_frames_never_panics(
        frame in frame(),
        position in any::<prop::sample::Index>(),
        flip in 1u8..=255,
        cut in any::<prop::sample::Index>(),
    ) {
        let mut bytes = frame.to_bytes().unwrap();
        let i = position.index(bytes.len());
        bytes[i] ^= flip;
        let _ = Frame::decode(&bytes);
        let len = cut.index(bytes.len() + 1);
        let _ = Frame::decode(&bytes[..len]);
    }
}
