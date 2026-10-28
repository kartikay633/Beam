//! The Kotlin-facing API, driven from Rust with a recording listener.

use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use beam_core_ffi::*;

/// Records every event; answers offers with `answer` (accept, trust).
struct Recorder {
    events: Mutex<Vec<String>>,
    answer: Option<(bool, bool)>,
    node: OnceLock<Weak<BeamNode>>,
    saved: Mutex<Vec<String>>,
    adverts: Mutex<Vec<Option<ServiceAdvert>>>,
}

impl Recorder {
    fn new(answer: Option<(bool, bool)>) -> Arc<Recorder> {
        Arc::new(Recorder {
            events: Mutex::default(),
            answer,
            node: OnceLock::new(),
            saved: Mutex::default(),
            adverts: Mutex::default(),
        })
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    fn push(&self, event: String) {
        self.events.lock().unwrap().push(event);
    }

    /// Waits until an event starting with `prefix` shows up.
    fn wait_for(&self, prefix: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(event) = self.events().into_iter().find(|e| e.starts_with(prefix)) {
                return event;
            }
            assert!(
                Instant::now() < deadline,
                "no {prefix:?} in {:?}",
                self.events()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl BeamListener for Recorder {
    fn device_found(&self, device: NearbyDevice) {
        self.push(format!("found {}", device.name));
    }
    fn device_lost(&self, device_id: String) {
        self.push(format!("lost {device_id}"));
    }
    fn offer_received(&self, offer: IncomingOffer) {
        self.push(format!(
            "offer {} from {}",
            offer.files.len(),
            offer.from_name
        ));
        let node = self.node.get().and_then(Weak::upgrade);
        if let (Some((accept, trust)), Some(node)) = (self.answer, node) {
            node.respond_offer(offer.offer_id, accept, trust).unwrap();
        }
    }
    fn offer_closed(&self, _offer_id: String) {
        self.push("offer closed".into());
    }
    fn transfer_started(&self, transfer: TransferInfo) {
        self.push(format!(
            "started {:?} {}",
            transfer.direction, transfer.peer_name
        ));
    }
    fn transfer_status(&self, _id: String, status: String, code: Option<String>) {
        self.push(format!("status {status} {}", code.unwrap_or_default()));
    }
    fn transfer_progress(&self, _id: String, done: u64, total: u64) {
        self.push(format!("progress {done}/{total}"));
    }
    fn transfer_finished(
        &self,
        _id: String,
        result: TransferResult,
        message: Option<String>,
        saved: Vec<String>,
    ) {
        self.saved.lock().unwrap().extend(saved);
        self.push(format!(
            "finished {result:?} {}",
            message.unwrap_or_default()
        ));
    }
    fn paired(&self, device: TrustedDevice) {
        self.push(format!("paired {}", device.name));
    }
    fn advertisement_changed(&self, advert: Option<ServiceAdvert>) {
        self.adverts.lock().unwrap().push(advert);
    }
}

fn node(
    dir: &std::path::Path,
    name: &str,
    kind: DeviceKind,
    listener: Arc<Recorder>,
) -> Arc<BeamNode> {
    node_with(dir, name, kind, listener, Discovery::Off)
}

fn node_with(
    dir: &std::path::Path,
    name: &str,
    kind: DeviceKind,
    listener: Arc<Recorder>,
    discovery: Discovery,
) -> Arc<BeamNode> {
    let node = BeamNode::new(
        NodeConfig {
            device_name: name.into(),
            device_kind: kind,
            identity_secret: generate_identity_secret(),
            data_dir: dir.join(name).join("data").display().to_string(),
            incoming_dir: dir.join(name).join("incoming").display().to_string(),
            visibility: Visibility::Everyone,
            port: 0,
            listen_address: Some("127.0.0.1".into()),
            discovery,
        },
        listener.clone(),
    )
    .unwrap();
    let _ = listener.node.set(Arc::downgrade(&node));
    node
}

/// Runs a `suspend`-style call the way Kotlin would: from outside the
/// node's own runtime.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Runtime::new().unwrap().block_on(future)
}

fn path_file(path: &std::path::Path, name: &str) -> OutgoingFile {
    OutgoingFile {
        name: name.into(),
        modified_unix: 0,
        source: FileSource::Path {
            path: path.display().to_string(),
        },
    }
}

#[test]
fn phone_sends_to_laptop_through_the_api() {
    let tmp = tempfile::tempdir().unwrap();
    let laptop_events = Recorder::new(Some((true, true)));
    let laptop = node(
        tmp.path(),
        "Laptop",
        DeviceKind::Laptop,
        laptop_events.clone(),
    );
    let port = laptop.start().unwrap();

    let phone_events = Recorder::new(None);
    let phone = node(tmp.path(), "Phone", DeviceKind::Phone, phone_events.clone());
    let photo = tmp.path().join("content-cache-1234");
    std::fs::write(&photo, vec![7u8; 5_000_000]).unwrap();

    let result = block_on(phone.send(
        "t-1".into(),
        format!("127.0.0.1:{port}"),
        vec![path_file(&photo, "holiday.jpg")],
    ))
    .unwrap();
    assert_eq!(result, TransferResult::Completed);

    laptop_events.wait_for("finished Completed");
    let saved = laptop_events.saved.lock().unwrap().clone();
    assert_eq!(saved.len(), 1);
    assert!(saved[0].ends_with("holiday.jpg"), "{saved:?}");
    assert_eq!(std::fs::read(&saved[0]).unwrap(), vec![7u8; 5_000_000]);

    // The sender saw the verification code prompt and progress.
    let events = phone_events.events();
    assert!(
        events
            .iter()
            .any(|e| e.starts_with("status Waiting for Laptop")),
        "{events:?}"
    );
    assert!(
        events.iter().any(|e| e == "progress 5000000/5000000"),
        "{events:?}"
    );
    // "Trust" was ticked: the phone is now trusted, with auto-accept.
    let trusted = laptop.trusted_devices();
    assert_eq!(trusted.len(), 1);
    assert_eq!(trusted[0].device_id, phone.device_id());
    assert!(trusted[0].auto_accept);
    laptop.stop();
}

#[test]
fn declining_reports_declined_and_trusts_nobody() {
    let tmp = tempfile::tempdir().unwrap();
    let laptop = node(
        tmp.path(),
        "Laptop",
        DeviceKind::Laptop,
        Recorder::new(Some((false, true))),
    );
    let port = laptop.start().unwrap();
    let phone = node(tmp.path(), "Phone", DeviceKind::Phone, Recorder::new(None));
    let file = tmp.path().join("a.txt");
    std::fs::write(&file, b"a").unwrap();
    let result = block_on(phone.send(
        "t-2".into(),
        format!("127.0.0.1:{port}"),
        vec![path_file(&file, "")],
    ))
    .unwrap();
    assert_eq!(result, TransferResult::Declined);
    assert!(laptop.trusted_devices().is_empty());
}

#[test]
fn identity_helpers() {
    let secret = generate_identity_secret();
    assert_eq!(secret.len(), 64);
    let id = device_id_for_secret(secret).unwrap();
    assert_eq!(id.len(), 19, "{id}");
    assert!(device_id_for_secret(vec![1, 2, 3]).is_err());
    assert!(parse_pairing_code("https://example.com".into()).is_err());
}

/// With platform discovery the app registers what the node says and hands
/// back what it resolves; the node decides what to show.
#[test]
fn platform_discovery_goes_through_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let laptop_events = Recorder::new(Some((true, false)));
    let laptop = node_with(
        dir.path(),
        "Laptop",
        DeviceKind::Laptop,
        laptop_events.clone(),
        Discovery::Platform,
    );
    let phone_events = Recorder::new(None);
    let phone = node_with(
        dir.path(),
        "Phone",
        DeviceKind::Phone,
        phone_events.clone(),
        Discovery::Platform,
    );
    let port = laptop.start().unwrap();
    phone.start().unwrap();

    let latest = |events: &Recorder| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(advert) = events.adverts.lock().unwrap().last() {
                return advert.clone();
            }
            assert!(Instant::now() < deadline, "no advertisement");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let advert = latest(&laptop_events).expect("everyone is announced");
    assert_eq!(advert.port, port);
    assert_eq!(advert.txt.get("name").map(String::as_str), Some("Laptop"));

    // What the phone's NsdManager would resolve, including its own service.
    let own = latest(&phone_events).unwrap();
    phone.service_found(own.instance_name, own.txt, vec!["127.0.0.1".into()], 1);
    phone.service_found(
        "beam-laptop".into(),
        advert.txt.clone(),
        vec!["127.0.0.1".into()],
        port,
    );
    assert_eq!(phone_events.wait_for("found"), "found Laptop");
    assert_eq!(
        phone_events
            .events()
            .iter()
            .filter(|e| e.starts_with("found"))
            .count(),
        1,
        "our own service is hidden"
    );

    // Sending by device id goes to the address the platform found.
    let file = dir.path().join("note.txt");
    std::fs::write(&file, b"via nsd").unwrap();
    let result = block_on(phone.send(
        "11111111-2222-3333-4444-555555555555".into(),
        laptop.device_id(),
        vec![path_file(&file, "note.txt")],
    ))
    .unwrap();
    assert_eq!(result, TransferResult::Completed);

    phone.service_lost("beam-laptop".into());
    assert_eq!(
        phone_events.wait_for("lost"),
        format!("lost {}", laptop.device_id())
    );

    laptop.set_visibility(Visibility::Off);
    let deadline = Instant::now() + Duration::from_secs(10);
    while latest(&laptop_events).is_some() {
        assert!(
            Instant::now() < deadline,
            "still announced after turning off"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
