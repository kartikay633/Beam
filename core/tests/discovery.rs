//! Visibility modes (enforced by the receiver, no network discovery needed)
//! and, behind `--ignored`, real mDNS discovery on this machine.

mod common;

use std::fs;
use std::time::Duration;

use beam_core::discovery::{first_reachable, resolve};
use beam_core::{
    send_to, Advertiser, Browser, DiscoveryEvent, SendOptions, SendOutcome, TransferError,
    TrustStore, Visibility, VisibilityMode,
};
use common::*;

fn one_file(dir: &std::path::Path) -> std::path::PathBuf {
    let file = dir.join("a.txt");
    fs::write(&file, b"hello").unwrap();
    file
}

#[tokio::test]
async fn visibility_off_declines_everyone_without_asking() {
    let tmp = tempfile::tempdir().unwrap();
    let sender = device("phone");
    let mut receiver = TestReceiver::start_with(
        device("laptop"),
        trust_of(&[(&sender, true)]),
        &tmp.path().join("in"),
        Visibility::new(VisibilityMode::Off),
        |_| panic!("nobody is asked while visibility is off"),
    )
    .await;

    let sent = send_to(
        receiver.addr,
        &sender,
        &TrustStore::in_memory(),
        &[one_file(tmp.path())],
        &SendOptions::default(),
        |_| {},
    )
    .await
    .unwrap();
    assert!(
        matches!(&sent, SendOutcome::Rejected { reason } if reason.contains("not accepting")),
        "{sent:?}"
    );
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Rejected(_)
    ));
    assert!(!tmp.path().join("in").exists());
}

#[tokio::test]
async fn trusted_only_declines_unknown_senders_and_accepts_trusted_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let friend = device("friend's phone");
    let stranger = device("stranger");
    let mut receiver = TestReceiver::start_with(
        device("laptop"),
        trust_of(&[(&friend, false)]),
        &tmp.path().join("in"),
        Visibility::new(VisibilityMode::TrustedOnly),
        |offer| {
            assert!(
                offer.from.is_trusted(),
                "only trusted senders reach the prompt"
            );
            beam_core::Decision::Accept
        },
    )
    .await;
    let file = one_file(tmp.path());

    let from_stranger = send_to(
        receiver.addr,
        &stranger,
        &TrustStore::in_memory(),
        std::slice::from_ref(&file),
        &SendOptions::default(),
        |_| {},
    )
    .await
    .unwrap();
    assert!(
        matches!(&from_stranger, SendOutcome::Rejected { reason } if reason.contains("trusted")),
        "{from_stranger:?}"
    );
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Rejected(_)
    ));

    let from_friend = send_to(
        receiver.addr,
        &friend,
        &TrustStore::in_memory(),
        &[file],
        &SendOptions::default(),
        |_| {},
    )
    .await
    .unwrap();
    assert!(matches!(from_friend, SendOutcome::Completed(_)));
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Completed { .. }
    ));
}

#[tokio::test]
async fn everyone_falls_back_to_trusted_only_when_time_is_up() {
    let tmp = tempfile::tempdir().unwrap();
    let stranger = device("stranger");
    let visibility =
        Visibility::with_everyone_duration(VisibilityMode::Everyone, Duration::from_millis(300));
    let mut receiver = TestReceiver::start_with(
        device("laptop"),
        TrustStore::in_memory(),
        &tmp.path().join("in"),
        visibility.clone(),
        accept_all,
    )
    .await;
    let file = one_file(tmp.path());
    let (no_trust, defaults) = (TrustStore::in_memory(), SendOptions::default());
    let send = || {
        send_to(
            receiver.addr,
            &stranger,
            &no_trust,
            std::slice::from_ref(&file),
            &defaults,
            |_| {},
        )
    };

    assert!(matches!(send().await.unwrap(), SendOutcome::Completed(_)));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(visibility.mode(), VisibilityMode::TrustedOnly);
    assert!(matches!(
        send().await.unwrap(),
        SendOutcome::Rejected { .. }
    ));
    receiver.next_outcome().await;
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Rejected(_)
    ));
}

