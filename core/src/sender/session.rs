//! One sender session: a control connection plus N data connections.
//!
//! - Control: HELLO, OFFER, ACCEPT; then CHUNK_ACK / CHUNK_RETRY / PING from
//!   the receiver, and FILE_DONE / PING from us.
//! - Data: each stream takes the next chunk from the shared queue and sends
//!   it as DATA frames followed by CHUNK_HASH. A chunk never spans streams.
//! - Flow control: a byte window limits unacknowledged data; each CHUNK_ACK
//!   frees room for more.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::io::WriteHalf;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;

use super::{
    cancelled, ChunkQueue, ChunkRef, FullHashes, SendEvent, SendOptions, SessionHistory,
    TransferPlan,
};
use crate::bitmap::ChunkBitmap;
use crate::connection::{ConnectionError, FrameWriter, DATA_PAYLOAD_LEN};
use crate::frame::Frame;
use crate::messages::{Cancel, ChunkHash, FileDone, Hash32, Offer, Ping, StreamOpen};
use crate::secure::{secure_connection, Role};
use crate::transfer::{
    chunk_end, open_control, spawn_control_reader, LocalDevice, Progress, TransferError,
    CHUNK_SIZE, MAX_CHUNK_RETRIES, MAX_STREAMS, PING_INTERVAL,
};
use crate::trust::TrustStore;

/// Unacknowledged data allowed per stream (four full chunks).
const WINDOW_PER_STREAM_KIB: usize = 16 * 1024;
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// How long to wait for the receiver's user to accept.
const ACCEPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

pub(super) enum SessionEnd {
    Completed,
    Rejected(String),
}

enum StreamEvent {
    ChunkSent {
        chunk: ChunkRef,
        bytes: u64,
        hash: Hash32,
    },
    Failed(TransferError),
}

