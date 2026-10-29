//! The control side of one incoming transfer.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch, Semaphore};
use tokio::task::JoinSet;

use super::manifest::{part_path, Manifest, ManifestFile};
use super::stream::{self, StreamContext, StreamEvent};
use super::writer::{self, PartFile, WriteJob, WriterEvent};
use super::{
    Decision, IncomingOffer, ReceiveHandler, ReceiveOutcome, SavedFile, Shared, StopReason,
};
use crate::bitmap::ChunkBitmap;
use crate::connection::{Connection, ConnectionError};
use crate::frame::Frame;
use crate::messages::{
    Accept, Cancel, ChunkAck, ChunkRetry, FileDone, FileEntry, FileHave, Hash32, Hello, Offer,
    PairRequest, PairResult, Ping, Reject, TransferComplete, TransferStats,
};
use crate::sanitize::{sanitize_file_name, unique_file_name};
use crate::secure::HandshakeInfo;
use crate::transfer::{
    answer_control, chunk_count, chunk_end, hash_file, pending_cancel, spawn_control_reader,
    PeerInfo, Progress, TransferError, TransferSummary, ACK_INTERVAL, CHECKPOINT_INTERVAL,
    CHUNK_SIZE, MAX_CHUNK_RETRIES, MAX_FILES_PER_OFFER, MAX_RAW_NAME_BYTES, PING_INTERVAL,
};
use crate::trust::TrustedDevice;
use crate::visibility::VisibilityMode;

/// How the background check of a finished file came out.
enum Finished {
    /// Hash matched; renamed to its final name.
    Saved(Hash32),
    /// The file on disk doesn't match: its chunks must be sent again.
    Corrupt,
}

/// Whole-file checks that read from disk at once. Each holds a 4 MiB read
/// buffer, so this bounds memory when many big files finish together.
const CONCURRENT_FILE_CHECKS: usize = 2;

/// The sender must make its offer within this long after HELLO.
const OFFER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// CHUNK_ACKs go out before the timer once this many bytes are waiting (one
/// full chunk), so a single stream never stalls on its send window...
const ACK_BATCH_BYTES: u64 = CHUNK_SIZE;
/// ...or once this many chunks are waiting (many small files).
const ACK_BATCH_CHUNKS: usize = 64;

pub(super) async fn run_control(
    mut conn: Connection<TcpStream>,
    hello: Hello,
    handshake: HandshakeInfo,
    shared: Arc<Shared>,
    handler: Arc<dyn ReceiveHandler>,
) {
    let config = &shared.config;
    let peer =
        match answer_control(&mut conn, hello, &handshake, &config.device, &config.trust).await {
            Ok(peer) => peer,
            Err(e) => return handler.connection_refused(&e),
        };
    let offer = match tokio::time::timeout(OFFER_TIMEOUT, conn.read_frame()).await {
        Ok(Ok(Frame::Offer(offer))) => offer,
        Ok(Ok(Frame::PairRequest(request))) => {
            return pair(conn, peer, request, &shared, handler.as_ref()).await
        }
        Ok(Ok(other)) => {
            return handler.connection_refused(&TransferError::unexpected("OFFER", &other))
        }
        Ok(Err(e)) => return handler.connection_refused(&e.into()),
        Err(_) => return handler.connection_refused(&ConnectionError::Timeout("OFFER").into()),
    };
    let transfer_id = offer.transfer_id;
    let result = receive(conn, peer, offer, &handshake, &shared, &handler).await;
    handler.finished(transfer_id, &result);
}

