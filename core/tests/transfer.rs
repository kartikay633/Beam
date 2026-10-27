//! End-to-end transfers between an in-process sender and receiver over
//! loopback TCP.

use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use beam_core::frame::Frame;
use beam_core::messages::*;
use beam_core::{
    send_to, Connection, Decision, DeviceInfo, ReceiveOutcome, Receiver, ReceiverConfig,
    SendOutcome, TransferError,
};
use tokio::net::TcpStream;
use uuid::Uuid;

fn device(name: &str) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(*b"testdev!"),
        name: name.to_string(),
        device_type: DeviceType::Laptop,
    }
}

async fn start_receiver(download_dir: &Path) -> Receiver {
    Receiver::bind(
        "127.0.0.1:0",
        ReceiverConfig {
            device: device("receiver"),
            download_dir: download_dir.to_path_buf(),
        },
    )
    .await
    .unwrap()
}

/// Writes `size` pseudo-random bytes (BLAKE3 in XOF mode, seeded by `seed`).
fn write_random_file(path: &Path, size: u64, seed: u64) {
    let mut output = blake3::Hasher::new()
        .update(&seed.to_le_bytes())
        .finalize_xof();
    let mut file = BufWriter::new(fs::File::create(path).unwrap());
    let mut buf = vec![0u8; 1 << 20];
    let mut remaining = size;
    while remaining > 0 {
        let n = remaining.min(buf.len() as u64) as usize;
        output.fill(&mut buf[..n]);
        file.write_all(&buf[..n]).unwrap();
        remaining -= n as u64;
    }
    file.flush().unwrap();
}

fn hash_file(path: &Path) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(fs::File::open(path).unwrap()).unwrap();
    hasher.finalize()
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread")]
async fn large_file_and_many_small_files_arrive_identical() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let inbox = tmp.path().join("inbox");
    fs::create_dir(&src).unwrap();

    let mut paths: Vec<PathBuf> = Vec::new();
    let big = src.join("big.bin");
    write_random_file(&big, 200 * 1_000_000 + 12_345, 1);
    paths.push(big);
    for i in 0..500u64 {
        let path = src.join(format!("small_{i:03}.dat"));
        // Includes empty files and sizes on both sides of a DATA frame boundary.
        let size = match i % 5 {
            0 => 0,
            1 => 1,
            2 => 63 * 1024,
            3 => 63 * 1024 + 1,
            _ => 1_000 + i * 37,
        };
        write_random_file(&path, size, 100 + i);
        paths.push(path);
    }

    let receiver = start_receiver(&inbox).await;
    let addr = receiver.local_addr().unwrap();
    let receiving = tokio::spawn(async move {
        receiver
            .accept_transfer(|_| async { Decision::Accept }, |_| {})
            .await
    });

    let started = Instant::now();
    let sent = send_to(addr, &device("sender"), &paths, |_| {})
        .await
        .unwrap();
    let received = receiving.await.unwrap().unwrap();
    println!(
        "transferred 501 files ({} MB) in {:.2?}",
        200,
        started.elapsed()
    );

    let SendOutcome::Completed(summary) = sent else {
        panic!("transfer was rejected");
    };
    assert_eq!(summary.files, 501);
    let ReceiveOutcome::Completed { files, from, .. } = received else {
        panic!("receiver did not complete");
    };
    assert_eq!(from.name, "sender");
    assert_eq!(files.len(), 501);

    for path in &paths {
        let saved = inbox.join(path.file_name().unwrap());
        assert_eq!(hash_file(path), hash_file(&saved), "{}", saved.display());
    }
    assert!(
        files_in(&inbox).iter().all(|n| !n.ends_with(".beam-part")),
        "no partial files are left behind"
    );
}

#[tokio::test]
async fn declined_transfer_reports_the_reason_and_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("hello.txt");
    fs::write(&file, b"hello").unwrap();
    let inbox = tmp.path().join("inbox");

    let receiver = start_receiver(&inbox).await;
    let addr = receiver.local_addr().unwrap();
    let receiving = tokio::spawn(async move {
        receiver
            .accept_transfer(
                |offer| async move {
                    assert_eq!(offer.files[0].name, "hello.txt");
                    assert_eq!(offer.total_size, 5);
                    Decision::Reject("not now".into())
                },
                |_| {},
            )
            .await
    });

    let sent = send_to(addr, &device("sender"), &[file], |_| {})
        .await
        .unwrap();
    assert!(matches!(sent, SendOutcome::Rejected { reason } if reason == "not now"));
    assert!(matches!(
        receiving.await.unwrap().unwrap(),
        ReceiveOutcome::Rejected { .. }
    ));
    assert!(!inbox.exists());
}

/// Speaks the protocol by hand so tests can send things a well-behaved
/// sender never would.
struct RawSender {
    conn: Connection<TcpStream>,
    transfer_id: Uuid,
}