/// mDNS names are unauthenticated. When the sender expects a particular
/// device id (found by name), a different device answering is refused
/// before anything is offered.
#[tokio::test]
async fn reaching_a_different_device_than_expected_sends_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let impostor = device("Kartikay's Laptop");
    let real = device("Kartikay's Laptop");
    let receiver = TestReceiver::start(
        impostor,
        TrustStore::in_memory(),
        &tmp.path().join("in"),
        |_| panic!("the impostor must never see the offer"),
    )
    .await;

    let err = send_to(
        receiver.addr,
        &device("phone"),
        &TrustStore::in_memory(),
        &[one_file(tmp.path())],
        &SendOptions {
            expect_device: Some(real.device_id()),
            ..SendOptions::default()
        },
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(err, TransferError::UnexpectedPeer { .. }), "{err}");
    assert!(!tmp.path().join("in").exists());
}

/// Real mDNS on this machine: a receiver announces itself, a browser finds
/// it by name, and a file is sent to it. Needs multicast on the local
/// network (and, on Windows, permission through the firewall), so it only
/// runs on request:
///
///     cargo test -p beam-core --test discovery -- --ignored
#[tokio::test(flavor = "multi_thread")]
#[ignore = "uses real multicast DNS; run with --ignored"]
async fn real_mdns_finds_a_receiver_by_name_and_sends_to_it() {
    let tmp = tempfile::tempdir().unwrap();
    let laptop = device(&format!("beam-test-{}", std::process::id()));
    let receiver = TestReceiver::start(
        laptop.clone(),
        TrustStore::in_memory(),
        &tmp.path().join("in"),
        accept_all,
    )
    .await;
    let _advertiser = Advertiser::start(
        laptop.clone(),
        receiver.addr.port(),
        Visibility::new(VisibilityMode::Everyone),
    )
    .unwrap();

    let phone = device("phone");
    let found = resolve(
        &laptop.name,
        &phone,
        TrustStore::in_memory(),
        Duration::from_secs(10),
    )
    .await
    .expect("found by name");
    assert_eq!(found.device_id, laptop.device_id());
    println!("found {} at {:?}", found.name, found.addresses);

    // The receiver only listens on loopback, so use its port on 127.0.0.1
    // if the announced addresses aren't reachable from here.
    let addr = match first_reachable(&found.addresses).await {
        Some(addr) => addr,
        None => receiver.addr,
    };
    let sent = send_to(
        addr,
        &phone,
        &TrustStore::in_memory(),
        &[one_file(tmp.path())],
        &SendOptions {
            expect_device: Some(found.device_id),
            ..SendOptions::default()
        },
        |_| {},
    )
    .await
    .unwrap();
    assert!(matches!(sent, SendOutcome::Completed(_)));
}

/// Real mDNS: in trusted-only mode a stranger's browser never sees the
/// device, while a trusted device's browser recognizes it by name.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "uses real multicast DNS; run with --ignored"]
async fn real_mdns_trusted_only_is_invisible_to_strangers() {
    let laptop = device(&format!("beam-hidden-{}", std::process::id()));
    let friend = device("friend");
    let stranger = device("stranger");
    let _advertiser = Advertiser::start(
        laptop.clone(),
        9,
        Visibility::new(VisibilityMode::TrustedOnly),
    )
    .unwrap();

    let mut friend_view = Browser::start(&friend, trust_of(&[(&laptop, false)])).unwrap();
    let mut stranger_view = Browser::start(&stranger, TrustStore::in_memory()).unwrap();

    let seen_by_friend = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(DiscoveryEvent::Found(d)) = friend_view.next().await {
                if d.device_id == laptop.device_id() {
                    return d;
                }
            }
        }
    })
    .await
    .expect("the trusted device recognizes it");
    assert_eq!(seen_by_friend.name, laptop.name);

    let seen_by_stranger = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(DiscoveryEvent::Found(d)) = stranger_view.next().await {
                if d.device_id == laptop.device_id() || d.name == laptop.name {
                    return d;
                }
            }
        }
    })
    .await;
    assert!(seen_by_stranger.is_err(), "the stranger never sees it");
}