async fn receive(
    mut conn: Connection<TcpStream>,
    peer: PeerInfo,
    offer: Offer,
    handshake: &HandshakeInfo,
    shared: &Arc<Shared>,
    handler: &Arc<dyn ReceiveHandler>,
) -> Result<ReceiveOutcome, TransferError> {
    let transfer_id = offer.transfer_id;
    let reject = |reason: String| {
        Frame::Reject(Reject {
            transfer_id,
            reason,
        })
    };
    if let Err(problem) = validate_offer(&offer) {
        let _ = conn
            .send(&reject(format!("invalid offer: {problem}")))
            .await;
        return Err(TransferError::InvalidOffer(problem));
    }
    let guard = match shared.claim(transfer_id, peer.public_key).await {
        Ok(guard) => guard,
        Err(e) => {
            let _ = conn.send(&reject(e.to_string())).await;
            return Err(e);
        }
    };

    let dir = shared.config.download_dir.clone();
    super::manifest::remove_stale(&dir, &shared.active_ids());
    let previous = Manifest::load(&dir, transfer_id);
    let resumable = previous
        .as_ref()
        .is_some_and(|m| manifest_matches(m, &offer, &peer.public_key));
    if let (Some(stale), false) = (&previous, resumable) {
        stale.discard(&dir);
    }

    // Visibility comes first: nobody is asked about an offer the current
    // mode doesn't allow. A resumed transfer was already accepted, so it may
    // continue in trusted-only mode (for example after "everyone" expired
    // mid-transfer), but not once visibility is off.
    let refusal = match shared.config.visibility.mode() {
        VisibilityMode::Off => Some("not accepting transfers right now"),
        VisibilityMode::TrustedOnly if !peer.is_trusted() && !resumable => {
            Some("only accepting transfers from trusted devices")
        }
        _ => None,
    };
    if let Some(reason) = refusal {
        conn.send(&reject(reason.to_string())).await?;
        let _ = conn.shutdown().await;
        return Ok(ReceiveOutcome::Rejected {
            from: peer,
            reason: reason.to_string(),
        });
    }

    let auto_accepted = !resumable && peer.trusted.as_ref().is_some_and(|t| t.auto_accept);
    let incoming = IncomingOffer {
        transfer_id,
        from: peer.clone(),
        files: offer
            .files
            .iter()
            .map(|f| FileEntry {
                name: sanitize_file_name(&f.name),
                ..f.clone()
            })
            .collect(),
        total_size: offer.total_size,
    };
    let decision = if resumable || auto_accepted {
        Decision::Accept
    } else {
        handler.decide(incoming.clone()).await
    };
    if let Decision::Reject(reason) = decision {
        conn.send(&reject(reason.clone())).await?;
        let _ = conn.shutdown().await;
        return Ok(ReceiveOutcome::Rejected { from: peer, reason });
    }
    handler.accepted(&incoming, resumable);

    std::fs::create_dir_all(&dir).map_err(TransferError::local_file(&dir))?;
    let mut transfer = if resumable {
        IncomingTransfer::resume(&dir, previous.expect("checked above"))
    } else {
        IncomingTransfer::new(&dir, &offer, &peer.public_key)
    };
    // From here on the transfer counts as accepted, even across reconnects.
    transfer.save_manifest()?;
    conn.send(&Frame::Accept(Accept {
        transfer_id,
        have: transfer.have(),
    }))
    .await?;

    let (_route, data_connections) =
        shared.add_route(handshake.session_token(), transfer_id, peer.public_key);
    let started = Instant::now();
    let resumed_bytes = transfer.progress().bytes_done;
    let result = transfer
        .run(conn, data_connections, guard.stop.clone(), handler.as_ref())
        .await;

    match result {
        Ok(()) => {
            let manifest_path = Manifest::path(&dir, transfer_id);
            let _ = std::fs::remove_file(&manifest_path);
            // Removes the progress folder too, if no other transfer uses it.
            if let Some(partial_dir) = manifest_path.parent() {
                let _ = std::fs::remove_dir(partial_dir);
            }
            Ok(ReceiveOutcome::Completed {
                from: peer,
                auto_accepted,
                resumed: resumable,
                files: transfer.saved_files(),
                summary: TransferSummary {
                    files: offer.files.len(),
                    bytes: offer.total_size,
                    duration: started.elapsed(),
                    streams: transfer.streams_seen,
                    resent_chunks: transfer.corrupt_chunks,
                    resumed_bytes,
                    ..TransferSummary::default()
                },
            })
        }
        Err(err) => {
            if err.is_retryable() || matches!(err, TransferError::Superseded) {
                // Keep everything for the sender's reconnect.
                let _ = transfer.save_manifest();
            } else {
                transfer.manifest.discard(&dir);
            }
            Err(err)
        }
    }
}

