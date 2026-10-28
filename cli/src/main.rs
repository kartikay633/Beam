//! beam-cli: send and receive files from a terminal.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use beam_core::discovery::{first_reachable, resolve};
use beam_core::{pair_with, PairingCode, PairingWindow};
use beam_core::{
    send_to, Advertiser, BoxFuture, Browser, Decision, DeviceId, DeviceType, DiscoveryEvent,
    Identity, IncomingOffer, LocalDevice, NearbyDevice, PeerInfo, Progress, ReceiveHandler,
    ReceiveOutcome, Receiver, ReceiverConfig, SendEvent, SendOptions, SendOutcome, TransferError,
    TransferSummary, TrustStore, TrustedDevice, Visibility, VisibilityMode,
};
use clap::{Parser, Subcommand};
use uuid::Uuid;

const IDENTITY_FILE: &str = "identity.json";
const TRUST_FILE: &str = "trusted_devices.json";
/// Files listed individually in the accept prompt before summarizing.
const PROMPT_FILE_LIMIT: usize = 10;
/// How long `send --to <name>` looks for the device on the network.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(name = "beam-cli", version = version_line(), about = "Send and receive files with Beam")]
struct Cli {
    /// Device name shown to the other side (defaults to the computer name).
    #[arg(long, global = true)]
    name: Option<String>,

