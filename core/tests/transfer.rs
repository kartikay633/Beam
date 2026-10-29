//! End-to-end transfers between an in-process sender and receiver over
//! loopback TCP: correctness, the security properties from SPEC.md section 6,
//! and the speed/reliability properties from section 7.3 (parallel streams,
//! corruption recovery, resume, chaos).

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beam_core::frame::Frame;
use beam_core::messages::*;
use beam_core::{
    secure_connection, send_to, Connection, Decision, Identity, LocalDevice, Role, SendEvent,
    SendOptions, SendOutcome, TrustStore,
};
use common::*;
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

fn options(streams: usize) -> SendOptions {
    SendOptions {
        streams,
        ..SendOptions::default()
    }
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

    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        accept_all,
    )
    .await;
    let started = Instant::now();
    let sent = send_to(
        receiver.addr,
        &device("sender"),
        &TrustStore::in_memory(),
        &paths,
        &options(4),
        |_| {},
    )
    .await
    .unwrap();
    let received = receiver.next_outcome().await;
    println!(
        "transferred 501 files (200 MB) over 4 streams in {:.2?}",
        started.elapsed()
    );

    let SendOutcome::Completed(summary) = sent else {
        panic!("transfer was rejected");
    };
    assert_eq!(summary.files, 501);
    assert_eq!(summary.streams, 4);
    let Finished::Completed {
        from,
        files,
        summary: received_summary,
        ..
    } = received
    else {
        panic!("receiver did not complete: {received:?}");
    };
    assert_eq!(from, "sender");
    assert_eq!(files.len(), 501);
    assert_eq!(received_summary.streams, 4);

    for path in &paths {
        let saved = inbox.join(path.file_name().unwrap());
        assert_eq!(hash_file(path), hash_file(&saved), "{}", saved.display());
    }
    assert_eq!(
        files_in(&inbox).len(),
        501,
        "only finished files remain (no .beam-part files, no progress folder)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thousand_small_files_are_pipelined() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    let paths: Vec<PathBuf> = (0..1_000u64)
        .map(|i| {
            let path = src.join(format!("f{i:04}.txt"));
            write_random_file(&path, 10_000, i);
            path
        })
        .collect();

    let inbox = tmp.path().join("inbox");
    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        accept_all,
    )
    .await;
    let started = Instant::now();
    let sent = send_to(
        receiver.addr,
        &device("sender"),
        &TrustStore::in_memory(),
        &paths,
        &options(4),
        |_| {},
    )
    .await
    .unwrap();
    let elapsed = started.elapsed();
    println!("1,000 x 10 KB files took {elapsed:.2?}");
    assert!(matches!(sent, SendOutcome::Completed(_)));
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Completed { .. }
    ));
    assert_eq!(files_in(&inbox).len(), 1_000);
    assert!(
        elapsed < Duration::from_secs(30),
        "no round trip per file: {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn corrupted_chunk_is_detected_and_sent_again() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("video.bin");
    write_random_file(&file, 3 * 4 * 1024 * 1024 + 5, 9); // 4 chunks
    let inbox = tmp.path().join("inbox");
    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        accept_all,
    )
    .await;

    let sent = send_to(
        receiver.addr,
        &device("sender"),
        &TrustStore::in_memory(),
        std::slice::from_ref(&file),
        &SendOptions {
            streams: 2,
            corrupt_once: Some((0, 2)),
            ..SendOptions::default()
        },
        |_| {},
    )
    .await
    .unwrap();
    let SendOutcome::Completed(summary) = sent else {
        panic!("rejected");
    };
    assert_eq!(
        summary.resent_chunks, 1,
        "the corrupted chunk was sent twice"
    );
    let Finished::Completed {
        summary: received, ..
    } = receiver.next_outcome().await
    else {
        panic!("receiver did not complete");
    };
    assert_eq!(received.resent_chunks, 1, "the receiver caught it");
    assert_eq!(hash_file(&file), hash_file(&inbox.join("video.bin")));
}