/// Answers a PAIR_REQUEST: a valid, unused token from our pairing code means
/// the sender scanned our screen, so its proven key becomes trusted.
async fn pair(
    mut conn: Connection<TcpStream>,
    peer: PeerInfo,
    request: PairRequest,
    shared: &Shared,
    handler: &dyn ReceiveHandler,
) {
    let accepted = shared.config.pairing.redeem(&request.token);
    let result = if accepted {
        shared
            .config
            .trust
            .upsert(TrustedDevice::new(
                peer.name().to_string(),
                peer.public_key,
                true,
            ))
            .map_err(|e| e.to_string())
    } else {
        Err("this pairing code has expired; show a new one".to_string())
    };
    let reply = PairResult {
        accepted: result.is_ok(),
        reason: result.clone().err().unwrap_or_default(),
    };
    let _ = conn.send(&Frame::PairResult(reply)).await;
    let _ = conn.shutdown().await;
    match result {
        Ok(()) => handler.paired(&peer),
        Err(reason) => handler.connection_refused(&TransferError::PairingRefused(reason)),
    }
}

fn validate_offer(offer: &Offer) -> Result<(), String> {
    if offer.files.is_empty() {
        return Err("no files".to_string());
    }
    if offer.files.len() > MAX_FILES_PER_OFFER {
        return Err(format!(
            "{} files is more than the limit of {MAX_FILES_PER_OFFER}",
            offer.files.len()
        ));
    }
    let mut ids = HashSet::with_capacity(offer.files.len());
    let mut total: u64 = 0;
    for file in &offer.files {
        if !ids.insert(file.file_id) {
            return Err(format!("duplicate file_id {}", file.file_id));
        }
        if file.name.len() > MAX_RAW_NAME_BYTES {
            return Err(format!(
                "a file name is longer than {MAX_RAW_NAME_BYTES} bytes"
            ));
        }
        total = total
            .checked_add(file.size)
            .ok_or_else(|| "total size overflows".to_string())?;
    }
    if total != offer.total_size {
        return Err(format!(
            "total_size is {} but the files add up to {total}",
            offer.total_size
        ));
    }
    Ok(())
}

/// A saved manifest can be resumed only by the same sender, offering the
/// same files (same fingerprints, same order).
fn manifest_matches(manifest: &Manifest, offer: &Offer, sender_key: &[u8; 32]) -> bool {
    manifest.sender_key == hex::encode(sender_key)
        && manifest.files.len() == offer.files.len()
        && manifest
            .files
            .iter()
            .zip(&offer.files)
            .all(|(saved, offered)| {
                saved.file_id == offered.file_id
                    && saved.name == offered.name
                    && saved.size == offered.size
                    && saved.modified_unix == offered.modified_unix
            })
}

struct IncomingFile {
    file_id: u32,
    size: u64,
    final_name: String,
    bitmap: ChunkBitmap,
    /// Whole-file hash, once verified and renamed.
    done: Option<Hash32>,
    /// FILE_DONE arrived; the final check and rename are running.
    finishing: bool,
    /// For a one-chunk file, the verified chunk hash, which is also the
    /// whole-file hash: no need to read the file back from disk.
    single_chunk_hash: Option<Hash32>,
}

struct IncomingTransfer {
    dir: PathBuf,
    manifest: Manifest,
    files: Vec<IncomingFile>,
    index_by_id: HashMap<u32, usize>,
    /// Lower-cased names claimed by this transfer.
    taken_names: HashSet<String>,
    streams_seen: usize,
    corrupt_chunks: u32,
    retries: HashMap<(usize, u64), u32>,
    file_checks: Arc<Semaphore>,
}

impl IncomingTransfer {
    fn new(dir: &Path, offer: &Offer, sender_key: &[u8; 32]) -> Self {
        let mut taken_names = HashSet::new();
        let manifest_files = offer
            .files
            .iter()
            .map(|f| {
                let final_name = unique_file_name(dir, &sanitize_file_name(&f.name), &taken_names);
                taken_names.insert(final_name.to_lowercase());
                ManifestFile {
                    file_id: f.file_id,
                    name: f.name.clone(),
                    size: f.size,
                    modified_unix: f.modified_unix,
                    final_name,
                    chunk_bitmap: hex::encode(ChunkBitmap::new(chunk_count(f.size)).as_bytes()),
                    done_blake3: None,
                }
            })
            .collect();
        Self::from_manifest(
            dir,
            Manifest {
                version: 1,
                transfer_id: offer.transfer_id,
                sender_key: hex::encode(sender_key),
                updated_unix: 0,
                files: manifest_files,
            },
            taken_names,
        )
    }

