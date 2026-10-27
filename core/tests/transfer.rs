//! End-to-end transfers between an in-process sender and receiver over
//! loopback TCP, including the security properties from SPEC.md section 6.

use std::fs;
use std::io::{BufWriter, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use beam_core::frame::Frame;
use beam_core::messages::*;
use beam_core::{
    secure_connection, send_to, Connection, Decision, Identity, LocalDevice, NoiseCipher,
    ReceiveOutcome, Receiver, ReceiverConfig, Role, SendEvent, SendOutcome, TransferError,
    TrustStore, TrustedDevice,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

fn device(name: &str) -> LocalDevice {
    LocalDevice {
        identity: Identity::generate(),
        name: name.to_string(),
        device_type: DeviceType::Laptop,
    }
}

fn trust_of(devices: &[(&LocalDevice, bool)]) -> TrustStore {
    let store = TrustStore::in_memory();
    for (device, auto_accept) in devices {
        store
            .upsert(TrustedDevice::new(
                device.name.clone(),
                *device.identity.public_key(),
                *auto_accept,
            ))
            .unwrap();
    }
    store
}

async fn start_receiver(me: LocalDevice, trust: TrustStore, download_dir: &Path) -> Receiver {
    Receiver::bind(
        "127.0.0.1:0",
        ReceiverConfig {
            device: me,
            download_dir: download_dir.to_path_buf(),
            trust,
        },
    )
    .await
    .unwrap()
}

/// Starts a receiver that accepts one transfer, answering every offer with
/// `decision`, and returns its address and the task.
async fn spawn_receiver(
    me: LocalDevice,
    trust: TrustStore,
    download_dir: &Path,
    decision: Decision,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<ReceiveOutcome, TransferError>>,
) {
    let receiver = start_receiver(me, trust, download_dir).await;
    let addr = receiver.local_addr().unwrap();
    let task = tokio::spawn(async move {
        receiver
            .accept_transfer(|_| async move { decision }, |_| {})
            .await
    });
    (addr, task)
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

    let (addr, receiving) = spawn_receiver(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        Decision::Accept,
    )
    .await;

    let started = Instant::now();
    let sent = send_to(
        addr,
        &device("sender"),
        &TrustStore::in_memory(),
        &paths,
        |_| {},
    )
    .await
    .unwrap();
    let received = receiving.await.unwrap().unwrap();
    println!(
        "transferred 501 files (200 MB) encrypted in {:.2?}",
        started.elapsed()
    );

    let SendOutcome::Completed(summary) = sent else {
        panic!("transfer was rejected");
    };
    assert_eq!(summary.files, 501);
    let ReceiveOutcome::Completed { files, from, .. } = received else {
        panic!("receiver did not complete");
    };
    assert_eq!(from.name(), "sender");
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

    let receiver = start_receiver(device("receiver"), TrustStore::in_memory(), &inbox).await;
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

    let sent = send_to(
        addr,
        &device("sender"),
        &TrustStore::in_memory(),
        &[file],
        |_| {},
    )
    .await
    .unwrap();
    assert!(matches!(sent, SendOutcome::Rejected { reason } if reason == "not now"));
    assert!(matches!(
        receiving.await.unwrap().unwrap(),
        ReceiveOutcome::Rejected { .. }
    ));
    assert!(!inbox.exists());
}

#[tokio::test]
async fn both_sides_see_the_same_verification_code() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a.txt");
    fs::write(&file, b"a").unwrap();

    let receiver_code = Arc::new(Mutex::new(None));
    let seen = receiver_code.clone();
    let receiver = start_receiver(
        device("receiver"),
        TrustStore::in_memory(),
        &tmp.path().join("in"),
    )
    .await;
    let addr = receiver.local_addr().unwrap();
    let receiving = tokio::spawn(async move {
        receiver
            .accept_transfer(
                move |offer| {
                    assert!(!offer.from.is_trusted());
                    *seen.lock().unwrap() = Some(offer.from.verification_code.clone());
                    async { Decision::Accept }
                },
                |_| {},
            )
            .await
    });

    let mut sender_code = None;
    send_to(
        addr,
        &device("sender"),
        &TrustStore::in_memory(),
        &[file],
        |event| {
            if let SendEvent::Connected(peer) = event {
                sender_code = Some(peer.verification_code);
            }
        },
    )
    .await
    .unwrap();
    receiving.await.unwrap().unwrap();

    let sender_code = sender_code.expect("sender saw the code");
    assert_eq!(sender_code.len(), 6);
    assert!(sender_code.chars().all(|c| c.is_ascii_digit()));
    assert_eq!(Some(sender_code), receiver_code.lock().unwrap().clone());
}

#[tokio::test]
async fn trusted_sender_with_auto_accept_skips_the_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a.txt");
    fs::write(&file, b"a").unwrap();
    let sender = device("phone");
    let receiver_device = device("laptop");

    let receiver = start_receiver(
        receiver_device.clone(),
        trust_of(&[(&sender, true)]),
        &tmp.path().join("in"),
    )
    .await;
    let addr = receiver.local_addr().unwrap();
    let receiving = tokio::spawn(async move {
        receiver
            .accept_transfer(
                |_| async { panic!("a trusted auto-accept sender must not be prompted") },
                |_| {},
            )
            .await
    });

    // The sender trusts the receiver too, and sees that.
    let mut sender_saw_trusted = false;
    let sent = send_to(
        addr,
        &sender,
        &trust_of(&[(&receiver_device, false)]),
        &[file],
        |event| {
            if let SendEvent::Connected(peer) = event {
                sender_saw_trusted = peer.is_trusted();
            }
        },
    )
    .await
    .unwrap();
    assert!(matches!(sent, SendOutcome::Completed(_)));
    assert!(sender_saw_trusted);
    assert!(matches!(
        receiving.await.unwrap().unwrap(),
        ReceiveOutcome::Completed {
            auto_accepted: true,
            ..
        }
    ));
}