/// SPEC.md 11: kill the connection at random points 50 times; the transfer
/// must resume and finish with an identical file.
#[tokio::test(flavor = "multi_thread")]
async fn chaos_fifty_disconnects_during_a_1_gb_transfer() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("one-gigabyte.bin");
    write_random_file(&file, 1_000_000_000, 42);
    let inbox = tmp.path().join("inbox");
    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        accept_all,
    )
    .await;
    let proxy = Proxy::start(receiver.addr, false).await;
    proxy.enable_chaos(50, 0x5eed);

    let started = Instant::now();
    let mut reconnect_events = 0;
    let sent = send_to(
        proxy.addr,
        &device("sender"),
        &TrustStore::in_memory(),
        std::slice::from_ref(&file),
        &options(4),
        |event| {
            if let SendEvent::Reconnecting { .. } = event {
                reconnect_events += 1;
            }
        },
    )
    .await
    .unwrap();
    let SendOutcome::Completed(summary) = sent else {
        panic!("rejected");
    };
    println!(
        "1 GB with {} kills: {:.1?}, {} reconnects, {} reconnect attempts",
        proxy.kills_done(),
        started.elapsed(),
        summary.reconnects,
        reconnect_events
    );
    assert_eq!(proxy.kills_done(), 50, "all 50 kills happened");
    assert!(summary.reconnects >= 1);
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Completed { .. }
    ));
    assert_eq!(
        hash_file(&file),
        hash_file(&inbox.join("one-gigabyte.bin")),
        "identical after 50 disconnects"
    );
    assert_eq!(files_in(&inbox), ["one-gigabyte.bin"]);
}

/// The receiver goes away completely mid-transfer (like a crash or an app
/// restart). A new receiver with the same identity and download folder picks
/// up the progress file, and only the missing chunks are sent.
#[tokio::test(flavor = "multi_thread")]
async fn resume_after_the_receiver_restarts() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("movie.bin");
    write_random_file(&file, 400_000_000, 7);
    let inbox = tmp.path().join("inbox");
    let receiver_device = device("laptop");
    let sender_device = device("phone");

    let first = TestReceiver::start(
        receiver_device.clone(),
        TrustStore::in_memory(),
        &inbox,
        accept_all,
    )
    .await;
    let proxy = Proxy::start(first.addr, false).await;
    let progress = first.progress.clone();

    let sending = {
        let (addr, file) = (proxy.addr, file.clone());
        tokio::spawn(async move {
            send_to(
                addr,
                &sender_device,
                &TrustStore::in_memory(),
                &[file],
                &options(2),
                |_| {},
            )
            .await
        })
    };

    // Let roughly a quarter arrive, then pull the plug on the receiver.
    while progress.load(std::sync::atomic::Ordering::Relaxed) < 100_000_000 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    drop(first);
    proxy.kill_all();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut second = TestReceiver::start(receiver_device, TrustStore::in_memory(), &inbox, |_| {
        panic!("a resumed transfer must not ask the user again")
    })
    .await;
    proxy.set_backend(second.addr);

    let sent = sending.await.unwrap().unwrap();
    assert!(matches!(sent, SendOutcome::Completed(_)));
    let Finished::Completed {
        resumed, summary, ..
    } = second.next_outcome().await
    else {
        panic!("second receiver did not complete");
    };
    assert!(resumed);
    println!(
        "resumed with {:.0} MB already on disk",
        summary.resumed_bytes as f64 / 1e6
    );
    assert!(
        summary.resumed_bytes >= 50_000_000,
        "chunks from before the restart were kept"
    );
    assert_eq!(hash_file(&file), hash_file(&inbox.join("movie.bin")));
}

