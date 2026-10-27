//! The receiving side of a transfer: HELLO → OFFER → ask the user → ACCEPT →
//! write DATA into `<name>.beam-part`, verify every chunk and the whole file,
//! rename to the final name → TRANSFER_COMPLETE.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Instant;

use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::net::{TcpListener, ToSocketAddrs};
use uuid::Uuid;

use crate::connection::{Connection, ConnectionError, RecordCipher};
use crate::frame::Frame;
use crate::messages::{
    Accept, Cancel, ChunkHash, Data, FileDone, FileEntry, Hello, Offer, Reject, TransferComplete,
    TransferStats,
};
use crate::sanitize::{sanitize_file_name, unique_file_name, PART_SUFFIX};
use crate::transfer::{
    chunk_count, chunk_end, exchange_hello, DeviceInfo, Progress, TransferError, TransferSummary,
    MAX_FILES_PER_OFFER, MAX_RAW_NAME_BYTES,
};

const FILE_WRITE_BUFFER: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ReceiverConfig {
    pub device: DeviceInfo,
    pub download_dir: PathBuf,
}

/// What the user is asked to accept. File names are already sanitized.
#[derive(Debug, Clone)]
pub struct IncomingOffer {
    pub transfer_id: Uuid,
    pub from: Hello,
    pub files: Vec<FileEntry>,
    pub total_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Accept,
    Reject(String),
}

#[derive(Debug, Clone)]
pub struct SavedFile {
    pub file_id: u32,
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug)]
pub enum ReceiveOutcome {
    Completed {
        from: Hello,
        files: Vec<SavedFile>,
        summary: TransferSummary,
    },
    Rejected {
        from: Hello,
    },
}

/// A TCP listener that receives one transfer per incoming connection.
pub struct Receiver {
    listener: TcpListener,
    config: ReceiverConfig,
}