/// Chunks sent but not yet acknowledged, with the window space each holds.
type InFlight = Arc<Mutex<HashMap<ChunkRef, u32>>>;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    addr: SocketAddr,
    me: &LocalDevice,
    trust: &TrustStore,
    plan: &Arc<TransferPlan>,
    hashes: &Arc<FullHashes>,
    options: &SendOptions,
    history: &mut SessionHistory,
    on_event: &mut impl FnMut(SendEvent),
) -> Result<SessionEnd, TransferError> {
    let started = Instant::now();
    let (mut conn, peer, handshake) = open_control(connect(addr).await?, me, trust).await?;
    if let Some(expected) = options.expect_device {
        if peer.device_id() != expected {
            return Err(TransferError::UnexpectedPeer {
                expected,
                actual: peer.device_id(),
            });
        }
    }
    if history.pinned_key.is_some_and(|key| key != peer.public_key) {
        return Err(TransferError::PeerChanged);
    }
    history.setup.get_or_insert(started.elapsed());
    if !history.accepted_once {
        on_event(SendEvent::Connected(peer.clone()));
    }

    conn.send(&Frame::Offer(Offer {
        transfer_id: plan.transfer_id,
        files: plan.files.iter().map(|f| f.entry.clone()).collect(),
        total_size: plan.total_size,
    }))
    .await?;
    let mut cancel = options.cancel.clone();
    let accept = loop {
        let frame = tokio::select! {
            frame = tokio::time::timeout(ACCEPT_TIMEOUT, conn.read_frame()) => frame
                .map_err(|_| ConnectionError::Timeout("receiver to accept"))??,
            _ = cancelled(&mut cancel) => {
                let _ = conn.send(&Frame::Cancel(Cancel {
                    transfer_id: plan.transfer_id,
                    reason: "cancelled by the sender".into(),
                })).await;
                return Err(TransferError::CancelledByUser);
            }
        };
        match frame {
            Frame::Accept(accept) if accept.transfer_id == plan.transfer_id => break accept,
            Frame::Reject(reject) if reject.transfer_id == plan.transfer_id => {
                return Ok(SessionEnd::Rejected(reject.reason))
            }
            Frame::Cancel(cancel) => return Err(TransferError::Cancelled(cancel.reason)),
            Frame::Ping(ping) => conn.send(&Frame::Pong(ping)).await?,
            Frame::Pong(_) => {}
            other => return Err(TransferError::unexpected("ACCEPT or REJECT", &other)),
        }
    };
    history.accepted_once = true;
    history.session_accepted = true;
    history.pinned_key = Some(peer.public_key);
    history.first_accept.get_or_insert_with(Instant::now);

    // Start from what the receiver already has.
    let mut acked: Vec<ChunkBitmap> = plan
        .files
        .iter()
        .map(|f| ChunkBitmap::new(f.chunks))
        .collect();
    for have in &accept.have {
        let index = file_index(plan, have.file_id)?;
        acked[index] = ChunkBitmap::from_bytes(plan.files[index].chunks, &have.chunk_bitmap)
            .ok_or_else(|| {
                TransferError::Protocol(format!("bad chunk bitmap for file {}", have.file_id))
            })?;
    }
    let resumed_bytes: u64 = plan
        .files
        .iter()
        .zip(&acked)
        .map(|(file, bitmap)| {
            (0..file.chunks)
                .filter(|&c| bitmap.contains(c))
                .map(|c| chunk_len(file.entry.size, c))
                .sum::<u64>()
        })
        .sum();
    history.resumed_bytes = resumed_bytes;
    on_event(SendEvent::Accepted { resumed_bytes });

    let missing: VecDeque<ChunkRef> = acked
        .iter()
        .enumerate()
        .flat_map(|(file, bitmap)| bitmap.missing().map(move |chunk| ChunkRef { file, chunk }))
        .collect();
    let mut progress = Progress {
        bytes_done: resumed_bytes,
        bytes_total: plan.total_size,
        files_done: 0,
        files_total: plan.files.len(),
    };
    on_event(SendEvent::Progress(progress));

    // Everything below runs concurrently; dropping `tasks` stops it all.
    let mut tasks = JoinSet::new();
    let (control_reader, mut control) = conn.into_split();
    let mut frames = spawn_control_reader(&mut tasks, control_reader);

    let stream_count = options.streams.clamp(1, MAX_STREAMS).min(missing.len());
    history.streams = history.streams.max(stream_count);
    let queue = Arc::new(ChunkQueue::new(missing));
    let window = Arc::new(Semaphore::new(stream_count.max(1) * WINDOW_PER_STREAM_KIB));
    let in_flight: InFlight = Arc::default();
    let (events_tx, mut events) = mpsc::channel(256);
    let token = handshake.session_token();
    for index in 0..stream_count {
        let data =
            open_data_stream(addr, me, peer.public_key, token, plan.transfer_id, index).await?;
        tasks.spawn(stream_loop(
            data,
            plan.clone(),
            queue.clone(),
            window.clone(),
            in_flight.clone(),
            events_tx.clone(),
            history.corrupt_once.clone(),
        ));
    }
    drop(events_tx);

    let mut retries: HashMap<ChunkRef, u32> = HashMap::new();
    let mut done_sent = vec![false; plan.files.len()];
    // Files whose chunks are all acknowledged, waiting for their full hash.
    let mut waiting_for_hash: BTreeSet<usize> = (0..plan.files.len())
        .filter(|&i| acked[i].is_complete())
        .collect();
    // One-chunk files normally get their hash when their chunk is sent; one
    // the receiver already has (resume) must be hashed from disk instead.
    hashes.compute(
        plan,
        waiting_for_hash
            .iter()
            .copied()
            .filter(|&i| plan.files[i].chunks == 1 && hashes.get(i).is_none())
            .collect(),
    );
    send_ready_file_dones(
        &mut control,
        plan,
        hashes,
        &mut waiting_for_hash,
        &mut done_sent,
    )
    .await?;

    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.tick().await;
    let mut ping_nonce = 0u64;

    loop {
        tokio::select! {
            Some(frame) = frames.recv() => match frame? {
                Frame::ChunkAck(ack) => {
                    let index = file_index(plan, ack.file_id)?;
                    for chunk in ack.chunk_indices {
                        if chunk >= plan.files[index].chunks {
                            return Err(TransferError::Protocol(format!(
                                "CHUNK_ACK for chunk {chunk} of file {} is out of range", ack.file_id
                            )));
                        }
                        acked[index].insert(chunk);
                        release(&in_flight, &window, ChunkRef { file: index, chunk });
                    }
                    if acked[index].is_complete() && !done_sent[index] {
                        waiting_for_hash.insert(index);
                        send_ready_file_dones(&mut control, plan, hashes, &mut waiting_for_hash, &mut done_sent).await?;
                    }
                }
                Frame::ChunkRetry(retry) => {
                    let index = file_index(plan, retry.file_id)?;
                    let chunk = ChunkRef { file: index, chunk: retry.chunk_index };
                    if retry.chunk_index >= plan.files[index].chunks {
                        continue;
                    }
                    // The receiver may ask again for chunks it acknowledged
                    // earlier, if the finished file failed its final check.
                    if acked[index].remove(retry.chunk_index) {
                        done_sent[index] = false;
                    }
                    let count = retries.entry(chunk).or_default();
                    *count += 1;
                    if *count >= MAX_CHUNK_RETRIES {
                        let err = TransferError::TooManyRetries {
                            file: plan.files[index].entry.name.clone(),
                            chunk: retry.chunk_index,
                        };
                        let _ = control.send(&Frame::Cancel(Cancel {
                            transfer_id: plan.transfer_id,
                            reason: err.to_string(),
                        })).await;
                        return Err(err);
                    }
                    history.resent_chunks += 1;
                    release(&in_flight, &window, chunk);
                    progress.bytes_done = progress
                        .bytes_done
                        .saturating_sub(chunk_len(plan.files[index].entry.size, retry.chunk_index));
                    queue.push_front(chunk);
                }
                Frame::TransferComplete(done) if done.transfer_id == plan.transfer_id => {
                    queue.close();
                    progress.bytes_done = plan.total_size;
                    progress.files_done = plan.files.len();
                    on_event(SendEvent::Progress(progress));
                    let _ = control.shutdown().await;
                    return Ok(SessionEnd::Completed);
                }
                Frame::Cancel(cancel) => return Err(TransferError::Cancelled(cancel.reason)),
                Frame::Ping(ping) => control.send(&Frame::Pong(ping)).await?,
                Frame::Pong(_) => {}
                other => return Err(TransferError::unexpected("CHUNK_ACK or TRANSFER_COMPLETE", &other)),
            },
            Some(event) = events.recv() => match event {
                StreamEvent::ChunkSent { chunk, bytes, hash } => {
                    // A one-chunk file's hash is its chunk's hash.
                    if plan.files[chunk.file].chunks == 1 {
                        hashes.set(chunk.file, hash);
                    }
                    progress.bytes_done = (progress.bytes_done + bytes).min(plan.total_size);
                    history.peak.add(bytes);
                    on_event(SendEvent::Progress(progress));
                }
                StreamEvent::Failed(err) => {
                    // A data connection breaking may just be the receiver
                    // closing after a CANCEL that is still on its way over
                    // the control connection. Look for it before treating
                    // this as a network problem (which would reconnect).
                    if err.is_retryable() {
                        if let Some(reason) = pending_cancel(&mut frames).await {
                            return Err(TransferError::Cancelled(reason));
                        }
                    }
                    return Err(err);
                }
            },
            _ = hashes.ready.notified() => {
                send_ready_file_dones(&mut control, plan, hashes, &mut waiting_for_hash, &mut done_sent).await?;
            }
            _ = ping.tick() => {
                ping_nonce += 1;
                control.send(&Frame::Ping(Ping { nonce: ping_nonce })).await?;
            }
            _ = cancelled(&mut cancel) => {
                queue.close();
                let _ = control.send(&Frame::Cancel(Cancel {
                    transfer_id: plan.transfer_id,
                    reason: "cancelled by the sender".into(),
                })).await;
                return Err(TransferError::CancelledByUser);
            }
        }
    }
}

