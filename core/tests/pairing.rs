//! QR-code pairing (SPEC.md 6.3) and sending from already-open files.

mod common;

use std::fs;
use std::time::Duration;

use beam_core::{
    pair_with, send_sources, send_to, PairingCode, SendOptions, SendOutcome, SendSource,
    TransferError, TrustStore,
};
use common::*;

fn code_for(
    receiver: &TestReceiver,
    desktop: &beam_core::LocalDevice,
    token: [u8; 16],
) -> PairingCode {
    PairingCode {
        device_id: desktop.device_id(),
        public_key: *desktop.identity.public_key(),
        name: desktop.name.clone(),
        addresses: vec![receiver.addr],
        token,
    }
}

#[tokio::test]
async fn scanning_a_code_makes_both_sides_trust_each_other() {
    let tmp = tempfile::tempdir().unwrap();
    let desktop = device("Kartikay's PC");
    let desktop_trust = TrustStore::in_memory();
    let mut receiver = TestReceiver::start(
        desktop.clone(),
        desktop_trust.clone(),
        &tmp.path().join("in"),
        |_| panic!("a paired phone is accepted without asking"),
    )
    .await;
    let token = receiver.pairing.open();
    let code = code_for(&receiver, &desktop, token);
    // What the phone actually gets: the QR text.
    let scanned = PairingCode::parse(&code.to_uri()).unwrap();

    let phone = device("Kartikay's Pixel");
    let phone_trust = TrustStore::in_memory();
    let trusted = pair_with(&scanned, &phone, &phone_trust).await.unwrap();
    assert_eq!(trusted.device_id, desktop.device_id());
    assert!(trusted.auto_accept);

    let on_desktop = desktop_trust
        .get(&phone.device_id())
        .unwrap()
        .expect("phone trusted");
    assert_eq!(on_desktop.public_key, *phone.identity.public_key());
    assert!(on_desktop.auto_accept);

    // And now files flow without a prompt.
    let file = tmp.path().join("photo.jpg");
    fs::write(&file, b"jpeg bytes").unwrap();
    let sent = send_to(
        receiver.addr,
        &phone,
        &phone_trust,
        &[file],
        &SendOptions::default(),
        |_| {},
    )
    .await
    .unwrap();
    assert!(matches!(sent, SendOutcome::Completed(_)));
    assert!(matches!(
        receiver.next_outcome().await,
        Finished::Completed {
            auto_accepted: true,
            ..
        }
    ));
}

#[tokio::test]
async fn a_used_or_wrong_token_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let desktop = device("PC");
    let desktop_trust = TrustStore::in_memory();
    let mut receiver = TestReceiver::start(
        desktop.clone(),
        desktop_trust.clone(),
        &tmp.path().join("in"),
        accept_all,
    )
    .await;
    let token = receiver.pairing.open();

    let stranger = device("stranger");
    let wrong = pair_with(
        &code_for(&receiver, &desktop, [0; 16]),
        &stranger,
        &TrustStore::in_memory(),
    )
    .await;
    assert!(
        matches!(wrong, Err(TransferError::PairingRefused(_))),
        "{wrong:?}"
    );
    // (The address probe before connecting shows up as a refusal too.)
    loop {
        if receiver.next_refusal().await.contains("pairing refused") {
            break;
        }
    }

    let phone = device("phone");
    pair_with(
        &code_for(&receiver, &desktop, token),
        &phone,
        &TrustStore::in_memory(),
    )
    .await
    .unwrap();
    let again = pair_with(
        &code_for(&receiver, &desktop, token),
        &device("third"),
        &TrustStore::in_memory(),
    )
    .await;
    assert!(
        matches!(again, Err(TransferError::PairingRefused(_))),
        "single use"
    );

    let trusted: Vec<_> = desktop_trust
        .list()
        .unwrap()
        .into_iter()
        .map(|d| d.device_id)
        .collect();
    assert_eq!(trusted, [phone.device_id()]);
}

#[tokio::test]
async fn a_code_for_a_different_key_is_not_paired_with_whoever_answers() {
    let tmp = tempfile::tempdir().unwrap();
    let impostor = device("Kartikay's PC");
    let receiver = TestReceiver::start(
        impostor,
        TrustStore::in_memory(),
        &tmp.path().join("in"),
        accept_all,
    )
    .await;
    let token = receiver.pairing.open();
    let real_desktop = device("Kartikay's PC");
    let phone_trust = TrustStore::in_memory();

    let result = pair_with(
        &code_for(&receiver, &real_desktop, token),
        &device("phone"),
        &phone_trust,
    )
    .await;
    assert!(
        matches!(result, Err(TransferError::UnexpectedPeer { .. })),
        "{result:?}"
    );
    assert!(phone_trust.list().unwrap().is_empty());
}

/// Android hands Beam open file descriptors, not paths: the file is sent
/// under the name the app gives it.
#[tokio::test(flavor = "multi_thread")]
async fn an_open_file_is_sent_under_its_given_name() {
    let tmp = tempfile::tempdir().unwrap();
    let on_disk = tmp.path().join("content-provider-cache-1234");
    write_random_file(&on_disk, 9 * 1024 * 1024 + 17, 5); // three chunks
    let inbox = tmp.path().join("inbox");
    let mut receiver =
        TestReceiver::start(device("pc"), TrustStore::in_memory(), &inbox, accept_all).await;

    let source = SendSource::open_file(
        fs::File::open(&on_disk).unwrap(),
        "IMG_0042.jpg",
        1_790_000_000,
    );
    let sent = tokio::time::timeout(
        Duration::from_secs(60),
        send_sources(
            receiver.addr,
            &device("phone"),
            &TrustStore::in_memory(),
            vec![source],
            &SendOptions::default(),
            |_| {},
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(sent, SendOutcome::Completed(_)));
    receiver.next_outcome().await;
    assert_eq!(hash_file(&on_disk), hash_file(&inbox.join("IMG_0042.jpg")));
}
