//! One incoming data connection. A chunk arrives as consecutive DATA frames
//! on a single stream, followed by its CHUNK_HASH. The chunk is buffered and
//! hashed as it arrives; only a verified chunk is handed to the writer, so
//! nothing unverified ever reaches the disk.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::AsyncRead;
use tokio::sync::mpsc;

use super::writer::WriteJob;
use crate::connection::{ConnectionError, FrameReader};
use crate::frame::Frame;
use crate::transfer::{chunk_count, chunk_end, TransferError, CHUNK_SIZE};

/// What a stream needs to know about the files of its transfer.
pub(super) struct StreamContext {
    pub sizes: Vec<u64>,
    pub index_by_id: HashMap<u32, usize>,
}

pub(super) enum StreamEvent {
    /// A chunk failed its hash; ask the sender for it again.
    Corrupt {
        file: usize,
        chunk: u64,
    },
    Failed(TransferError),
}

struct PendingChunk {
    file: usize,
    file_id: u32,
    chunk: u64,
    start: u64,
    end: u64,
    data: Vec<u8>,
    hasher: blake3::Hasher,
}

pub(super) async fn run<R: AsyncRead + Unpin>(
    mut reader: FrameReader<R>,
    context: Arc<StreamContext>,
    writer: mpsc::Sender<WriteJob>,
    events: mpsc::UnboundedSender<StreamEvent>,
) {
    if let Err(err) = receive(&mut reader, &context, &writer, &events).await {
        let _ = events.send(StreamEvent::Failed(err));
    }
}

async fn receive<R: AsyncRead + Unpin>(
    reader: &mut FrameReader<R>,
    context: &StreamContext,
    writer: &mpsc::Sender<WriteJob>,
    events: &mpsc::UnboundedSender<StreamEvent>,
) -> Result<(), TransferError> {
    let mut pending: Option<PendingChunk> = None;
    loop {
        let frame = match reader.read_frame().await {
            Ok(frame) => frame,
            // The sender closes data streams when it has nothing left to send.
            Err(ConnectionError::Closed) if pending.is_none() => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        match frame {
            Frame::Data(data) => {
                let file = *context.index_by_id.get(&data.file_id).ok_or_else(|| {
                    TransferError::Protocol(format!("DATA for unknown file_id {}", data.file_id))
                })?;
                let size = context.sizes[file];
                if pending.is_none() {
                    // A new chunk must start at a chunk boundary.
                    let chunk = data.offset / CHUNK_SIZE;
                    if data.offset % CHUNK_SIZE != 0 || chunk >= chunk_count(size) {
                        return Err(TransferError::Protocol(format!(
                            "DATA for file {} at offset {} does not start a chunk",
                            data.file_id, data.offset
                        )));
                    }
                    let end = chunk_end(chunk, size);
                    pending = Some(PendingChunk {
                        file,
                        file_id: data.file_id,
                        chunk,
                        start: data.offset,
                        end,
                        data: Vec::with_capacity((end - data.offset) as usize),
                        hasher: blake3::Hasher::new(),
                    });
                }
                let current = pending.as_mut().expect("set above");
                let expected = current.start + current.data.len() as u64;
                if data.file_id != current.file_id
                    || data.offset != expected
                    || data.bytes.is_empty()
                    || expected + data.bytes.len() as u64 > current.end
                {
                    return Err(TransferError::Protocol(format!(
                        "DATA for file {} at offset {} does not continue chunk {} of file {}",
                        data.file_id, data.offset, current.chunk, current.file_id
                    )));
                }
                current.hasher.update(&data.bytes);
                current.data.extend_from_slice(&data.bytes);
            }
            Frame::ChunkHash(hash) => {
                let current = pending.take().ok_or_else(|| {
                    TransferError::Protocol("CHUNK_HASH without DATA".to_string())
                })?;
                if hash.file_id != current.file_id
                    || hash.chunk_index != current.chunk
                    || current.start + current.data.len() as u64 != current.end
                {
                    return Err(TransferError::Protocol(format!(
                        "CHUNK_HASH for chunk {} of file {} does not match the data received",
                        hash.chunk_index, hash.file_id
                    )));
                }
                if current.hasher.finalize().as_bytes() == &hash.blake3.0 {
                    let job = WriteJob::Chunk {
                        file: current.file,
                        chunk: current.chunk,
                        offset: current.start,
                        data: current.data,
                        hash: hash.blake3,
                    };
                    if writer.send(job).await.is_err() {
                        // The transfer is shutting down.
                        return Ok(());
                    }
                } else {
                    let _ = events.send(StreamEvent::Corrupt {
                        file: current.file,
                        chunk: current.chunk,
                    });
                }
            }
            other => return Err(TransferError::unexpected("DATA or CHUNK_HASH", &other)),
        }
    }
}