/// Waits briefly for a CANCEL on the control connection.
async fn pending_cancel(
    frames: &mut mpsc::Receiver<Result<Frame, TransferError>>,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(500);
    while let Ok(Some(Ok(frame))) = tokio::time::timeout_at(deadline, frames.recv()).await {
        if let Frame::Cancel(cancel) = frame {
            return Some(cancel.reason);
        }
    }
    None
}

/// Sends FILE_DONE for every file that is fully acknowledged and whose
/// whole-file hash is ready.
async fn send_ready_file_dones(
    control: &mut FrameWriter<WriteHalf<TcpStream>>,
    plan: &TransferPlan,
    hashes: &FullHashes,
    waiting: &mut BTreeSet<usize>,
    done_sent: &mut [bool],
) -> Result<(), TransferError> {
    let mut sent_any = false;
    for index in waiting.clone() {
        let Some(result) = hashes.get(index) else {
            continue;
        };
        let blake3_full = result.map_err(|e| {
            TransferError::local_file(&plan.files[index].source.label())(std::io::Error::other(e))
        })?;
        control
            .write_frame(&Frame::FileDone(FileDone {
                file_id: plan.files[index].entry.file_id,
                blake3_full,
            }))
            .await?;
        waiting.remove(&index);
        done_sent[index] = true;
        sent_any = true;
    }
    if sent_any {
        control.flush().await?;
    }
    Ok(())
}

fn release(in_flight: &InFlight, window: &Semaphore, chunk: ChunkRef) {
    if let Some(permits) = in_flight
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&chunk)
    {
        window.add_permits(permits as usize);
    }
}