#[tokio::test]
async fn declined_transfer_reports_the_reason_and_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("hello.txt");
    fs::write(&file, b"hello").unwrap();
    let inbox = tmp.path().join("inbox");

    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        |offer| {
            assert_eq!(offer.files[0].name, "hello.txt");
            assert_eq!(offer.total_size, 5);
            Decision::Reject("not now".into())
        },
    )
    .await;

    let sent = send_to(
        receiver.addr,
        &device("sender"),
        &TrustStore::in_memory(),
        &[file],
        &options(4),
        |_| {},
    )
    .await
    .unwrap();
    assert!(matches!(sent, SendOutcome::Rejected { reason } if reason == "not now"));
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Rejected(_)
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
    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &tmp.path().join("in"),
        move |offer| {
            assert!(!offer.from.is_trusted());
            *seen.lock().unwrap() = Some(offer.from.verification_code.clone());
            Decision::Accept
        },
    )
    .await;

    let mut sender_code = None;
    send_to(
        receiver.addr,
        &device("sender"),
        &TrustStore::in_memory(),
        &[file],
        &options(4),
        |event| {
            if let SendEvent::Connected(peer) = event {
                sender_code = Some(peer.verification_code);
            }
        },
    )
    .await
    .unwrap();
    receiver.next_outcome().await;

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

    let mut receiver = TestReceiver::start(
        receiver_device.clone(),
        trust_of(&[(&sender, true)]),
        &tmp.path().join("in"),
        |_| panic!("a trusted auto-accept sender must not be prompted"),
    )
    .await;

    // The sender trusts the receiver too, and sees that.
    let mut sender_saw_trusted = false;
    let sent = send_to(
        receiver.addr,
        &sender,
        &trust_of(&[(&receiver_device, false)]),
        &[file],
        &options(4),
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
        receiver.next_outcome().await,
        Finished::Completed {
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

    let mut receiver = TestReceiver::start(
        device("laptop"),
        trust_of(&[(&sender, false)]),
        &tmp.path().join("in"),
        |offer| {
            assert!(offer.from.is_trusted());
            Decision::Reject("asked and said no".into())
        },
    )
    .await;

    let sent = send_to(
        receiver.addr,
        &sender,
        &TrustStore::in_memory(),
        &[file],
        &options(4),
        |_| {},
    )
    .await
    .unwrap();
    assert!(matches!(sent, SendOutcome::Rejected { .. }));
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Rejected(_)
    ));
}

/// An attacker that copies a trusted device's name and id but has its own
/// key is refused before any offer is shown.
#[tokio::test]
async fn impersonating_a_trusted_device_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let real_phone = device("Kartikay's Pixel");
    let mut receiver = TestReceiver::start(
        device("laptop"),
        trust_of(&[(&real_phone, true)]),
        &tmp.path().join("in"),
        |_| panic!("must not reach the offer"),
    )
    .await;

    let attacker = Identity::generate();
    let stream = TcpStream::connect(receiver.addr).await.unwrap();
    let (mut conn, _) = secure_connection(stream, &attacker, Role::Initiator)
        .await
        .unwrap();
    // The real phone's HELLO: its name and id, but sent under the attacker's key.
    conn.send(&Frame::Hello(real_phone.hello())).await.unwrap();

    let refused = receiver.next_refusal().await;
    assert!(
        refused.contains(&real_phone.device_id().to_string()),
        "{refused}"
    );
    assert!(refused.contains("connection refused"), "{refused}");
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
    let mut receiver = TestReceiver::start(
        real_receiver.clone(),
        trust_of(&[(&sender, true)]),
        &tmp.path().join("in"),
        move |offer| {
            *seen.lock().unwrap() = offer.from.verification_code.clone();
            assert!(
                !offer.from.is_trusted(),
                "the MITM is not the trusted phone"
            );
            Decision::Reject("unknown".into())
        },
    )
    .await;
    let receiver_addr = receiver.addr;

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
        &options(4),
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
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Rejected(_)
    ));

    let sender_view = sender_view.unwrap();
    assert_eq!(sender_view.name(), "laptop", "the MITM copied the name…");
    assert!(!sender_view.is_trusted(), "…but not the trusted key");
    assert_ne!(
        sender_view.verification_code,
        *receiver_code.lock().unwrap(),
        "the two screens show different codes"
    );
}

