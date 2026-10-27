//! beam-cli: send and receive files from a terminal.

use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use beam_core::{
    send_to, Decision, DeviceId, DeviceInfo, DeviceType, IncomingOffer, Progress, ReceiveOutcome,
    Receiver, ReceiverConfig, SendOutcome,
};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "beam-cli", version = version_line(), about = "Send and receive files with Beam")]
struct Cli {
    /// Device name shown to the other side (defaults to the computer name).
    #[arg(long, global = true)]
    name: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Wait for incoming transfers and accept them automatically.
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
    let device = local_device(cli.name);
    match cli.command {
        Command::Receive { port, bind, dir } => {
            receive(device, SocketAddr::new(bind, port), dir).await
        }
        Command::Send { to, files } => send(device, to, files).await,
    }
}

/// Identity placeholder until phase 2 derives the id from a static key.
fn local_device(name: Option<String>) -> DeviceInfo {
    let name = name
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "beam-cli".to_string());
    let random = uuid::Uuid::new_v4();
    let mut id = [0u8; 8];
    id.copy_from_slice(&random.as_bytes()[..8]);
    DeviceInfo {
        id: DeviceId(id),
        name,
        device_type: DeviceType::Laptop,
    }
}

async fn receive(device: DeviceInfo, addr: SocketAddr, dir: PathBuf) -> Result<()> {
    let config = ReceiverConfig {
        device,
        download_dir: dir.clone(),
    };
    let receiver = Receiver::bind(addr, config)
        .await
        .with_context(|| format!("could not listen on {addr}"))?;
    println!(
        "Listening on {} as \"{}\", saving to {}",
        receiver.local_addr()?,
        receiver.config().device.name,
        dir.display()
    );
    println!("Incoming transfers are accepted automatically. Press Ctrl+C to stop.");

    loop {
        let mut printer = ProgressPrinter::new("Receiving");
        let result = receiver
            .accept_transfer(auto_accept, |p| printer.update(p))
            .await;
        printer.finish();
        match result {
            Ok(ReceiveOutcome::Completed {
                from,
                files,
                summary,
            }) => {
                for file in &files {
                    println!("  saved {}", file.path.display());
                }
                println!(
                    "Received {} file(s), {} from \"{}\" in {:.1} s ({:.1} MB/s)",
                    summary.files,
                    format_bytes(summary.bytes),
                    from.name,
                    summary.duration.as_secs_f64(),
                    summary.megabytes_per_second()
                );
            }
            Ok(ReceiveOutcome::Rejected { from }) => {
                println!("Declined a transfer from \"{}\"", from.name);
            }
            Err(err) => eprintln!("Transfer failed: {err}"),
        }
    }
}

async fn auto_accept(offer: IncomingOffer) -> Decision {
    println!(
        "\"{}\" ({}) is sending {} file(s), {}. Accepting.",
        offer.from.name,
        offer.from.device_id,
        offer.files.len(),
        format_bytes(offer.total_size)
    );
    Decision::Accept
}

async fn send(device: DeviceInfo, to: SocketAddr, files: Vec<PathBuf>) -> Result<()> {
    let mut printer = ProgressPrinter::new("Sending");
    let result = send_to(to, &device, &files, |p| printer.update(p)).await;
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
}