    /// Folder for this device's key and trusted devices
    /// (default: the system config folder, e.g. %APPDATA%\Beam).
    #[arg(long, global = true, env = "BEAM_CONFIG_DIR")]
    config_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Wait for incoming transfers. Each one is confirmed on this terminal
    /// unless it comes from a trusted device with auto-accept on.
    Receive {
        /// TCP port to listen on.
        #[arg(long, default_value_t = 9000)]
        port: u16,
        /// Local address to listen on (0.0.0.0 = every network interface).
        #[arg(long, default_value = "0.0.0.0")]
        bind: IpAddr,
        /// Folder to save received files into.
        #[arg(long, default_value = "./inbox")]
        dir: PathBuf,
        /// Who can see this device and send to it: everyone (for 10 minutes,
        /// then trusted), trusted, or off.
        #[arg(long, default_value = "everyone")]
        visibility: VisibilityMode,
        /// Print a pairing code another device can use with `beam-cli pair`.
        #[arg(long)]
        pair: bool,
    },
    /// Pair with a device that showed a pairing code (`receive --pair`, or
    /// the desktop app's QR code), so the two trust each other.
    Pair {
        /// The `beam://pair?…` code.
        code: String,
    },
    /// List Beam devices on the local network as they appear and disappear.
    Discover {
        /// Stop after this many seconds (default: run until Ctrl+C).
        #[arg(long)]
        seconds: Option<u64>,
    },
    /// Send files to a receiver.
    Send {
        /// Receiver: a device name or id found on the local network
        /// (see `discover`), or an address such as 192.168.1.20:9000.
        #[arg(long)]
        to: String,
        /// Parallel data connections.
        #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u16).range(1..=16))]
        streams: u16,
        /// Print the final statistics as one line of JSON on stdout.
        #[arg(long)]
        json: bool,
        /// Files to send.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Show this device's id (fingerprint) and public key.
    Identity,
    /// Manage trusted devices.
    Trust {
        #[command(subcommand)]
        command: TrustCommand,
    },
}

#[derive(Subcommand)]
enum TrustCommand {
    /// List trusted devices.
    List,
    /// Trust a device by its public key (as shown by `beam-cli identity`).
    Add {
        /// The device's 64-hex-digit public key.
        public_key: String,
        /// Name to remember it by.
        #[arg(long)]
        name: String,
        /// Accept its transfers without asking.
        #[arg(long)]
        auto_accept: bool,
    },
    /// Stop trusting a device.
    Remove {
        /// Device id, e.g. 0123-4567-89ab-cdef.
        id: DeviceId,
    },
}

fn version_line() -> &'static str {
    static LINE: OnceLock<String> = OnceLock::new();
    LINE.get_or_init(|| {
        format!(
            "{} (beam-core {})",
            env!("CARGO_PKG_VERSION"),
            beam_core::VERSION
        )
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_dir = match cli.config_dir {
        Some(dir) => dir,
        None => dirs::config_dir()
            .context("could not find the system config folder; pass --config-dir")?
            .join("Beam"),
    };
    let trust = TrustStore::open(&config_dir.join(TRUST_FILE))?;

    match cli.command {
        Command::Receive {
            port,
            bind,
            dir,
            visibility,
            pair,
        } => {
            let device = local_device(&config_dir, cli.name)?;
            receive(
                device,
                trust,
                SocketAddr::new(bind, port),
                dir,
                visibility,
                pair,
            )
            .await
        }
        Command::Pair { code } => {
            let device = local_device(&config_dir, cli.name)?;
            let code = PairingCode::parse(code.trim())?;
            eprintln!("Pairing with \"{}\" ({})…", code.name, code.device_id);
            let trusted = pair_with(&code, &device, &trust).await?;
            println!(
                "Paired: \"{}\" ({}) and this device now trust each other.",
                trusted.name, trusted.device_id
            );
            Ok(())
        }
        Command::Discover { seconds } => {
            let device = local_device(&config_dir, cli.name)?;
            discover(device, trust, seconds.map(Duration::from_secs)).await
        }
        Command::Send {
            to,
            streams,
            json,
            files,
        } => {
            let device = local_device(&config_dir, cli.name)?;
            let options = SendOptions {
                streams: streams as usize,
                ..SendOptions::default()
            };
            send(device, trust, to, files, options, json).await
        }
        Command::Identity => {
            let device = local_device(&config_dir, cli.name)?;
            println!("Name:       {}", device.name);
            println!("Device id:  {}", device.device_id());
            println!("Public key: {}", hex_key(device.identity.public_key()));
            println!("Stored in:  {}", config_dir.join(IDENTITY_FILE).display());
            Ok(())
        }
        Command::Trust { command } => trust_command(&trust, command),
    }
}

fn local_device(config_dir: &Path, name: Option<String>) -> Result<LocalDevice> {
    let identity = Identity::load_or_create(&config_dir.join(IDENTITY_FILE))?;
    let name = name
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "beam-cli".to_string());
    Ok(LocalDevice {
        identity,
        name,
        device_type: DeviceType::Laptop,
    })
}

fn trust_command(trust: &TrustStore, command: TrustCommand) -> Result<()> {
    match command {
        TrustCommand::List => {
            let devices = trust.list()?;
            if devices.is_empty() {
                println!("No trusted devices yet. Answer \"t\" when receiving to trust a sender.");
                return Ok(());
            }
            println!("{:<21}{:<12}NAME", "DEVICE ID", "AUTO-ACCEPT");
            for device in devices {
                let auto = if device.auto_accept { "yes" } else { "no" };
                println!(
                    "{:<21}{:<12}{}",
                    device.device_id.to_string(),
                    auto,
                    device.name
                );
            }
            Ok(())
        }
        TrustCommand::Add {
            public_key,
            name,
            auto_accept,
        } => {
            let key: [u8; 32] = parse_hex(&public_key)
                .and_then(|bytes| bytes.try_into().ok())
                .context("a public key is 64 hex digits (see `beam-cli identity`)")?;
            let device = TrustedDevice::new(name, key, auto_accept);
            println!("Trusted \"{}\" ({}).", device.name, device.device_id);
            trust.upsert(device)?;
            Ok(())
        }
        TrustCommand::Remove { id } => {
            if trust.remove(&id)? {
                println!("Removed {id} from trusted devices.");
                Ok(())
            } else {
                anyhow::bail!("{id} is not a trusted device (see `beam-cli trust list`)")
            }
        }
    }
}

async fn receive(
    device: LocalDevice,
    trust: TrustStore,
    addr: SocketAddr,
    dir: PathBuf,
    mode: VisibilityMode,
    show_pairing: bool,
) -> Result<()> {
    let visibility = Visibility::new(mode);
    let pairing = PairingWindow::default();
    let config = ReceiverConfig {
        device: device.clone(),
        download_dir: dir.clone(),
        trust: trust.clone(),
        visibility: visibility.clone(),
        pairing: pairing.clone(),
    };
    let receiver = Receiver::bind(addr, config)
        .await
        .with_context(|| format!("could not listen on {addr}"))?;
    let port = receiver.local_addr()?.port();
    let me = &receiver.config().device;
    println!(
        "Listening on {} as \"{}\" ({}), saving to {}",
        receiver.local_addr()?,
        me.name,
        me.device_id(),
        dir.display()
    );
    let _advertiser = Advertiser::start(device.clone(), port, visibility.clone())
        .context("could not announce this device on the local network")?;
    println!("{}", describe_visibility(mode));
    if show_pairing {
        let code = PairingCode {
            device_id: device.device_id(),
            public_key: *device.identity.public_key(),
            name: device.name.clone(),
            addresses: lan_addresses(port),
            token: pairing.open(),
        };
        println!(
            "Pairing code (valid for 5 minutes, one device):\n  {}",
            code.to_uri()
        );
        println!("On the other device: beam-cli pair \"<code>\"");
    }
    tokio::spawn(report_visibility_expiry(visibility));
    println!("Press Ctrl+C to stop.");
    receiver
        .run(CliHandler {
            trust,
            prompt: Arc::new(tokio::sync::Mutex::new(())),
            printers: Mutex::default(),
        })
        .await?;
    Ok(())
}

struct CliHandler {
    trust: TrustStore,
    /// One question on the terminal at a time.
    prompt: Arc<tokio::sync::Mutex<()>>,
    printers: Mutex<HashMap<Uuid, ProgressPrinter>>,
}

impl ReceiveHandler for CliHandler {
    fn decide(&self, offer: IncomingOffer) -> BoxFuture<Decision> {
        let (trust, prompt) = (self.trust.clone(), self.prompt.clone());
        Box::pin(async move {
            let _turn = prompt.lock().await;
            ask_to_accept(offer, trust).await
        })
    }

