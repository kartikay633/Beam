//! beam-cli: send and receive files from a terminal.

use std::io::{BufRead, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use beam_core::{
    send_to, Decision, DeviceId, DeviceType, Identity, IncomingOffer, LocalDevice, PeerInfo,
    Progress, ReceiveOutcome, Receiver, ReceiverConfig, SendEvent, SendOutcome, TrustStore,
    TrustedDevice,
};
use clap::{Parser, Subcommand};

const IDENTITY_FILE: &str = "identity.json";
const TRUST_FILE: &str = "trusted_devices.json";
/// Files listed individually in the accept prompt before summarizing.
const PROMPT_FILE_LIMIT: usize = 10;

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
    },
    /// Send files to a receiver.
    Send {
        /// Receiver address, e.g. 127.0.0.1:9000.
        #[arg(long)]
        to: SocketAddr,
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
        Command::Receive { port, bind, dir } => {
            let device = local_device(&config_dir, cli.name)?;
            receive(device, trust, SocketAddr::new(bind, port), dir).await
        }
        Command::Send { to, files } => {
            let device = local_device(&config_dir, cli.name)?;
            send(device, trust, to, files).await
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
) -> Result<()> {
    let config = ReceiverConfig {
        device,
        download_dir: dir.clone(),
        trust: trust.clone(),
    };
    let receiver = Receiver::bind(addr, config)
        .await
        .with_context(|| format!("could not listen on {addr}"))?;
    let me = &receiver.config().device;
    println!(
        "Listening on {} as \"{}\" ({}), saving to {}",
        receiver.local_addr()?,
        me.name,
        me.device_id(),
        dir.display()
    );
    println!("Press Ctrl+C to stop.");

    loop {
        let mut printer = ProgressPrinter::new("Receiving");
        let trust = trust.clone();
        let result = receiver
            .accept_transfer(|offer| ask_to_accept(offer, trust), |p| printer.update(p))
            .await;
        printer.finish();
        match result {
            Ok(ReceiveOutcome::Completed {
                from,
                auto_accepted,
                files,
                summary,
            }) => {
                if auto_accepted {
                    println!(
                        "Accepted automatically: \"{}\" is trusted with auto-accept.",
                        from.name()
                    );
                }
                for file in &files {
                    println!("  saved {}", file.path.display());
                }
                println!(
                    "Received {} file(s), {} from \"{}\" in {:.1} s ({:.1} MB/s)",
                    summary.files,
                    format_bytes(summary.bytes),
                    from.name(),
                    summary.duration.as_secs_f64(),
                    summary.megabytes_per_second()
                );
            }
            Ok(ReceiveOutcome::Rejected { from }) => {
                println!("Declined the transfer from \"{}\".", from.name());
            }
            Err(err) => eprintln!("Transfer failed: {err}"),
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

async fn send(
    device: LocalDevice,
    trust: TrustStore,
    to: SocketAddr,
    files: Vec<PathBuf>,
) -> Result<()> {
    let mut printer = ProgressPrinter::new("Sending");
    let result = send_to(to, &device, &trust, &files, |event| match event {
        SendEvent::Connected(peer) => {
            println!(
                "Connected to \"{}\" ({}), encrypted.",
                peer.name(),
                peer.device_id()
            );
            println!("{}", trust_line(&peer));
            println!(
                "  Verification code: {}  (the receiver shows the same code)",
                format_code(&peer.verification_code)
            );
            println!("Waiting for the receiver to accept…");
        }
        SendEvent::Progress(p) => printer.update(p),
    })
    .await;
    printer.finish();
    match result.with_context(|| format!("sending to {to} failed"))? {
        SendOutcome::Completed(summary) => {
            println!(
                "Sent {} file(s), {} in {:.1} s ({:.1} MB/s)",
                summary.files,
                format_bytes(summary.bytes),
                summary.duration.as_secs_f64(),
                summary.megabytes_per_second()
            );
            Ok(())
        }
        SendOutcome::Rejected { reason } => {
            anyhow::bail!("the receiver declined the transfer: {reason}")
        }
    }
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

    fn finish(&mut self) {
        if self.printed {
            eprintln!();
        }
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
}