fn file_index(plan: &TransferPlan, file_id: u32) -> Result<usize, TransferError> {
    // File ids are assigned 0..n by `describe_files`.
    let index = file_id as usize;
    if plan
        .files
        .get(index)
        .is_some_and(|f| f.entry.file_id == file_id)
    {
        Ok(index)
    } else {
        Err(TransferError::Protocol(format!(
            "unknown file_id {file_id}"
        )))
    }
}

fn chunk_len(size: u64, chunk: u64) -> u64 {
    chunk_end(chunk, size) - chunk * CHUNK_SIZE
}

async fn connect(addr: SocketAddr) -> Result<TcpStream, TransferError> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| ConnectionError::Timeout("TCP connection"))?
        .map_err(ConnectionError::from)?;
    stream.set_nodelay(true).map_err(ConnectionError::from)?;
    Ok(stream)
}

/// Opens one data connection: its own Noise handshake, which must prove the
/// same receiver key as the control connection, then STREAM_OPEN.
async fn open_data_stream(
    addr: SocketAddr,
    me: &LocalDevice,
    receiver_key: [u8; 32],
    session_token: [u8; 32],
    transfer_id: uuid::Uuid,
    index: usize,
) -> Result<FrameWriter<WriteHalf<TcpStream>>, TransferError> {
    let (mut conn, handshake) =
        secure_connection(connect(addr).await?, &me.identity, Role::Initiator).await?;
    if handshake.remote_public_key != receiver_key {
        return Err(TransferError::BadDataStream(
            "data connection reached a different device".to_string(),
        ));
    }
    conn.send(&Frame::StreamOpen(StreamOpen {
        transfer_id,
        session_token,
        stream_index: index as u16,
    }))
    .await?;
    // The receiver never writes on data connections.
    let (_reader, writer) = conn.into_split();
    Ok(writer)
}

async fn stream_loop(
    mut data: FrameWriter<WriteHalf<TcpStream>>,
    plan: Arc<TransferPlan>,
    queue: Arc<ChunkQueue>,
    window: Arc<Semaphore>,
    in_flight: InFlight,
    events: mpsc::Sender<StreamEvent>,
    corrupt_once: Arc<Mutex<Option<(u32, u64)>>>,
) {
    let result: Result<(), TransferError> = async {
        let mut buf = Vec::with_capacity(CHUNK_SIZE as usize);
        while let Some(chunk) = queue.next().await {
            let file = &plan.files[chunk.file];
            let start = chunk.chunk * CHUNK_SIZE;
            let len = chunk_len(file.entry.size, chunk.chunk);
            let permits = len.div_ceil(1024).max(1) as u32;
            window
                .acquire_many(permits)
                .await
                .expect("window semaphore is never closed")
                .forget();
            in_flight
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(chunk, permits);

            buf.resize(len as usize, 0);
            let source = file.source.clone();
            buf = tokio::task::spawn_blocking(move || {
                source.read_at(&mut buf, start)?;
                Ok::<_, std::io::Error>(buf)
            })
            .await
            .map_err(|e| TransferError::Protocol(format!("read task failed: {e}")))?
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    TransferError::FileChanged {
                        file: file.source.label().display().to_string(),
                    }
                } else {
                    TransferError::local_file(&file.source.label())(e)
                }
            })?;
            let hash = blake3::hash(&buf);

            let file_id = file.entry.file_id;
            let corrupt = {
                let mut pending = corrupt_once.lock().unwrap_or_else(|e| e.into_inner());
                pending
                    .take_if(|target| *target == (file_id, chunk.chunk))
                    .is_some()
            };
            for (i, piece) in buf.chunks(DATA_PAYLOAD_LEN).enumerate() {
                let offset = start + (i * DATA_PAYLOAD_LEN) as u64;
                if corrupt && i == 0 {
                    let mut bad = piece.to_vec();
                    bad[0] ^= 0xFF;
                    data.write_data(file_id, offset, &bad).await?;
                } else {
                    data.write_data(file_id, offset, piece).await?;
                }
            }
            data.write_frame(&Frame::ChunkHash(ChunkHash {
                file_id,
                chunk_index: chunk.chunk,
                blake3: hash.into(),
            }))
            .await?;
            data.flush().await?;
            let sent = StreamEvent::ChunkSent {
                chunk,
                bytes: len,
                hash: hash.into(),
            };
            if events.send(sent).await.is_err() {
                break;
            }
        }
        let _ = data.shutdown().await;
        Ok(())
    }
    .await;
    if let Err(err) = result {
        let _ = events.send(StreamEvent::Failed(err)).await;
    }
}