    fn progress(&self, transfer_id: Uuid, progress: Progress) {
        self.printers
            .lock()
            .unwrap()
            .entry(transfer_id)
            .or_insert_with(|| ProgressPrinter::new("Receiving"))
            .update(progress);
    }

    fn finished(&self, transfer_id: Uuid, result: &Result<ReceiveOutcome, TransferError>) {
        if let Some(mut printer) = self.printers.lock().unwrap().remove(&transfer_id) {
            printer.finish();
        }
        match result {
            Ok(ReceiveOutcome::Completed {
                from,
                auto_accepted,
                resumed,
                files,
                summary,
            }) => {
                if *resumed {
                    println!("Resumed an interrupted transfer from \"{}\".", from.name());
                } else if *auto_accepted {
                    println!(
                        "Accepted automatically: \"{}\" is trusted with auto-accept.",
                        from.name()
                    );
                }
                for file in files.iter().take(PROMPT_FILE_LIMIT) {
                    println!("  saved {}", file.path.display());
                }
                if files.len() > PROMPT_FILE_LIMIT {
                    println!("  … and {} more", files.len() - PROMPT_FILE_LIMIT);
                }
                println!(
                    "Received {} file(s), {} from \"{}\" in {:.1} s ({:.1} MB/s, {} stream(s))",
                    summary.files,
                    format_bytes(summary.bytes),
                    from.name(),
                    summary.duration.as_secs_f64(),
                    summary.megabytes_per_second(),
                    summary.streams
                );
            }
            Ok(ReceiveOutcome::Rejected { from, reason }) => {
                println!("Declined the transfer from \"{}\" ({reason}).", from.name());
            }
            Err(err) if err.is_retryable() || matches!(err, TransferError::Superseded) => {
                eprintln!("Transfer interrupted ({err}); kept the partial files for resuming.");
            }
            Err(err) => eprintln!("Transfer failed: {err}"),
        }
    }