    fn resume(dir: &Path, manifest: Manifest) -> Self {
        let taken_names = manifest
            .files
            .iter()
            .map(|f| f.final_name.to_lowercase())
            .collect();
        Self::from_manifest(dir, manifest, taken_names)
    }

    fn from_manifest(dir: &Path, manifest: Manifest, taken_names: HashSet<String>) -> Self {
        let files: Vec<IncomingFile> = manifest
            .files
            .iter()
            .map(|f| {
                let chunks = chunk_count(f.size);
                let done = f
                    .done_blake3
                    .as_deref()
                    .and_then(|h| hex::decode(h).ok())
                    .and_then(|b| b.try_into().ok())
                    .map(Hash32);
                let mut bitmap = hex::decode(&f.chunk_bitmap)
                    .ok()
                    .and_then(|b| ChunkBitmap::from_bytes(chunks, &b))
                    .unwrap_or_else(|| ChunkBitmap::new(chunks));
                // Trust the bitmap only if the part file is still there, intact in size.
                let part_ok = std::fs::metadata(part_path(dir, &f.final_name))
                    .is_ok_and(|m| m.len() == f.size);
                if done.is_some() {
                    (0..chunks).for_each(|c| {
                        bitmap.insert(c);
                    });
                } else if !part_ok {
                    bitmap = ChunkBitmap::new(chunks);
                }
                IncomingFile {
                    file_id: f.file_id,
                    size: f.size,
                    final_name: f.final_name.clone(),
                    bitmap,
                    done,
                    finishing: false,
                    single_chunk_hash: None,
                }
            })
            .collect();
        let index_by_id = files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.file_id, i))
            .collect();
        IncomingTransfer {
            dir: dir.to_path_buf(),
            manifest,
            files,
            index_by_id,
            taken_names,
            streams_seen: 0,
            corrupt_chunks: 0,
            retries: HashMap::new(),
            file_checks: Arc::new(Semaphore::new(CONCURRENT_FILE_CHECKS)),
        }
    }

    fn have(&self) -> Vec<FileHave> {
        self.files
            .iter()
            .filter(|f| f.bitmap.count() > 0)
            .map(|f| FileHave {
                file_id: f.file_id,
                chunk_bitmap: f.bitmap.as_bytes().to_vec(),
            })
            .collect()
    }

    fn progress(&self) -> Progress {
        Progress {
            bytes_done: self
                .files
                .iter()
                .map(|f| {
                    (0..f.bitmap.len())
                        .filter(|&c| f.bitmap.contains(c))
                        .map(|c| chunk_end(c, f.size) - c * CHUNK_SIZE)
                        .sum::<u64>()
                })
                .sum(),
            bytes_total: self.files.iter().map(|f| f.size).sum(),
            files_done: self.files.iter().filter(|f| f.done.is_some()).count(),
            files_total: self.files.len(),
        }
    }

    fn is_complete(&self) -> bool {
        self.files.iter().all(|f| f.done.is_some())
    }

    fn save_manifest(&mut self) -> Result<(), TransferError> {
        for (saved, file) in self.manifest.files.iter_mut().zip(&self.files) {
            saved.final_name = file.final_name.clone();
            saved.chunk_bitmap = hex::encode(file.bitmap.as_bytes());
            saved.done_blake3 = file.done.map(|h| hex::encode(h.0));
        }
        let path = Manifest::path(&self.dir, self.manifest.transfer_id);
        self.manifest
            .save(&self.dir)
            .map_err(TransferError::local_file(&path))
    }

    fn saved_files(&self) -> Vec<SavedFile> {
        self.files
            .iter()
            .filter(|f| f.done.is_some())
            .map(|f| SavedFile {
                file_id: f.file_id,
                path: self.dir.join(&f.final_name),
                size: f.size,
            })
            .collect()
    }

    fn index_of(&self, file_id: u32) -> Result<usize, TransferError> {
        self.index_by_id
            .get(&file_id)
            .copied()
            .ok_or_else(|| TransferError::Protocol(format!("unknown file_id {file_id}")))
    }

    /// Receives until every file is verified and saved.
    async fn run(
        &mut self,
        conn: Connection<TcpStream>,
        mut data_connections: mpsc::Receiver<Connection<TcpStream>>,
        mut stop: watch::Receiver<StopReason>,
        handler: &dyn ReceiveHandler,
    ) -> Result<(), TransferError> {
        let transfer_id = self.manifest.transfer_id;
        let run_started = Instant::now();
        let (control_reader, mut control) = conn.into_split();
        let mut tasks = JoinSet::new();
        let mut frames = spawn_control_reader(&mut tasks, control_reader);

        let (writer_events_tx, mut writer_events) = mpsc::unbounded_channel();
        let parts = self
            .files
            .iter()
            .map(|f| PartFile {
                path: part_path(&self.dir, &f.final_name),
                size: f.size,
            })
            .collect();
        let (jobs, writer_task) = writer::spawn(parts, writer_events_tx);
        let (stream_events_tx, mut stream_events) = mpsc::unbounded_channel();
        let (verified_tx, mut verified) = mpsc::unbounded_channel();
        let context = Arc::new(StreamContext {
            sizes: self.files.iter().map(|f| f.size).collect(),
            index_by_id: self.index_by_id.clone(),
        });

        let mut pending_acks: BTreeMap<u32, Vec<u64>> = BTreeMap::new();
        let mut pending_count = 0usize;
        let mut pending_bytes = 0u64;
        let mut checkpoint_due = false;
        let mut ack_timer = tokio::time::interval(ACK_INTERVAL);
        let mut checkpoint_timer = tokio::time::interval(CHECKPOINT_INTERVAL);
        // The first tick of an interval fires at once; pings start one
        // interval in.
        let mut ping_timer =
            tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
        let mut ping_nonce = 0u64;
        let mut progress = self.progress();
        handler.progress(transfer_id, progress);

        let result: Result<(), TransferError> = async {
            if self.is_complete() {
                return Ok(());
            }
            loop {
                tokio::select! {
                    Some(data) = data_connections.recv() => {
                        self.streams_seen += 1;
                        let (reader, writer_half) = data.into_split();
                        let (context, jobs, events) = (context.clone(), jobs.clone(), stream_events_tx.clone());
                        tasks.spawn(async move {
                            let _keep_open = writer_half;
                            stream::run(reader, context, jobs, events).await;
                        });
                    }
                    Some(frame) = frames.recv() => match frame? {
                        Frame::FileDone(done) => {
                            let index = self.index_of(done.file_id)?;
                            if self.start_finishing(index, &done, &jobs, &verified_tx, &mut tasks).await? {
                                progress = self.progress();
                                handler.progress(transfer_id, progress);
                            }
                        }
                        Frame::Ping(ping) => control.send(&Frame::Pong(ping)).await?,
                        Frame::Pong(_) => {}
                        Frame::Cancel(cancel) => return Err(TransferError::Cancelled(cancel.reason)),
                        other => return Err(TransferError::unexpected("FILE_DONE", &other)),
                    },
                    Some(event) = writer_events.recv() => match event {
                        WriterEvent::Persisted { file, chunk, hash } => {
                            let entry = &mut self.files[file];
                            if entry.bitmap.len() == 1 {
                                entry.single_chunk_hash = Some(hash);
                            }
                            let len = chunk_end(chunk, entry.size) - chunk * CHUNK_SIZE;
                            if entry.done.is_none() && entry.bitmap.insert(chunk) {
                                progress.bytes_done += len;
                                handler.progress(transfer_id, progress);
                                checkpoint_due = true;
                            }
                            pending_acks.entry(entry.file_id).or_default().push(chunk);
                            pending_count += 1;
                            pending_bytes += len;
                            if pending_bytes >= ACK_BATCH_BYTES || pending_count >= ACK_BATCH_CHUNKS {
                                flush_acks(&mut control, &mut pending_acks, &mut pending_count).await?;
                                pending_bytes = 0;
                            }
                        }
                        WriterEvent::Failed(e) => {
                            return Err(TransferError::local_file(&self.dir)(e));
                        }
                    },
                    Some(event) = stream_events.recv() => match event {
                        StreamEvent::Corrupt { file, chunk } => {
                            let count = self.retries.entry((file, chunk)).or_default();
                            *count += 1;
                            if *count >= MAX_CHUNK_RETRIES {
                                return Err(TransferError::TooManyRetries {
                                    file: self.files[file].final_name.clone(),
                                    chunk,
                                });
                            }
                            self.corrupt_chunks += 1;
                            control.send(&Frame::ChunkRetry(ChunkRetry {
                                file_id: self.files[file].file_id,
                                chunk_index: chunk,
                            })).await?;
                        }
                        StreamEvent::Failed(err) => return Err(err),
                    },
                    Some((index, result)) = verified.recv() => {
                        match result? {
                            Finished::Saved(hash) => self.files[index].done = Some(hash),
                            Finished::Corrupt if self.files[index].bitmap.is_empty() => {
                                return Err(TransferError::HashMismatch {
                                    file: self.files[index].final_name.clone(),
                                    chunk: None,
                                });
                            }
                            Finished::Corrupt => self.request_again(index, &jobs, &mut control).await?,
                        }
                        checkpoint_due = true;
                        progress = self.progress();
                        handler.progress(transfer_id, progress);
                        if self.is_complete() {
                            flush_acks(&mut control, &mut pending_acks, &mut pending_count).await?;
                            return Ok(());
                        }
                    }
                    _ = ack_timer.tick() => {
                        flush_acks(&mut control, &mut pending_acks, &mut pending_count).await?;
                        pending_bytes = 0;
                    }
                    _ = checkpoint_timer.tick(), if checkpoint_due => {
                        checkpoint(&jobs).await?;
                        self.save_manifest()?;
                        checkpoint_due = false;
                    }
                    _ = ping_timer.tick() => {
                        ping_nonce += 1;
                        control.send(&Frame::Ping(Ping { nonce: ping_nonce })).await?;
                    }
                    _ = stop.changed() => {
                        return Err(match *stop.borrow() {
                            StopReason::Cancelled => TransferError::CancelledByUser,
                            _ => TransferError::Superseded,
                        });
                    }
                }
            }
        }
        .await;

        // A transfer ending for good tells the sender *before* the data
        // connections close; otherwise the sender would see them drop first,
        // take it for a network problem, and reconnect.
        if let Err(err) = &result {
            if !err.is_retryable()
                && !matches!(err, TransferError::Cancelled(_) | TransferError::Superseded)
            {
                let _ = control
                    .send(&Frame::Cancel(Cancel {
                        transfer_id,
                        reason: match err {
                            TransferError::CancelledByUser => "cancelled by the receiver".into(),
                            other => other.to_string(),
                        },
                    }))
                    .await;
            }
        }

        // Stop the streams, then let the writer finish and sync what it has.
        tasks.shutdown().await;
        drop(jobs);
        let _ = writer_task.await;
        while let Ok(WriterEvent::Persisted { file, chunk, .. }) = writer_events.try_recv() {
            if self.files[file].done.is_none() {
                self.files[file].bitmap.insert(chunk);
            }
        }

        // The sender cancelling closes its connections right after sending
        // CANCEL; a stream or write may fail here first. Read the CANCEL
        // rather than wait for a resume that will never come. Save progress
        // before waiting: a new receiver (after a crash or restart) may read
        // it at any moment.
        let result = match result {
            Err(err) if err.is_retryable() => {
                let _ = self.save_manifest();
                match pending_cancel(&mut frames).await {
                    Some(reason) => Err(TransferError::Cancelled(reason)),
                    None => Err(err),
                }
            }
            other => other,
        };

        if result.is_ok() {
            control
                .send(&Frame::TransferComplete(TransferComplete {
                    transfer_id,
                    stats: TransferStats {
                        bytes: progress.bytes_total,
                        files: self.files.len() as u32,
                        duration_ms: run_started.elapsed().as_millis() as u64,
                    },
                }))
                .await?;
            let _ = control.shutdown().await;
        }
        result
    }

    /// Handles FILE_DONE: closes the part file, then (in the background)
    /// checks the whole-file hash and renames the part file to its final
    /// name. Returns `true` if the file was already done (a resumed duplicate).
    async fn start_finishing(
        &mut self,
        index: usize,
        done: &FileDone,
        jobs: &mpsc::Sender<WriteJob>,
        verified: &mpsc::UnboundedSender<(usize, Result<Finished, TransferError>)>,
        tasks: &mut JoinSet<()>,
    ) -> Result<bool, TransferError> {
        let file = &mut self.files[index];
        if let Some(hash) = file.done {
            return if hash == done.blake3_full {
                Ok(true)
            } else {
                Err(TransferError::HashMismatch {
                    file: file.final_name.clone(),
                    chunk: None,
                })
            };
        }
        if file.finishing || !file.bitmap.is_complete() {
            return Err(TransferError::Protocol(format!(
                "FILE_DONE for file {} before all of its chunks were verified",
                done.file_id
            )));
        }
        file.finishing = true;
        let part = part_path(&self.dir, &file.final_name);

        // Something may have created a file with our name since the offer.
        if self.dir.join(&file.final_name).exists() {
            file.final_name = unique_file_name(&self.dir, &file.final_name, &self.taken_names);
            self.taken_names.insert(file.final_name.to_lowercase());
        }
        let target = self.dir.join(&file.final_name);
        let known_hash = match file.bitmap.len() {
            0 => Some(Hash32::from(blake3::hash(b""))),
            1 => file.single_chunk_hash,
            _ => None,
        };
        let expected = done.blake3_full;
        let name = file.final_name.clone();
        let verified = verified.clone();
        let jobs = jobs.clone();
        let file_checks = self.file_checks.clone();
        // Closing, checking and renaming run in the background, so the
        // session keeps handling other files meanwhile.
        tasks.spawn(async move {
            let (reply, closed) = oneshot::channel();
            let closed = match jobs.send(WriteJob::Finish { file: index, reply }).await {
                Ok(()) => closed.await.ok(),
                Err(_) => None,
            };
            let closed = match closed {
                Some(Ok(())) => Ok(()),
                Some(Err(e)) => Err(TransferError::LocalFile {
                    path: name.clone(),
                    source: e,
                }),
                None => Err(TransferError::Protocol("disk writer stopped".into())),
            };
            if let Err(e) = closed {
                let _ = verified.send((index, Err(e)));
                return;
            }
            let _slot = match known_hash {
                Some(_) => None,
                None => file_checks.acquire_owned().await.ok(),
            };
            let checked = tokio::task::spawn_blocking(move || -> std::io::Result<bool> {
                let actual = match known_hash {
                    Some(hash) => hash,
                    None => hash_file(&part)?.into(),
                };
                if actual != expected {
                    return Ok(false);
                }
                std::fs::rename(&part, &target)?;
                Ok(true)
            })
            .await;
            let result = match checked {
                Ok(Ok(true)) => Ok(Finished::Saved(expected)),
                Ok(Ok(false)) => Ok(Finished::Corrupt),
                Ok(Err(e)) => Err(TransferError::LocalFile {
                    path: name,
                    source: e,
                }),
                Err(e) => Err(TransferError::Protocol(format!(
                    "finishing task failed: {e}"
                ))),
            };
            let _ = verified.send((index, result));
        });
        Ok(false)
    }

    /// The file on disk doesn't match the sender's whole-file hash (data lost
    /// before it reached the disk, or the source file changed): forget its
    /// chunks and ask for all of them again. A chunk that keeps failing ends
    /// the transfer.
    async fn request_again(
        &mut self,
        index: usize,
        jobs: &mpsc::Sender<WriteJob>,
        control: &mut crate::connection::FrameWriter<tokio::io::WriteHalf<TcpStream>>,
    ) -> Result<(), TransferError> {
        // Queued before the retry requests, so the writer accepts the new
        // chunks when they arrive.
        jobs.send(WriteJob::Reset { file: index })
            .await
            .map_err(|_| TransferError::Protocol("disk writer stopped".into()))?;
        let file = &mut self.files[index];
        file.finishing = false;
        file.single_chunk_hash = None;
        file.bitmap = ChunkBitmap::new(file.bitmap.len());
        for chunk in 0..file.bitmap.len() {
            let count = self.retries.entry((index, chunk)).or_default();
            *count += 1;
            if *count >= MAX_CHUNK_RETRIES {
                return Err(TransferError::HashMismatch {
                    file: file.final_name.clone(),
                    chunk: None,
                });
            }
            control
                .write_frame(&Frame::ChunkRetry(ChunkRetry {
                    file_id: file.file_id,
                    chunk_index: chunk,
                }))
                .await?;
        }
        self.corrupt_chunks += file.bitmap.len() as u32;
        control.flush().await?;
        Ok(())
    }
}

