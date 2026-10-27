//! The disk writer: one blocking thread per transfer, fed through a bounded
//! channel. Verified chunks are written with positional writes into
//! `.beam-part` files that are preallocated to their final size, so chunks
//! from different streams can land in any order.
//!
//! The bounded channel is the receiver's backpressure: when the disk is
//! slower than the network, data streams wait to hand over chunks, stop
//! reading, and TCP slows the sender down. Memory stays flat.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;

use tokio::sync::{mpsc, oneshot};

use crate::messages::Hash32;
use crate::transfer::write_all_at;

/// Chunks (up to 4 MiB each) that may wait for the disk.
pub(super) const WRITE_QUEUE_CHUNKS: usize = 4;
/// Partially written files kept open at once; beyond this, all are synced
/// and closed. (Fully written files are closed right away.)
const MAX_OPEN_FILES: usize = 64;

pub(super) enum WriteJob {
    Chunk {
        file: usize,
        chunk: u64,
        offset: u64,
        data: Vec<u8>,
        /// The verified BLAKE3 of `data`.
        hash: Hash32,
    },
    /// Sync everything written so far, then reply.
    Checkpoint(oneshot::Sender<io::Result<()>>),
    /// The file failed its final check and will be sent again: accept its
    /// chunks once more.
    Reset { file: usize },
    /// Close one file (creating it if it never got data), then reply.
    Finish {
        file: usize,
        reply: oneshot::Sender<io::Result<()>>,
    },
}

/// Reported back to the session.
pub(super) enum WriterEvent {
    Persisted {
        file: usize,
        chunk: u64,
        hash: Hash32,
    },
    Failed(io::Error),
}

pub(super) struct PartFile {
    pub path: PathBuf,
    pub size: u64,
}

/// Starts the writer thread. It exits when every sender of `jobs` is dropped.
pub(super) fn spawn(
    parts: Vec<PartFile>,
    events: mpsc::UnboundedSender<WriterEvent>,
) -> (mpsc::Sender<WriteJob>, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel(WRITE_QUEUE_CHUNKS);
    let handle = tokio::task::spawn_blocking(move || {
        let mut writer = Writer {
            parts,
            open: HashMap::new(),
            written: HashMap::new(),
            dirty: HashSet::new(),
            finished: HashSet::new(),
        };
        while let Some(job) = rx.blocking_recv() {
            match job {
                WriteJob::Chunk {
                    file,
                    chunk,
                    offset,
                    data,
                    hash,
                } => {
                    // A late duplicate of a chunk of a finished (renamed)
                    // file must not recreate its part file.
                    if writer.finished.contains(&file) {
                        continue;
                    }
                    match writer.write(file, offset, &data) {
                        Ok(()) => {
                            let _ = events.send(WriterEvent::Persisted { file, chunk, hash });
                        }
                        Err(e) => {
                            let _ = events.send(WriterEvent::Failed(e));
                            return;
                        }
                    }
                }
                WriteJob::Checkpoint(reply) => {
                    let _ = reply.send(writer.sync_all());
                }
                WriteJob::Finish { file, reply } => {
                    let _ = reply.send(writer.finish(file));
                }
                WriteJob::Reset { file } => {
                    writer.finished.remove(&file);
                    writer.written.remove(&file);
                }
            }
        }
        let _ = writer.sync_all();
    });
    (tx, handle)
}

struct Writer {
    parts: Vec<PartFile>,
    open: HashMap<usize, File>,
    /// Bytes written to each file in this session.
    written: HashMap<usize, u64>,
    /// Open files written since their last sync.
    dirty: HashSet<usize>,
    finished: HashSet<usize>,
}

