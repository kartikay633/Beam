//! The sending side of a transfer: HELLO → OFFER → wait for ACCEPT → stream
//! every file as DATA / CHUNK_HASH / FILE_DONE → wait for TRANSFER_COMPLETE.
//!
//! Files are sent back to back without waiting for a round trip per file, so
//! many small files are pipelined.

use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, BufReader};
use tokio::net::{TcpStream, ToSocketAddrs};
use uuid::Uuid;

use crate::connection::{Connection, ConnectionError, RecordCipher, DATA_PAYLOAD_LEN};
use crate::frame::Frame;
use crate::messages::{ChunkHash, FileDone, FileEntry, Offer};
use crate::secure::Role;
use crate::transfer::{
    chunk_count, chunk_end, establish_session, LocalDevice, PeerInfo, Progress, TransferError,
    TransferSummary,
};
use crate::trust::TrustStore;

const FILE_READ_BUFFER: usize = 1024 * 1024;

#[derive(Debug)]
pub enum SendOutcome {
    Completed(TransferSummary),
    Rejected { reason: String },
}

/// What the sender reports while it works.
#[derive(Debug, Clone)]
pub enum SendEvent {
    /// The encrypted session is up and the receiver's identity is verified.
    /// Show the verification code now, while the receiver decides.
    Connected(PeerInfo),
    Progress(Progress),
}

/// Checks the files, connects to `addr` over TCP and sends them.
pub async fn send_to(
    addr: impl ToSocketAddrs,
    me: &LocalDevice,
    trust: &TrustStore,
    paths: &[PathBuf],
    on_event: impl FnMut(SendEvent),
) -> Result<SendOutcome, TransferError> {
    // Fail on a bad path before touching the network.
    let files = describe_files(paths).await?;
    let stream = TcpStream::connect(addr)
        .await
        .map_err(ConnectionError::from)?;
    stream.set_nodelay(true).map_err(ConnectionError::from)?;
    run_sender(stream, me, trust, files, paths, on_event).await
}

/// Runs the sender protocol (handshake included) over a connected stream.
pub async fn send_files<S>(
    stream: S,
    me: &LocalDevice,
    trust: &TrustStore,
    paths: &[PathBuf],
    on_event: impl FnMut(SendEvent),
) -> Result<SendOutcome, TransferError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let files = describe_files(paths).await?;
    run_sender(stream, me, trust, files, paths, on_event).await
}

async fn run_sender<S>(
    stream: S,
    me: &LocalDevice,
    trust: &TrustStore,
    files: Vec<FileEntry>,
    paths: &[PathBuf],
    mut on_event: impl FnMut(SendEvent),
) -> Result<SendOutcome, TransferError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let total_size = files.iter().map(|f| f.size).sum();
    let transfer_id = Uuid::new_v4();

    let (mut conn, peer) = establish_session(stream, me, trust, Role::Initiator).await?;
    on_event(SendEvent::Connected(peer));
    let mut on_progress = |p| on_event(SendEvent::Progress(p));

    conn.send(&Frame::Offer(Offer {
        transfer_id,
        files: files.clone(),
        total_size,
    }))
    .await?;

    match conn.read_frame().await? {
        Frame::Accept(accept) if accept.transfer_id == transfer_id => {}
        Frame::Reject(reject) if reject.transfer_id == transfer_id => {
            return Ok(SendOutcome::Rejected {
                reason: reject.reason,
            })
        }
        Frame::Cancel(cancel) => return Err(TransferError::Cancelled(cancel.reason)),
        other => return Err(TransferError::unexpected("ACCEPT or REJECT", &other)),
    }

    let started = Instant::now();
    let mut progress = Progress {
        bytes_done: 0,
        bytes_total: total_size,
        files_done: 0,
        files_total: files.len(),
    };
    on_progress(progress);

    let mut buf = vec![0u8; DATA_PAYLOAD_LEN];
    for (entry, path) in files.iter().zip(paths) {
        send_file(
            &mut conn,
            entry,
            path,
            &mut buf,
            &mut progress,
            &mut on_progress,
        )
        .await?;
        progress.files_done += 1;
        on_progress(progress);
    }
    conn.flush().await?;

    loop {
        match conn.read_frame().await? {
            Frame::TransferComplete(done) if done.transfer_id == transfer_id => break,
            Frame::Ping(ping) => conn.send(&Frame::Pong(ping)).await?,
            Frame::Cancel(cancel) => return Err(TransferError::Cancelled(cancel.reason)),
            other => return Err(TransferError::unexpected("TRANSFER_COMPLETE", &other)),
        }
    }
    // The transfer is complete; a failed close changes nothing for the user.
    let _ = conn.shutdown().await;

    Ok(SendOutcome::Completed(TransferSummary {
        files: files.len(),
        bytes: total_size,
        duration: started.elapsed(),
    }))
}