impl Receiver {
    pub async fn bind(addr: impl ToSocketAddrs, config: ReceiverConfig) -> io::Result<Self> {
        Ok(Receiver {
            listener: TcpListener::bind(addr).await?,
            config,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn config(&self) -> &ReceiverConfig {
        &self.config
    }

    /// Waits for the next connection and runs one transfer on it.
    pub async fn accept_transfer<D, F>(
        &self,
        decide: D,
        on_progress: impl FnMut(Progress),
    ) -> Result<ReceiveOutcome, TransferError>
    where
        D: FnOnce(IncomingOffer) -> F,
        F: Future<Output = Decision>,
    {
        let (stream, _) = self
            .listener
            .accept()
            .await
            .map_err(ConnectionError::from)?;
        stream.set_nodelay(true).map_err(ConnectionError::from)?;
        receive_transfer(Connection::plain(stream), &self.config, decide, on_progress).await
    }
}

/// Runs the receiver protocol over an established connection.
pub async fn receive_transfer<S, C, D, F>(
    mut conn: Connection<S, C>,
    config: &ReceiverConfig,
    decide: D,
    mut on_progress: impl FnMut(Progress),
) -> Result<ReceiveOutcome, TransferError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    C: RecordCipher,
    D: FnOnce(IncomingOffer) -> F,
    F: Future<Output = Decision>,
{
    let from = exchange_hello(&mut conn, &config.device, false).await?;
    let offer = match conn.read_frame().await? {
        Frame::Offer(offer) => offer,
        other => return Err(TransferError::unexpected("OFFER", &other)),
    };
    let transfer_id = offer.transfer_id;

    if let Err(problem) = validate_offer(&offer) {
        let _ = conn
            .send(&Frame::Reject(Reject {
                transfer_id,
                reason: format!("invalid offer: {problem}"),
            }))
            .await;
        return Err(TransferError::InvalidOffer(problem));
    }

    let incoming = IncomingOffer {
        transfer_id,
        from: from.clone(),
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
    let files = incoming.files.clone();
    if let Decision::Reject(reason) = decide(incoming).await {
        conn.send(&Frame::Reject(Reject {
            transfer_id,
            reason,
        }))
        .await?;
        let _ = conn.shutdown().await;
        return Ok(ReceiveOutcome::Rejected { from });
    }

    let dir = &config.download_dir;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(TransferError::local_file(dir))?;
    let mut transfer = IncomingTransfer::new(dir, files, offer.total_size);

    conn.send(&Frame::Accept(Accept {
        transfer_id,
        have: Vec::new(),
    }))
    .await?;
    let started = Instant::now();

    match transfer.run(&mut conn, &mut on_progress).await {
        Ok(()) => {
            let duration = started.elapsed();
            conn.send(&Frame::TransferComplete(TransferComplete {
                transfer_id,
                stats: TransferStats {
                    bytes: offer.total_size,
                    files: offer.files.len() as u32,
                    duration_ms: duration.as_millis() as u64,
                },
            }))
            .await?;
            let _ = conn.shutdown().await;
            Ok(ReceiveOutcome::Completed {
                from,
                summary: TransferSummary {
                    files: offer.files.len(),
                    bytes: offer.total_size,
                    duration,
                },
                files: transfer.saved_files(),
            })
        }
        Err(err) => {
            transfer.discard_partial_files().await;
            let peer_is_gone = matches!(
                err,
                TransferError::Cancelled(_) | TransferError::Connection(_)
            );
            if !peer_is_gone {
                let _ = conn
                    .send(&Frame::Cancel(Cancel {
                        transfer_id,
                        reason: err.to_string(),
                    }))
                    .await;
            }
            Err(err)
        }
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

/// Receive-side state for one transfer.
struct IncomingTransfer {
    dir: PathBuf,
    files: Vec<IncomingFile>,
    index_by_id: HashMap<u32, usize>,
    /// Lower-cased final names claimed by this transfer, so two files with the
    /// same name don't overwrite each other.
    taken_names: HashSet<String>,
    remaining: usize,
    progress: Progress,
}

struct IncomingFile {
    entry: FileEntry,
    final_name: String,
    received: u64,
    next_chunk: u64,
    state: FileState,
}

enum FileState {
    /// No DATA yet; nothing on disk.
    Waiting,
    /// `.beam-part` file is open and being written.
    Writing(Box<OpenFile>),
    Saved(PathBuf),
}

struct OpenFile {
    writer: BufWriter<File>,
    chunk_hasher: blake3::Hasher,
    full_hasher: blake3::Hasher,
}

impl IncomingTransfer {
    fn new(dir: &Path, files: Vec<FileEntry>, total_size: u64) -> Self {
        let mut taken_names = HashSet::new();
        let mut index_by_id = HashMap::with_capacity(files.len());
        let files_total = files.len();
        let files = files
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                let final_name = unique_file_name(dir, &entry.name, &taken_names);
                taken_names.insert(final_name.to_lowercase());
                index_by_id.insert(entry.file_id, index);
                IncomingFile {
                    entry,
                    final_name,
                    received: 0,
                    next_chunk: 0,
                    state: FileState::Waiting,
                }
            })
            .collect();
        IncomingTransfer {
            dir: dir.to_path_buf(),
            files,
            index_by_id,
            taken_names,
            remaining: files_total,
            progress: Progress {
                bytes_done: 0,
                bytes_total: total_size,
                files_done: 0,
                files_total,
            },
        }
    }

    async fn run<S, C>(
        &mut self,
        conn: &mut Connection<S, C>,
        on_progress: &mut impl FnMut(Progress),
    ) -> Result<(), TransferError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
        C: RecordCipher,
    {
        on_progress(self.progress);
        while self.remaining > 0 {
            match conn.read_frame().await? {
                Frame::Data(data) => {
                    self.on_data(data).await?;
                    on_progress(self.progress);
                }
                Frame::ChunkHash(hash) => self.on_chunk_hash(hash)?,
                Frame::FileDone(done) => {
                    self.on_file_done(done).await?;
                    on_progress(self.progress);
                }
                Frame::Ping(ping) => conn.send(&Frame::Pong(ping)).await?,
                Frame::Cancel(cancel) => return Err(TransferError::Cancelled(cancel.reason)),
                other => {
                    return Err(TransferError::unexpected(
                        "DATA, CHUNK_HASH or FILE_DONE",
                        &other,
                    ))
                }
            }
        }
        Ok(())
    }

    fn index_of(&self, file_id: u32) -> Result<usize, TransferError> {
        self.index_by_id
            .get(&file_id)
            .copied()
            .ok_or_else(|| TransferError::Protocol(format!("unknown file_id {file_id}")))
    }

    fn file_mut(&mut self, file_id: u32) -> Result<&mut IncomingFile, TransferError> {
        let index = self.index_of(file_id)?;
        Ok(&mut self.files[index])
    }

    async fn on_data(&mut self, data: Data) -> Result<(), TransferError> {
        let dir = self.dir.clone();
        let file = self.file_mut(data.file_id)?;
        if data.offset != file.received {
            return Err(TransferError::Protocol(format!(
                "DATA for file {} at offset {}, expected offset {}",
                data.file_id, data.offset, file.received
            )));
        }
        let end = data.offset + data.bytes.len() as u64;
        if data.bytes.is_empty() || end > chunk_end(file.next_chunk, file.entry.size) {
            return Err(TransferError::Protocol(format!(
                "DATA for file {} at offset {} runs past its chunk or the end of the file",
                data.file_id, data.offset
            )));
        }

        if matches!(file.state, FileState::Waiting) {
            file.state = FileState::Writing(Box::new(file.open_part_file(&dir).await?));
        }
        let FileState::Writing(open) = &mut file.state else {
            return Err(TransferError::Protocol(format!(
                "DATA for file {} after FILE_DONE",
                data.file_id
            )));
        };
        open.writer
            .write_all(&data.bytes)
            .await
            .map_err(TransferError::local_file(&dir.join(&file.final_name)))?;
        open.chunk_hasher.update(&data.bytes);
        open.full_hasher.update(&data.bytes);
        file.received = end;
        self.progress.bytes_done += data.bytes.len() as u64;
        Ok(())
    }

    fn on_chunk_hash(&mut self, hash: ChunkHash) -> Result<(), TransferError> {
        let file = self.file_mut(hash.file_id)?;
        let size = file.entry.size;
        let chunk_complete = hash.chunk_index == file.next_chunk
            && file.next_chunk < chunk_count(size)
            && file.received == chunk_end(file.next_chunk, size);
        let FileState::Writing(open) = &mut file.state else {
            return Err(TransferError::Protocol(format!(
                "CHUNK_HASH for file {} with no data in progress",
                hash.file_id
            )));
        };
        if !chunk_complete {
            return Err(TransferError::Protocol(format!(
                "CHUNK_HASH for chunk {} of file {} does not match a complete chunk",
                hash.chunk_index, hash.file_id
            )));
        }
        let actual = open.chunk_hasher.finalize();
        open.chunk_hasher.reset();
        if actual.as_bytes() != &hash.blake3.0 {
            return Err(TransferError::HashMismatch {
                file: file.final_name.clone(),
                chunk: Some(hash.chunk_index),
            });
        }
        file.next_chunk += 1;
        Ok(())
    }

    async fn on_file_done(&mut self, done: FileDone) -> Result<(), TransferError> {
        let dir = self.dir.clone();
        let index = self.index_of(done.file_id)?;
        let file = &mut self.files[index];
        if file.next_chunk != chunk_count(file.entry.size) {
            return Err(TransferError::Protocol(format!(
                "FILE_DONE for file {} before all of its chunks were verified",
                done.file_id
            )));
        }
        let open = match std::mem::replace(&mut file.state, FileState::Waiting) {
            FileState::Writing(open) => open,
            // An empty file has no chunks, so it was never opened.
            FileState::Waiting => Box::new(file.open_part_file(&dir).await?),
            FileState::Saved(path) => {
                file.state = FileState::Saved(path);
                return Err(TransferError::Protocol(format!(
                    "duplicate FILE_DONE for file {}",
                    done.file_id
                )));
            }
        };
        let OpenFile {
            mut writer,
            full_hasher,
            ..
        } = *open;
        let part_path = file.part_path(&dir);
        writer
            .flush()
            .await
            .map_err(TransferError::local_file(&part_path))?;
        drop(writer);

        if full_hasher.finalize().as_bytes() != &done.blake3_full.0 {
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(TransferError::HashMismatch {
                file: file.final_name.clone(),
                chunk: None,
            });
        }

        // Something may have created a file with our name since the offer.
        if dir.join(&file.final_name).exists() {
            file.final_name = unique_file_name(&dir, &file.final_name, &self.taken_names);
        }
        let final_path = dir.join(&file.final_name);
        tokio::fs::rename(&part_path, &final_path)
            .await
            .map_err(TransferError::local_file(&final_path))?;
        self.taken_names.insert(file.final_name.to_lowercase());

        file.state = FileState::Saved(final_path);
        self.remaining -= 1;
        self.progress.files_done += 1;
        Ok(())
    }

    fn saved_files(&self) -> Vec<SavedFile> {
        self.files
            .iter()
            .filter_map(|f| match &f.state {
                FileState::Saved(path) => Some(SavedFile {
                    file_id: f.entry.file_id,
                    path: path.clone(),
                    size: f.entry.size,
                }),
                _ => None,
            })
            .collect()
    }

    /// Removes `.beam-part` files of a failed transfer. (Resume, which keeps
    /// them, comes in phase 3.)
    async fn discard_partial_files(&mut self) {
        for file in &mut self.files {
            if let FileState::Writing(open) = std::mem::replace(&mut file.state, FileState::Waiting)
            {
                // Close the handle before deleting: Windows can't delete open files.
                let _ = open.writer.into_inner().into_std().await;
                let _ = tokio::fs::remove_file(file.part_path(&self.dir)).await;
            }
        }
    }
}

impl IncomingFile {
    fn part_path(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}{PART_SUFFIX}", self.final_name))
    }

    async fn open_part_file(&self, dir: &Path) -> Result<OpenFile, TransferError> {
        let path = self.part_path(dir);
        let file = File::create(&path)
            .await
            .map_err(TransferError::local_file(&path))?;
        Ok(OpenFile {
            writer: BufWriter::with_capacity(FILE_WRITE_BUFFER, file),
            chunk_hasher: blake3::Hasher::new(),
            full_hasher: blake3::Hasher::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