async fn relay_one(from: &mut Connection<TcpStream>, to: &mut Connection<TcpStream>) {
    let frame = from.read_frame().await.unwrap();
    to.send(&frame).await.unwrap();
}

/// Records every byte that crosses the network (control and data
/// connections) and checks that neither the file contents nor the file name
/// nor the device names appear in clear.
#[tokio::test(flavor = "multi_thread")]
async fn nothing_readable_crosses_the_wire() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = b"BEAM-PLAINTEXT-MARKER ";
    let file = tmp.path().join("top-secret-plans.txt");
    fs::write(&file, marker.repeat(500_000)).unwrap(); // 11 MB: several chunks

    let mut receiver = TestReceiver::start(
        device("receiver-laptop"),
        TrustStore::in_memory(),
        &tmp.path().join("in"),
        accept_all,
    )
    .await;
    let tap = Proxy::start(receiver.addr, true).await;

    let sent = send_to(
        tap.addr,
        &device("sender-phone"),
        &TrustStore::in_memory(),
        std::slice::from_ref(&file),
        &options(4),
        |_| {},
    )
    .await
    .unwrap();
    assert!(matches!(sent, SendOutcome::Completed(_)));
    receiver.next_outcome().await;

    let wire = tap.recorded();
    assert!(wire.len() > 11_000_000, "the file did cross the wire");
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

/// The next control frame that isn't a keepalive (PINGs are answered).
async fn next_control_frame(raw: &mut RawSender) -> Frame {
    loop {
        match raw.control.read_frame().await.unwrap() {
            Frame::Ping(p) => raw.control.send(&Frame::Pong(p)).await.unwrap(),
            Frame::Pong(_) => {}
            other => return other,
        }
    }
}

/// Speaks the protocol by hand (with real encryption) so tests can send
/// things a well-behaved sender never would.
struct RawSender {
    control: Connection<TcpStream>,
    data: Connection<TcpStream>,
    transfer_id: Uuid,
}

impl RawSender {
    async fn offer(addr: std::net::SocketAddr, files: &[(&str, &[u8])]) -> RawSender {
        let me = device("raw");
        let (mut control, handshake) = secure_connection(
            TcpStream::connect(addr).await.unwrap(),
            &me.identity,
            Role::Initiator,
        )
        .await
        .unwrap();
        control.send(&Frame::Hello(me.hello())).await.unwrap();
        assert!(matches!(
            control.read_frame().await.unwrap(),
            Frame::Hello(_)
        ));

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
        control
            .send(&Frame::Offer(Offer {
                transfer_id,
                files: entries,
                total_size,
            }))
            .await
            .unwrap();
        assert!(matches!(
            control.read_frame().await.unwrap(),
            Frame::Accept(_)
        ));

        let (mut data, _) = secure_connection(
            TcpStream::connect(addr).await.unwrap(),
            &me.identity,
            Role::Initiator,
        )
        .await
        .unwrap();
        data.send(&Frame::StreamOpen(StreamOpen {
            transfer_id,
            session_token: handshake.session_token(),
            stream_index: 0,
        }))
        .await
        .unwrap();
        RawSender {
            control,
            data,
            transfer_id,
        }
    }