async fn flush_acks(
    control: &mut crate::connection::FrameWriter<tokio::io::WriteHalf<TcpStream>>,
    pending: &mut BTreeMap<u32, Vec<u64>>,
    count: &mut usize,
) -> Result<(), TransferError> {
    if pending.is_empty() {
        return Ok(());
    }
    for (file_id, chunk_indices) in std::mem::take(pending) {
        control
            .write_frame(&Frame::ChunkAck(ChunkAck {
                file_id,
                chunk_indices,
            }))
            .await?;
    }
    *count = 0;
    control.flush().await?;
    Ok(())
}

/// Syncs everything written so far, so the manifest saved next only claims
/// chunks that are really on disk.
async fn checkpoint(jobs: &mpsc::Sender<WriteJob>) -> Result<(), TransferError> {
    let (reply, synced) = oneshot::channel();
    jobs.send(WriteJob::Checkpoint(reply))
        .await
        .map_err(|_| TransferError::Protocol("disk writer stopped".into()))?;
    synced
        .await
        .map_err(|_| TransferError::Protocol("disk writer stopped".into()))?
        .map_err(|e| TransferError::LocalFile {
            path: "download folder".into(),
            source: e,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn offer(files: Vec<(u32, &str, u64)>, total_size: u64) -> Offer {
        Offer {
            transfer_id: Uuid::nil(),
            files: files
                .into_iter()
                .map(|(file_id, name, size)| FileEntry {
                    file_id,
                    name: name.to_string(),
                    size,
                    mime: String::new(),
                    modified_unix: 0,
                })
                .collect(),
            total_size,
        }
    }

    #[test]
    fn offer_validation() {
        assert!(validate_offer(&offer(vec![(0, "a", 1), (1, "b", 2)], 3)).is_ok());
        assert!(validate_offer(&offer(vec![], 0)).is_err());
        assert!(validate_offer(&offer(vec![(0, "a", 1), (0, "b", 2)], 3)).is_err());
        assert!(validate_offer(&offer(vec![(0, "a", 1)], 2)).is_err());
        assert!(validate_offer(&offer(vec![(0, "a", u64::MAX), (1, "b", 1)], 0)).is_err());
        let long = "x".repeat(MAX_RAW_NAME_BYTES + 1);
        assert!(validate_offer(&offer(vec![(0, &long, 1)], 1)).is_err());
        let many: Vec<_> = (0..=MAX_FILES_PER_OFFER as u32)
            .map(|i| (i, "f", 0))
            .collect();
        assert!(validate_offer(&offer(many, 0)).is_err());
    }

    #[test]
    fn manifests_resume_only_for_the_same_sender_and_files() {
        let dir = tempfile::tempdir().unwrap();
        let key = [7u8; 32];
        let the_offer = offer(vec![(0, "a.bin", 10), (1, "b.bin", 20)], 30);
        let transfer = IncomingTransfer::new(dir.path(), &the_offer, &key);
        let manifest = transfer.manifest.clone();

        assert!(manifest_matches(&manifest, &the_offer, &key));
        assert!(
            !manifest_matches(&manifest, &the_offer, &[8u8; 32]),
            "other sender"
        );
        let changed = offer(vec![(0, "a.bin", 11), (1, "b.bin", 20)], 31);
        assert!(
            !manifest_matches(&manifest, &changed, &key),
            "file size changed"
        );
    }
}