impl Writer {
    fn write(&mut self, file: usize, offset: u64, data: &[u8]) -> io::Result<()> {
        let handle = self.handle(file)?;
        write_all_at(handle, data, offset)?;
        let written = self.written.entry(file).or_default();
        *written += data.len() as u64;
        if *written >= self.parts[file].size {
            // Every byte is here: close now, without an fsync. If the data
            // never reaches the disk (power cut), the whole-file check at
            // FILE_DONE catches it and the chunks are requested again.
            self.open.remove(&file);
            self.dirty.remove(&file);
        } else {
            self.dirty.insert(file);
        }
        Ok(())
    }

    fn handle(&mut self, file: usize) -> io::Result<&File> {
        if !self.open.contains_key(&file) {
            if self.open.len() >= MAX_OPEN_FILES {
                self.sync_all()?;
                self.open.clear();
            }
            let part = &self.parts[file];
            let handle = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&part.path)?;
            // Preallocate on first open; a resumed part file already has its size.
            if handle.metadata()?.len() != part.size {
                handle.set_len(part.size)?;
            }
            self.open.insert(file, handle);
        }
        Ok(&self.open[&file])
    }

    fn sync_all(&mut self) -> io::Result<()> {
        for file in self.dirty.drain() {
            if let Some(handle) = self.open.get(&file) {
                handle.sync_data()?;
            }
        }
        Ok(())
    }

    /// No sync here: a finished file is verified and renamed like any file
    /// an app saves. (An fsync per file made 1,000 small files take seconds.)
    /// Checkpoints still sync *partially written* files before the progress
    /// file claims their chunks, so a big file never has to start over.
    fn finish(&mut self, file: usize) -> io::Result<()> {
        // Opening also creates and sizes files that never got a chunk
        // (empty files, or files completed in an earlier session).
        self.handle(file)?;
        self.dirty.remove(&file);
        self.open.remove(&file);
        self.written.remove(&file);
        self.finished.insert(file);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn chunks_land_at_their_offsets_in_any_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.beam-part");
        let (events_tx, mut events) = mpsc::unbounded_channel();
        let (jobs, handle) = spawn(
            vec![PartFile {
                path: path.clone(),
                size: 10,
            }],
            events_tx,
        );

        for (chunk, offset, data) in [(1, 5, b"world"), (0, 0, b"hello")] {
            jobs.send(WriteJob::Chunk {
                file: 0,
                chunk,
                offset,
                data: data.to_vec(),
                hash: blake3::hash(data).into(),
            })
            .await
            .unwrap();
        }
        let (reply, done) = oneshot::channel();
        jobs.send(WriteJob::Finish { file: 0, reply })
            .await
            .unwrap();
        done.await.unwrap().unwrap();

        let mut persisted = Vec::new();
        while let Ok(WriterEvent::Persisted { chunk, .. }) = events.try_recv() {
            persisted.push(chunk);
        }
        assert_eq!(persisted, [1, 0]);
        assert_eq!(std::fs::read(&path).unwrap(), b"helloworld");

        // A late duplicate after Finish must not touch the file again.
        std::fs::rename(&path, dir.path().join("x")).unwrap();
        jobs.send(WriteJob::Chunk {
            file: 0,
            chunk: 0,
            offset: 0,
            data: b"HELLO".to_vec(),
            hash: blake3::hash(b"HELLO").into(),
        })
        .await
        .unwrap();
        drop(jobs);
        handle.await.unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn new_part_files_are_preallocated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.beam-part");
        let (events_tx, _events) = mpsc::unbounded_channel();
        let (jobs, handle) = spawn(
            vec![PartFile {
                path: path.clone(),
                size: 1 << 20,
            }],
            events_tx,
        );
        jobs.send(WriteJob::Chunk {
            file: 0,
            chunk: 0,
            offset: 0,
            data: vec![1; 10],
            hash: blake3::hash(&[1; 10]).into(),
        })
        .await
        .unwrap();
        let (reply, done) = oneshot::channel();
        jobs.send(WriteJob::Checkpoint(reply)).await.unwrap();
        done.await.unwrap().unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 1 << 20);
        drop(jobs);
        handle.await.unwrap();
    }
}