#[tokio::test]
async fn trusted_sender_without_auto_accept_is_still_asked() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a.txt");
    fs::write(&file, b"a").unwrap();
    let sender = device("phone");

    let receiver = start_receiver(
        device("laptop"),
        trust_of(&[(&sender, false)]),
        &tmp.path().join("in"),
    )
    .await;
    let addr = receiver.local_addr().unwrap();
    let receiving = tokio::spawn(async move {
        receiver
            .accept_transfer(
                |offer| async move {
                    assert!(offer.from.is_trusted());
                    Decision::Reject("asked and said no".into())
                },
                |_| {},
            )
            .await
    });

    let sent = send_to(addr, &sender, &TrustStore::in_memory(), &[file], |_| {})
        .await
        .unwrap();
    assert!(matches!(sent, SendOutcome::Rejected { .. }));
    receiving.await.unwrap().unwrap();
}

/// An attacker that copies a trusted device's name and id but has its own
/// key is refused before any offer is shown.
#[tokio::test]
async fn impersonating_a_trusted_device_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let real_phone = device("Kartikay's Pixel");
    let receiver = start_receiver(
        device("laptop"),
        trust_of(&[(&real_phone, true)]),
        &tmp.path().join("in"),
    )
    .await;
    let addr = receiver.local_addr().unwrap();
    let receiving = tokio::spawn(async move {
        receiver
            .accept_transfer(|_| async { panic!("must not reach the offer") }, |_| {})
            .await
    });

    let attacker = Identity::generate();
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut conn, _) = secure_connection(stream, &attacker, Role::Initiator)
        .await
        .unwrap();
    // The real phone's HELLO: its name and id, but sent under the attacker's key.
    conn.send(&Frame::Hello(real_phone.hello())).await.unwrap();

    let err = receiving.await.unwrap().unwrap_err();
    assert!(
        matches!(err, TransferError::IdentityMismatch { claimed, .. } if claimed == real_phone.device_id()),
        "{err}"
    );
    assert!(err.to_string().contains("connection refused"));
}

/// The sender trusts the real receiver. A man-in-the-middle answering on
/// the receiver's address can't present the receiver's key, so it can only
/// pose as an unknown device, and the two verification codes don't match.
#[tokio::test]
async fn man_in_the_middle_is_detected_by_trust_and_verification_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a.txt");
    fs::write(&file, b"a").unwrap();
    let sender = device("phone");
    let real_receiver = device("laptop");

    // The real receiver records the code it shows, then declines.
    let receiver_code = Arc::new(Mutex::new(String::new()));
    let seen = receiver_code.clone();
    let receiver = start_receiver(
        real_receiver.clone(),
        trust_of(&[(&sender, true)]),
        &tmp.path().join("in"),
    )
    .await;
    let receiver_addr = receiver.local_addr().unwrap();
    let receiving = tokio::spawn(async move {
        receiver
            .accept_transfer(
                move |offer| {
                    *seen.lock().unwrap() = offer.from.verification_code.clone();
                    assert!(
                        !offer.from.is_trusted(),
                        "the MITM is not the trusted phone"
                    );
                    async { Decision::Reject("unknown".into()) }
                },
                |_| {},
            )
            .await
    });

    // The MITM terminates the sender's session with its own key, opens its
    // own session to the receiver, and relays HELLO, OFFER and the answer.
    let mitm_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mitm_addr = mitm_listener.local_addr().unwrap();
    let mitm = tokio::spawn(async move {
        let mitm_key = Identity::generate();
        let (from_sender, _) = mitm_listener.accept().await.unwrap();
        let (mut to_sender, _) = secure_connection(from_sender, &mitm_key, Role::Responder)
            .await
            .unwrap();
        let Frame::Hello(sender_hello) = to_sender.read_frame().await.unwrap() else {
            panic!("expected HELLO");
        };

        let (mut to_receiver, _) = secure_connection(
            TcpStream::connect(receiver_addr).await.unwrap(),
            &mitm_key,
            Role::Initiator,
        )
        .await
        .unwrap();
        let posing_as_sender = LocalDevice {
            identity: mitm_key.clone(),
            name: sender_hello.name.clone(),
            device_type: DeviceType::Phone,
        };
        to_receiver
            .send(&Frame::Hello(posing_as_sender.hello()))
            .await
            .unwrap();
        let Frame::Hello(receiver_hello) = to_receiver.read_frame().await.unwrap() else {
            panic!("expected HELLO");
        };
        let posing_as_receiver = LocalDevice {
            identity: mitm_key,
            name: receiver_hello.name,
            device_type: DeviceType::Laptop,
        };
        to_sender
            .send(&Frame::Hello(posing_as_receiver.hello()))
            .await
            .unwrap();
        relay_one(&mut to_sender, &mut to_receiver).await; // OFFER
        relay_one(&mut to_receiver, &mut to_sender).await; // REJECT
    });

    let mut sender_view = None;
    let sent = send_to(
        mitm_addr,
        &sender,
        &trust_of(&[(&real_receiver, false)]),
        &[file],
        |event| {
            if let SendEvent::Connected(peer) = event {
                sender_view = Some(peer);
            }
        },
    )
    .await
    .unwrap();
    assert!(matches!(sent, SendOutcome::Rejected { .. }));
    mitm.await.unwrap();
    receiving.await.unwrap().unwrap();

    let sender_view = sender_view.unwrap();
    assert_eq!(sender_view.name(), "laptop", "the MITM copied the name…");
    assert!(!sender_view.is_trusted(), "…but not the trusted key");
    assert_ne!(
        sender_view.verification_code,
        *receiver_code.lock().unwrap(),
        "the two screens show different codes"
    );
}