    /// Sends small files (one chunk each), waits for their CHUNK_ACKs, then
    /// sends FILE_DONE for all of them and waits for TRANSFER_COMPLETE.
    async fn send_all(&mut self, files: &[(&str, &[u8])]) {
        let mut waiting = std::collections::HashSet::new();
        for (i, (_, bytes)) in files.iter().enumerate() {
            if bytes.is_empty() {
                continue;
            }
            self.data.write_data(i as u32, 0, bytes).await.unwrap();
            self.data
                .write_frame(&Frame::ChunkHash(ChunkHash {
                    file_id: i as u32,
                    chunk_index: 0,
                    blake3: blake3::hash(bytes).into(),
                }))
                .await
                .unwrap();
            waiting.insert(i as u32);
        }
        self.data.flush().await.unwrap();
        while !waiting.is_empty() {
            match self.control.read_frame().await.unwrap() {
                Frame::ChunkAck(ack) => {
                    waiting.remove(&ack.file_id);
                }
                Frame::Ping(p) => self.control.send(&Frame::Pong(p)).await.unwrap(),
                other => panic!("unexpected {other:?}"),
            }
        }
        for (i, (_, bytes)) in files.iter().enumerate() {
            self.control
                .send(&Frame::FileDone(FileDone {
                    file_id: i as u32,
                    blake3_full: blake3::hash(bytes).into(),
                }))
                .await
                .unwrap();
        }
        loop {
            match self.control.read_frame().await.unwrap() {
                Frame::TransferComplete(c) => {
                    assert_eq!(c.transfer_id, self.transfer_id);
                    return;
                }
                Frame::Ping(p) => self.control.send(&Frame::Pong(p)).await.unwrap(),
                Frame::ChunkAck(_) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
    }
}

#[tokio::test]
async fn hostile_file_names_stay_inside_the_download_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let inbox = tmp.path().join("inbox");
    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        accept_all,
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
    let mut raw = RawSender::offer(receiver.addr, files).await;
    raw.send_all(files).await;
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Completed { .. }
    ));

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

/// If a finished file doesn't match FILE_DONE's whole-file hash (data lost
/// before reaching the disk, or the source changed), the receiver asks for
/// its chunks again instead of failing the transfer.
#[tokio::test]
async fn file_that_fails_its_final_check_is_requested_again() {
    let tmp = tempfile::tempdir().unwrap();
    let inbox = tmp.path().join("inbox");
    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        accept_all,
    )
    .await;
    let content: &[u8] = b"the real content";
    let mut raw = RawSender::offer(receiver.addr, &[("a.txt", content)]).await;

    async fn send_chunk(raw: &mut RawSender, content: &[u8]) {
        raw.data.write_data(0, 0, content).await.unwrap();
        raw.data
            .send(&Frame::ChunkHash(ChunkHash {
                file_id: 0,
                chunk_index: 0,
                blake3: blake3::hash(content).into(),
            }))
            .await
            .unwrap();
    }
    send_chunk(&mut raw, content).await;
    assert!(matches!(
        next_control_frame(&mut raw).await,
        Frame::ChunkAck(_)
    ));
    // Claim a different whole-file hash.
    raw.control
        .send(&Frame::FileDone(FileDone {
            file_id: 0,
            blake3_full: blake3::hash(b"something else").into(),
        }))
        .await
        .unwrap();
    assert!(matches!(
        next_control_frame(&mut raw).await,
        Frame::ChunkRetry(ChunkRetry {
            file_id: 0,
            chunk_index: 0
        })
    ));

    // Send it again, correctly this time.
    send_chunk(&mut raw, content).await;
    assert!(matches!(
        next_control_frame(&mut raw).await,
        Frame::ChunkAck(_)
    ));
    raw.control
        .send(&Frame::FileDone(FileDone {
            file_id: 0,
            blake3_full: blake3::hash(content).into(),
        }))
        .await
        .unwrap();
    assert!(matches!(
        next_control_frame(&mut raw).await,
        Frame::TransferComplete(_)
    ));
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Completed { .. }
    ));
    assert_eq!(fs::read(inbox.join("a.txt")).unwrap(), content);
}

#[tokio::test]
async fn data_stream_with_a_wrong_session_token_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &tmp.path().join("in"),
        accept_all,
    )
    .await;
    let me = device("intruder");
    let (mut data, _) = secure_connection(
        TcpStream::connect(receiver.addr).await.unwrap(),
        &me.identity,
        Role::Initiator,
    )
    .await
    .unwrap();
    data.send(&Frame::StreamOpen(StreamOpen {
        transfer_id: Uuid::new_v4(),
        session_token: [9; 32],
        stream_index: 0,
    }))
    .await
    .unwrap();
    assert!(receiver
        .next_refusal()
        .await
        .contains("unknown session token"));
}