    fn connection_refused(&self, error: &TransferError) {
        // Dropped connections are routine (reconnects, port scanners); only
        // identity problems deserve the user's attention.
        if matches!(
            error,
            TransferError::IdentityMismatch { .. } | TransferError::TrustedKeyMismatch { .. }
        ) {
            eprintln!("Refused a connection: {error}");
        }
    }
}

/// Shows the offer and asks on the terminal. Answering "t" also trusts the
/// sender and enables auto-accept for it.
async fn ask_to_accept(offer: IncomingOffer, trust: TrustStore) -> Decision {
    let from = &offer.from;
    println!();
    println!(
        "\"{}\" ({}) wants to send {} file(s), {}:",
        from.name(),
        from.device_id(),
        offer.files.len(),
        format_bytes(offer.total_size)
    );
    for file in offer.files.iter().take(PROMPT_FILE_LIMIT) {
        println!("    {} ({})", file.name, format_bytes(file.size));
    }
    if offer.files.len() > PROMPT_FILE_LIMIT {
        println!("    … and {} more", offer.files.len() - PROMPT_FILE_LIMIT);
    }
    println!("{}", trust_line(from));
    println!(
        "  Verification code: {}  (the sender shows the same code)",
        format_code(&from.verification_code)
    );
    print!("Accept? [y]es / [n]o / [t]rust this device and auto-accept from now on: ");
    let _ = std::io::stdout().flush();

    let answer = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).map(|_| line)
    })
    .await;
    let answer = match answer {
        Ok(Ok(line)) => line.trim().to_ascii_lowercase(),
        _ => String::new(),
    };

    match answer.as_str() {
        "y" | "yes" => Decision::Accept,
        "t" | "trust" => {
            let device = TrustedDevice::new(from.name().to_string(), from.public_key, true);
            match trust.upsert(device) {
                Ok(()) => println!("Trusted \"{}\" with auto-accept.", from.name()),
                Err(err) => eprintln!("Could not save the trusted device: {err}"),
            }
            Decision::Accept
        }
        "" => Decision::Reject("no answer".to_string()),
        _ => Decision::Reject("declined by user".to_string()),
    }
}

/// This machine's non-loopback addresses with `port`, IPv4 first.
fn lan_addresses(port: u16) -> Vec<SocketAddr> {
    let mut ips: Vec<IpAddr> = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            // Asks the OS which local address would reach the outside world;
            // nothing is sent.
            socket.connect("192.0.2.1:9")?;
            socket.local_addr()
        })
        .map(|a| vec![a.ip()])
        .unwrap_or_default();
    ips.retain(|ip| !ip.is_loopback() && !ip.is_unspecified());
    ips.push(IpAddr::from([127, 0, 0, 1]));
    ips.into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect()
}

fn describe_visibility(mode: VisibilityMode) -> &'static str {
    match mode {
        VisibilityMode::Everyone => {
            "Visible to everyone nearby for 10 minutes, then to trusted devices only."
        }
        VisibilityMode::TrustedOnly => {
            "Visible to trusted devices only; offers from other devices are declined."
        }
        VisibilityMode::Off => "Not visible, and declining every offer (--visibility off).",
    }
}

/// Tells the user when "everyone" falls back to trusted-only.
async fn report_visibility_expiry(visibility: Visibility) {
    if let Some(until) = visibility.everyone_until() {
        tokio::time::sleep_until(until).await;
        if visibility.mode() == VisibilityMode::TrustedOnly {
            println!("\n{}", describe_visibility(VisibilityMode::TrustedOnly));
        }
    }
}

/// Prints devices as they come and go, until `limit` (or forever).
async fn discover(device: LocalDevice, trust: TrustStore, limit: Option<Duration>) -> Result<()> {
    let mut browser = Browser::start(&device, trust)?;
    println!("Looking for Beam devices on the local network… (Ctrl+C to stop)");
    let deadline = limit.map(|d| tokio::time::Instant::now() + d);
    // What was last printed for each device: mDNS re-announces a device as
    // more of its addresses resolve, so only print real changes.
    let mut seen: HashMap<DeviceId, (NearbyDevice, String)> = HashMap::new();
    loop {
        let next = match deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, browser.next()).await {
                Ok(event) => event,
                Err(_) => break,
            },
            None => browser.next().await,
        };
        match next {
            Some(DiscoveryEvent::Found(found)) => {
                let line = describe_device(&found);
                match seen.get(&found.device_id) {
                    Some((_, printed)) if *printed == line => {}
                    Some(_) => println!("~ {line}"),
                    None => println!("+ {line}"),
                }
                seen.insert(found.device_id, (found, line));
            }
            Some(DiscoveryEvent::Lost(id)) => {
                if let Some((lost, _)) = seen.remove(&id) {
                    println!("- {} ({}) left", lost.name, lost.device_id);
                }
            }
            None => break,
        }
    }
    if seen.is_empty() {
        println!("No devices found.");
    }
    Ok(())
}