impl RawSender {
    async fn offer(addr: std::net::SocketAddr, files: &[(&str, &[u8])]) -> RawSender {
        let mut conn = Connection::plain(TcpStream::connect(addr).await.unwrap());
        conn.send(&Frame::Hello(device("raw").hello()))
            .await
            .unwrap();
        assert!(matches!(conn.read_frame().await.unwrap(), Frame::Hello(_)));

        let transfer_id = Uuid::new_v4();
        let entries: Vec<FileEntry> = files
            .iter()
            .enumerate()
            .map(|(i, (name, bytes))| FileEntry {
                file_id: i as u32,
                name: name.to_string(),
                size: bytes.len() as u64,
                mime: "application/octet-stream".into(),
                modified_unix: 0,
            })
            .collect();
        let total_size = entries.iter().map(|e| e.size).sum();
        conn.send(&Frame::Offer(Offer {
            transfer_id,
            files: entries,
            total_size,
        }))
        .await
        .unwrap();
        assert!(matches!(conn.read_frame().await.unwrap(), Frame::Accept(_)));
        RawSender { conn, transfer_id }
    }

    /// Sends a small file as one DATA frame, one CHUNK_HASH and FILE_DONE.
    /// `chunk_hash` overrides the correct chunk hash.
    async fn send_file(&mut self, file_id: u32, bytes: &[u8], chunk_hash: Option<[u8; 32]>) {
        if !bytes.is_empty() {
            self.conn.write_data(file_id, 0, bytes).await.unwrap();
            let hash = chunk_hash.unwrap_or(*blake3::hash(bytes).as_bytes());
            self.conn
                .write_frame(&Frame::ChunkHash(ChunkHash {
                    file_id,
                    chunk_index: 0,
                    blake3: Hash32(hash),
                }))
                .await
                .unwrap();
        }
        self.conn
            .send(&Frame::FileDone(FileDone {
                file_id,
                blake3_full: blake3::hash(bytes).into(),
            }))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn hostile_file_names_stay_inside_the_download_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let inbox = tmp.path().join("inbox");
    let receiver = start_receiver(&inbox).await;
    let addr = receiver.local_addr().unwrap();
    let receiving = tokio::spawn(async move {
        receiver
            .accept_transfer(|_| async { Decision::Accept }, |_| {})
            .await
    });

    let files: &[(&str, &[u8])] = &[
        ("../escape.txt", b"one"),
        ("..\\..\\escape2.txt", b"two"),
        ("C:\\Windows\\win.ini", b"three"),
        ("CON.txt", b"four"),
        ("same.txt", b"five"),
        ("SAME.txt", b"six"),
        ("..", b""),
    ];
    let mut raw = RawSender::offer(addr, files).await;
    for (i, (_, bytes)) in files.iter().enumerate() {
        raw.send_file(i as u32, bytes, None).await;
    }
    assert!(matches!(
        raw.conn.read_frame().await.unwrap(),
        Frame::TransferComplete(c) if c.transfer_id == raw.transfer_id
    ));
    receiving.await.unwrap().unwrap();

    assert_eq!(
        files_in(&inbox),
        [
            "SAME (1).txt",
            "_CON.txt",
            "escape.txt",
            "escape2.txt",
            "file",
            "same.txt",
            "win.ini"
        ]
    );
    assert_eq!(files_in(tmp.path()), ["inbox"], "nothing escaped the inbox");
    assert_eq!(fs::read(inbox.join("SAME (1).txt")).unwrap(), b"six");
}

#[tokio::test]
async fn corrupted_chunk_is_detected_and_nothing_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let inbox = tmp.path().join("inbox");
    let receiver = start_receiver(&inbox).await;
    let addr = receiver.local_addr().unwrap();
    let receiving = tokio::spawn(async move {
        receiver
            .accept_transfer(|_| async { Decision::Accept }, |_| {})
            .await
    });

    let mut raw = RawSender::offer(addr, &[("photo.jpg", b"original bytes")]).await;
    raw.send_file(0, b"original bytes", Some([0xAA; 32])).await;

    let err = receiving.await.unwrap().unwrap_err();
    assert!(
        matches!(&err, TransferError::HashMismatch { chunk: Some(0), .. }),
        "{err}"
    );
    assert!(matches!(
        raw.conn.read_frame().await.unwrap(),
        Frame::Cancel(c) if c.reason.contains("hash mismatch")
    ));
    assert!(files_in(&inbox).is_empty(), "{:?}", files_in(&inbox));
}

#[tokio::test]
async fn sending_a_folder_fails_before_connecting() {
    let tmp = tempfile::tempdir().unwrap();
    let err = send_to(
        "127.0.0.1:9",
        &device("sender"),
        &[tmp.path().to_path_buf()],
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("not a regular file"), "{err}");
}