#[tokio::test]
async fn sending_a_folder_fails_before_connecting() {
    let tmp = tempfile::tempdir().unwrap();
    let err = send_to(
        "127.0.0.1:9",
        &device("sender"),
        &TrustStore::in_memory(),
        &[tmp.path().to_path_buf()],
        &options(4),
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("not a regular file"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn sender_can_cancel_mid_transfer() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("big.bin");
    write_random_file(&file, 300_000_000, 3);
    let inbox = tmp.path().join("inbox");
    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        accept_all,
    )
    .await;

    let (cancel, cancel_rx) = tokio::sync::watch::channel(false);
    let progress = receiver.progress.clone();
    let canceller = tokio::spawn(async move {
        while progress.load(std::sync::atomic::Ordering::Relaxed) < 20_000_000 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        cancel.send(true).unwrap();
    });
    let err = send_to(
        receiver.addr,
        &device("sender"),
        &TrustStore::in_memory(),
        &[file],
        &SendOptions {
            cancel: Some(cancel_rx),
            ..SendOptions::default()
        },
        |_| {},
    )
    .await
    .unwrap_err();
    canceller.await.unwrap();
    assert!(
        matches!(err, beam_core::TransferError::CancelledByUser),
        "{err}"
    );
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Failed {
            retryable: false,
            ..
        }
    ));
    assert!(
        files_in(&inbox).is_empty(),
        "partial files are removed: {:?}",
        files_in(&inbox)
    );
}

/// The race CI kept finding: the sender's CANCEL reaches the receiver only
/// after its data connections have closed. The receiver must still treat
/// it as cancelled, not as a network drop to wait out.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_arriving_after_the_streams_close_still_cancels() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("big.bin");
    write_random_file(&file, 300_000_000, 5);
    let inbox = tmp.path().join("inbox");
    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        accept_all,
    )
    .await;
    let proxy = Proxy::start(receiver.addr, false).await;
    proxy.delay_control(Duration::from_millis(150));

    let (cancel, cancel_rx) = tokio::sync::watch::channel(false);
    let progress = receiver.progress.clone();
    let canceller = tokio::spawn(async move {
        while progress.load(std::sync::atomic::Ordering::Relaxed) < 20_000_000 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        cancel.send(true).unwrap();
    });
    let err = send_to(
        proxy.addr,
        &device("sender"),
        &TrustStore::in_memory(),
        &[file],
        &SendOptions {
            cancel: Some(cancel_rx),
            ..SendOptions::default()
        },
        |_| {},
    )
    .await
    .unwrap_err();
    canceller.await.unwrap();
    assert!(
        matches!(err, beam_core::TransferError::CancelledByUser),
        "{err}"
    );
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Failed {
            retryable: false,
            ..
        }
    ));
    assert!(files_in(&inbox).is_empty(), "{:?}", files_in(&inbox));
}

#[tokio::test(flavor = "multi_thread")]
async fn receiver_can_cancel_mid_transfer() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("big.bin");
    write_random_file(&file, 300_000_000, 4);
    let inbox = tmp.path().join("inbox");
    let mut receiver = TestReceiver::start(
        device("receiver"),
        TrustStore::in_memory(),
        &inbox,
        accept_all,
    )
    .await;

    let (handle, progress, accepted) = (
        receiver.handle.clone(),
        receiver.progress.clone(),
        receiver.last_accepted.clone(),
    );
    let canceller = tokio::spawn(async move {
        while progress.load(std::sync::atomic::Ordering::Relaxed) < 20_000_000 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let id = accepted.lock().unwrap().expect("the offer was accepted");
        assert!(handle.cancel(id));
    });
    let err = send_to(
        receiver.addr,
        &device("sender"),
        &TrustStore::in_memory(),
        &[file],
        &options(4),
        |_| {},
    )
    .await
    .unwrap_err();
    canceller.await.unwrap();
    assert!(
        matches!(&err, beam_core::TransferError::Cancelled(reason) if reason.contains("cancelled")),
        "{err}"
    );
    receiver.next_outcome().await;
    assert!(files_in(&inbox).is_empty(), "{:?}", files_in(&inbox));
}