async fn relay_one(
    from: &mut Connection<TcpStream, NoiseCipher>,
    to: &mut Connection<TcpStream, NoiseCipher>,
) {
    let frame = from.read_frame().await.unwrap();
    to.send(&frame).await.unwrap();
}

/// Records every byte that crosses the network and checks that neither the
/// file contents nor the file name nor the device name appear in clear.
#[tokio::test]
async fn nothing_readable_crosses_the_wire() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = b"BEAM-PLAINTEXT-MARKER ";
    let file = tmp.path().join("top-secret-plans.txt");
    fs::write(&file, marker.repeat(10_000)).unwrap();

    let (receiver_addr, receiving) = spawn_receiver(
        device("receiver-laptop"),
        TrustStore::in_memory(),
        &tmp.path().join("in"),
        Decision::Accept,
    )
    .await;

    let captured = Arc::new(Mutex::new(Vec::new()));
    let tap = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tap_addr = tap.local_addr().unwrap();
    let recording = captured.clone();
    let proxy = tokio::spawn(async move {
        let (client, _) = tap.accept().await.unwrap();
        let server = TcpStream::connect(receiver_addr).await.unwrap();
        let (client_read, client_write) = client.into_split();
        let (server_read, server_write) = server.into_split();
        tokio::join!(
            pipe(client_read, server_write, recording.clone()),
            pipe(server_read, client_write, recording),
        );
    });

    let sent = send_to(
        tap_addr,
        &device("sender-phone"),
        &TrustStore::in_memory(),
        std::slice::from_ref(&file),
        |_| {},
    )
    .await
    .unwrap();
    assert!(matches!(sent, SendOutcome::Completed(_)));
    receiving.await.unwrap().unwrap();
    proxy.await.unwrap();

    let wire = captured.lock().unwrap().clone();
    assert!(wire.len() > 200_000, "the file did cross the wire");
    for secret in [
        &marker[..],
        b"top-secret-plans",
        b"sender-phone",
        b"receiver-laptop",
    ] {
        assert!(
            !wire.windows(secret.len()).any(|w| w == secret),
            "{:?} visible on the wire",
            String::from_utf8_lossy(secret)
        );
    }
    assert_eq!(
        hash_file(&file),
        hash_file(&tmp.path().join("in").join("top-secret-plans.txt"))
    );
}

async fn pipe(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    recording: Arc<Mutex<Vec<u8>>>,
) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        recording.lock().unwrap().extend_from_slice(&buf[..n]);
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = to.shutdown().await;
}

/// Speaks the protocol by hand (with real encryption) so tests can send
/// things a well-behaved sender never would.
struct RawSender {
    conn: Connection<TcpStream, NoiseCipher>,
    transfer_id: Uuid,
}

impl RawSender {
    async fn offer(addr: SocketAddr, files: &[(&str, &[u8])]) -> RawSender {
        let me = device("raw");
        let stream = TcpStream::connect(addr).await.unwrap();
        let (mut conn, _) = secure_connection(stream, &me.identity, Role::Initiator)
            .await
            .unwrap();
        conn.send(&Frame::Hello(me.hello())).await.unwrap();
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
    let (addr, receiving) = spawn_receiver(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        Decision::Accept,
    )
    .await;

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
    let (addr, receiving) = spawn_receiver(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        Decision::Accept,
    )
    .await;

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
        &TrustStore::in_memory(),
        &[tmp.path().to_path_buf()],
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("not a regular file"), "{err}");
}