fn describe_device(device: &NearbyDevice) -> String {
    let kind = match device.device_type {
        Some(DeviceType::Phone) => "phone",
        Some(DeviceType::Laptop) => "laptop",
        None => "device",
    };
    let trust = if device.trusted.is_some() {
        "trusted"
    } else {
        "not trusted"
    };
    let address = device
        .addresses
        .first()
        .map_or_else(|| "no address".to_string(), |a| a.to_string());
    format!(
        "{:<24} {:<7} {}  {:<22} {}",
        device.name, kind, device.device_id, address, trust
    )
}

/// Turns `--to` into an address. A device name or id is looked up with
/// mDNS, and the transfer is then pinned to that device id.
async fn find_receiver(
    to: &str,
    device: &LocalDevice,
    trust: &TrustStore,
    options: &mut SendOptions,
) -> Result<SocketAddr> {
    if let Ok(addr) = to.parse::<SocketAddr>() {
        return Ok(addr);
    }
    eprintln!("Looking for \"{to}\" on the local network…");
    let found = resolve(to, device, trust.clone(), RESOLVE_TIMEOUT).await?;
    let addr = first_reachable(&found.addresses)
        .await
        .ok_or_else(|| beam_core::DiscoveryError::Unreachable(found.name.clone()))?;
    eprintln!("Found \"{}\" ({}) at {addr}.", found.name, found.device_id);
    options.expect_device = Some(found.device_id);
    Ok(addr)
}

async fn send(
    device: LocalDevice,
    trust: TrustStore,
    to: String,
    files: Vec<PathBuf>,
    mut options: SendOptions,
    json: bool,
) -> Result<()> {
    let to = find_receiver(&to, &device, &trust, &mut options).await?;
    let mut printer = ProgressPrinter::new("Sending");
    let result = send_to(to, &device, &trust, &files, &options, |event| match event {
        SendEvent::Connected(peer) => {
            eprintln!(
                "Connected to \"{}\" ({}), encrypted.",
                peer.name(),
                peer.device_id()
            );
            eprintln!("{}", trust_line(&peer));
            eprintln!(
                "  Verification code: {}  (the receiver shows the same code)",
                format_code(&peer.verification_code)
            );
            eprintln!("Waiting for the receiver to accept…");
        }
        SendEvent::Accepted { resumed_bytes } if resumed_bytes > 0 => {
            printer.interrupt();
            eprintln!(
                "Resuming: the receiver already has {}.",
                format_bytes(resumed_bytes)
            );
        }
        SendEvent::Accepted { .. } => {}
        SendEvent::Progress(p) => printer.update(p),
        SendEvent::Reconnecting { attempt, error } => {
            printer.interrupt();
            eprintln!("Connection lost ({error}); reconnecting (attempt {attempt})…");
        }
    })
    .await;
    printer.finish();
    match result.with_context(|| format!("sending to {to} failed"))? {
        SendOutcome::Completed(summary) => {
            eprintln!(
                "Sent {} file(s), {} in {:.1} s ({:.1} MB/s, peak {:.1} MB/s, {} stream(s){})",
                summary.files,
                format_bytes(summary.bytes),
                summary.duration.as_secs_f64(),
                summary.megabytes_per_second(),
                summary.peak_mb_per_s,
                summary.streams,
                match summary.reconnects {
                    0 => String::new(),
                    n => format!(", {n} reconnect(s)"),
                }
            );
            if json {
                println!("{}", summary_json(&summary));
            }
            Ok(())
        }
        SendOutcome::Rejected { reason } => {
            anyhow::bail!("the receiver declined the transfer: {reason}")
        }
    }
}