async fn send_file<S, C>(
    conn: &mut Connection<S, C>,
    entry: &FileEntry,
    path: &Path,
    buf: &mut [u8],
    progress: &mut Progress,
    on_progress: &mut impl FnMut(Progress),
) -> Result<(), TransferError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    C: RecordCipher,
{
    let file = File::open(path)
        .await
        .map_err(TransferError::local_file(path))?;
    let mut file = BufReader::with_capacity(FILE_READ_BUFFER, file);
    let mut full_hasher = blake3::Hasher::new();
    let mut offset = 0u64;

    for chunk_index in 0..chunk_count(entry.size) {
        let end = chunk_end(chunk_index, entry.size);
        let mut chunk_hasher = blake3::Hasher::new();
        while offset < end {
            let len = (end - offset).min(buf.len() as u64) as usize;
            let piece = &mut buf[..len];
            file.read_exact(piece).await.map_err(|e| {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    TransferError::FileChanged {
                        file: path.display().to_string(),
                    }
                } else {
                    TransferError::local_file(path)(e)
                }
            })?;
            chunk_hasher.update(piece);
            full_hasher.update(piece);
            conn.write_data(entry.file_id, offset, piece).await?;

            offset += len as u64;
            progress.bytes_done += len as u64;
            on_progress(*progress);
        }
        conn.write_frame(&Frame::ChunkHash(ChunkHash {
            file_id: entry.file_id,
            chunk_index,
            blake3: chunk_hasher.finalize().into(),
        }))
        .await?;
    }

    conn.write_frame(&Frame::FileDone(FileDone {
        file_id: entry.file_id,
        blake3_full: full_hasher.finalize().into(),
    }))
    .await?;
    Ok(())
}

async fn describe_files(paths: &[PathBuf]) -> Result<Vec<FileEntry>, TransferError> {
    let mut files = Vec::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(TransferError::local_file(path))?;
        if !metadata.is_file() {
            return Err(TransferError::local_file(path)(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a regular file",
            )));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        let modified_unix = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        let file_id = u32::try_from(index)
            .map_err(|_| TransferError::InvalidOffer("too many files".to_string()))?;
        files.push(FileEntry {
            file_id,
            mime: guess_mime(&name).to_string(),
            name,
            size: metadata.len(),
            modified_unix,
        });
    }
    Ok(files)
}

/// Best-effort MIME type from the extension. Receivers only use it for display.
fn guess_mime(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    match ext.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("heic") => "image/heic",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("mp3") => "audio/mpeg",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::guess_mime;

    #[test]
    fn mime_guessing() {
        assert_eq!(guess_mime("IMG_001.JPG"), "image/jpeg");
        assert_eq!(guess_mime("notes.txt"), "text/plain");
        assert_eq!(guess_mime("archive.tar.gz"), "application/octet-stream");
        assert_eq!(guess_mime("README"), "application/octet-stream");
    }
}