/// One line of JSON for scripts (the benchmark runner).
fn summary_json(s: &TransferSummary) -> String {
    format!(
        concat!(
            "{{\"files\":{},\"bytes\":{},\"seconds\":{:.3},\"mb_per_s\":{:.2},",
            "\"peak_mb_per_s\":{:.2},\"setup_ms\":{},\"streams\":{},\"reconnects\":{},",
            "\"resent_chunks\":{},\"link\":\"tcp\"}}"
        ),
        s.files,
        s.bytes,
        s.duration.as_secs_f64(),
        s.megabytes_per_second(),
        s.peak_mb_per_s,
        s.setup.as_millis(),
        s.streams,
        s.reconnects,
        s.resent_chunks
    )
}

fn trust_line(peer: &PeerInfo) -> &'static str {
    if peer.is_trusted() {
        "  Trusted device: its key matches the one you trusted."
    } else {
        "  Unknown device: compare the verification code on both screens."
    }
}

/// "123456" → "123 456", easier to read aloud.
fn format_code(code: &str) -> String {
    if code.len() == 6 {
        format!("{} {}", &code[..3], &code[3..])
    } else {
        code.to_string()
    }
}

fn hex_key(key: &[u8]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Prints a single, self-updating progress line at most a few times a second.
struct ProgressPrinter {
    verb: &'static str,
    started: Option<Instant>,
    last_print: Option<Instant>,
    printed: bool,
}

impl ProgressPrinter {
    const INTERVAL: Duration = Duration::from_millis(200);

    fn new(verb: &'static str) -> Self {
        ProgressPrinter {
            verb,
            started: None,
            last_print: None,
            printed: false,
        }
    }

    fn update(&mut self, progress: Progress) {
        let now = Instant::now();
        let started = *self.started.get_or_insert(now);
        let finished = progress.bytes_done == progress.bytes_total
            && progress.files_done == progress.files_total;
        if !finished && self.last_print.is_some_and(|t| now - t < Self::INTERVAL) {
            return;
        }
        self.last_print = Some(now);
        self.printed = true;

        let elapsed = (now - started).as_secs_f64();
        let speed = if elapsed > 0.0 {
            progress.bytes_done as f64 / 1_000_000.0 / elapsed
        } else {
            0.0
        };
        let percent = if progress.bytes_total == 0 {
            100.0
        } else {
            progress.bytes_done as f64 * 100.0 / progress.bytes_total as f64
        };
        eprint!(
            "\r{} {:5.1}%  {} / {}  file {}/{}  {:.1} MB/s   ",
            self.verb,
            percent,
            format_bytes(progress.bytes_done),
            format_bytes(progress.bytes_total),
            progress.files_done,
            progress.files_total,
            speed
        );
        let _ = std::io::stderr().flush();
    }

    /// Ends the current progress line so a message can be printed.
    fn interrupt(&mut self) {
        if std::mem::take(&mut self.printed) {
            eprintln!();
        }
    }

    fn finish(&mut self) {
        self.interrupt();
    }
}

/// Formats a byte count with decimal units (1 MB = 1,000,000 bytes), matching
/// the MB/s figures.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn version_names_both_crates() {
        assert!(version_line().contains("beam-core "));
    }

    #[test]
    fn bytes_are_formatted_with_decimal_units() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1_500), "1.5 KB");
        assert_eq!(format_bytes(200_000_000), "200.0 MB");
        assert_eq!(format_bytes(2_300_000_000), "2.3 GB");
    }

    #[test]
    fn codes_are_split_for_reading() {
        assert_eq!(format_code("042137"), "042 137");
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(parse_hex("0"), None);
        assert_eq!(parse_hex("zz"), None);
    }

    #[test]
    fn summary_json_is_one_valid_looking_line() {
        let json = summary_json(&TransferSummary {
            files: 2,
            bytes: 10,
            streams: 4,
            ..TransferSummary::default()
        });
        assert!(json.starts_with("{\"files\":2,\"bytes\":10,"));
        assert!(json.contains("\"streams\":4"));
        assert!(!json.contains('\n'));
    }
}
