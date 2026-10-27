"""Run the Beam benchmarks (SPEC.md section 12).

For each test set and each stream count, sends the set several times with
beam-cli and records every run in raw_results.csv and the median per scenario
in results.csv.

Scenarios:
  loopback  Starts a receiver on 127.0.0.1 itself. Measures the software
            (crypto, hashing, disk), not the network.
  lan       Sends to a receiver you started on another machine (--to). On that
            machine, first trust this sender with auto-accept; the runner
            prints the exact command.

Usage:
    python bench/run.py --data-dir D:/beam-bench-data --work-dir D:/beam-bench-work
    python bench/run.py --scenario lan --to 192.168.1.20:9000 \\
        --sender-device "ThinkPad X1" --receiver-device "Pixel 8" --wifi-band 5 --distance-m 3

Always record the device models, Wi-Fi band and distance for LAN runs.
Honest numbers only.
"""

import argparse
import csv
import json
import os
import shutil
import statistics
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

try:
    import psutil  # optional: peak memory measurements
except ImportError:
    psutil = None

BENCH_DIR = Path(__file__).resolve().parent
REPO = BENCH_DIR.parent
RAW_FIELDS = [
    "timestamp", "scenario", "link", "test_set", "streams", "run", "files", "bytes",
    "seconds", "mb_per_s", "peak_mb_per_s", "setup_ms", "reconnects",
    "sender_peak_rss_mb", "receiver_peak_rss_mb",
    "sender_device", "receiver_device", "wifi_band_ghz", "distance_m", "build",
]
SUMMARY_FIELDS = [
    "timestamp", "scenario", "link", "test_set", "streams", "runs", "files", "bytes",
    "median_mb_per_s", "min_mb_per_s", "max_mb_per_s", "median_setup_ms",
    "max_sender_rss_mb", "max_receiver_rss_mb",
    "sender_device", "receiver_device", "wifi_band_ghz", "distance_m", "build",
]


def default_cli() -> Path:
    exe = "beam-cli.exe" if os.name == "nt" else "beam-cli"
    target = os.environ.get("CARGO_TARGET_DIR", str(REPO / "target"))
    return Path(target) / "release" / exe


class PeakMemory:
    """Polls a process's resident memory in the background."""

    def __init__(self, pid: int):
        self.peak = 0
        self._stop = threading.Event()
        self._proc = psutil.Process(pid) if psutil else None
        self._thread = threading.Thread(target=self._poll, daemon=True)
        self._thread.start()

    def _poll(self) -> None:
        while self._proc and not self._stop.is_set():
            try:
                self.peak = max(self.peak, self._proc.memory_info().rss)
            except psutil.Error:
                return
            time.sleep(0.02)

    def stop_mb(self) -> str:
        self._stop.set()
        self._thread.join()
        return f"{self.peak / 1e6:.1f}" if self._proc else ""


def beam(cli: Path, config: Path, *args: str, **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run([str(cli), "--config-dir", str(config), *args],
                          check=True, capture_output=True, text=True, **kwargs)


def public_key(cli: Path, config: Path) -> str:
    out = beam(cli, config, "identity").stdout
    return next(line.split(":", 1)[1].strip() for line in out.splitlines()
                if line.startswith("Public key:"))


def start_receiver(cli: Path, config: Path, inbox: Path, port: int) -> subprocess.Popen:
    receiver = subprocess.Popen(
        [str(cli), "--config-dir", str(config), "--name", "bench-receiver", "receive",
         "--bind", "127.0.0.1", "--port", str(port), "--dir", str(inbox)],
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    line = receiver.stdout.readline()
    if "Listening" not in line:
        receiver.kill()
        sys.exit(f"receiver did not start: {line!r}")
    # Keep draining its output so it never blocks on a full pipe.
    threading.Thread(target=lambda: receiver.stdout.read(), daemon=True).start()
    return receiver


def files_of(data_dir: Path, test_set: str) -> list:
    folder = data_dir / test_set
    files = sorted(p for p in folder.iterdir() if p.is_file()) if folder.is_dir() else []
    if not files:
        sys.exit(f"no files in {folder}; run bench/generate.py first")
    return files


def send_once(cli: Path, config: Path, to: str, streams: int, files: list) -> tuple:
    # Run inside the set's folder and pass bare file names: 1,000 absolute
    # paths would overflow Windows' 32,767-character command line.
    folder = files[0].parent
    sender = subprocess.Popen(
        [str(cli), "--config-dir", str(config.resolve()), "--name", "bench-sender", "send",
         "--to", to, "--streams", str(streams), "--json", *(f.name for f in files)],
        cwd=folder, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    memory = PeakMemory(sender.pid)
    stdout, stderr = sender.communicate()
    rss = memory.stop_mb()
    if sender.returncode != 0:
        sys.exit(f"send failed:\n{stderr}")
    return json.loads(stdout.strip().splitlines()[-1]), rss


def append_rows(path: Path, fields: list, rows: list) -> None:
    new = not path.exists()
    with open(path, "a", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=fields)
        if new:
            writer.writeheader()
        writer.writerows(rows)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--scenario", choices=["loopback", "lan"], default="loopback")
    parser.add_argument("--to", help="receiver address for --scenario lan, e.g. 192.168.1.20:9000")
    parser.add_argument("--cli", type=Path, default=default_cli(), help="path to beam-cli (release build)")
    parser.add_argument("--data-dir", type=Path, default=BENCH_DIR / "data")
    parser.add_argument("--work-dir", type=Path, default=BENCH_DIR / "work",
                        help="scratch space for identities and the loopback inbox")
    parser.add_argument("--sets", nargs="+", default=["big", "medium", "small"])
    parser.add_argument("--streams", nargs="+", type=int, default=[1, 2, 4, 8])
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--port", type=int, default=9100)
    parser.add_argument("--sender-device", default="")
    parser.add_argument("--receiver-device", default="")
    parser.add_argument("--wifi-band", default="", help="2.4 or 5 (GHz), for LAN runs")
    parser.add_argument("--distance-m", default="", help="distance between devices, for LAN runs")
    parser.add_argument("--out-dir", type=Path, default=BENCH_DIR)
    args = parser.parse_args()

    if not args.cli.exists():
        sys.exit(f"{args.cli} not found; build it with: cargo build --release -p beam-cli")
    if args.scenario == "lan" and not args.to:
        sys.exit("--scenario lan needs --to HOST:PORT")
    build = subprocess.run([str(args.cli), "--version"], capture_output=True, text=True).stdout.strip()

    sender_config = args.work_dir / "sender"
    receiver_config = args.work_dir / "receiver"
    inbox = args.work_dir / "inbox"
    key = public_key(args.cli, sender_config)

    receiver = None
    if args.scenario == "loopback":
        beam(args.cli, receiver_config, "trust", "add", key, "--name", "bench-sender", "--auto-accept")
        receiver = start_receiver(args.cli, receiver_config, inbox, args.port)
        to = f"127.0.0.1:{args.port}"
        receiver_memory = PeakMemory(receiver.pid)
    else:
        to = args.to
        receiver_memory = None
        print("On the receiving machine, trust this sender first:")
        print(f"  beam-cli trust add {key} --name bench-sender --auto-accept\n")

    args.out_dir.mkdir(parents=True, exist_ok=True)
    raw_rows, summary_rows = [], []
    stamp = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    device_info = {
        "sender_device": args.sender_device, "receiver_device": args.receiver_device,
        "wifi_band_ghz": args.wifi_band, "distance_m": args.distance_m, "build": build,
    }
    try:
        for test_set in args.sets:
            files = files_of(args.data_dir, test_set)
            for streams in args.streams:
                runs = []
                for run in range(1, args.runs + 1):
                    if receiver is not None:
                        shutil.rmtree(inbox, ignore_errors=True)
                        receiver_memory.peak = 0
                    stats, sender_rss = send_once(args.cli, sender_config, to, streams, files)
                    receiver_rss = f"{receiver_memory.peak / 1e6:.1f}" if receiver_memory and psutil else ""
                    row = {
                        "timestamp": stamp, "scenario": args.scenario, "link": stats["link"],
                        "test_set": test_set, "streams": streams, "run": run,
                        "files": stats["files"], "bytes": stats["bytes"],
                        "seconds": stats["seconds"], "mb_per_s": stats["mb_per_s"],
                        "peak_mb_per_s": stats["peak_mb_per_s"], "setup_ms": stats["setup_ms"],
                        "reconnects": stats["reconnects"],
                        "sender_peak_rss_mb": sender_rss, "receiver_peak_rss_mb": receiver_rss,
                        **device_info,
                    }
                    runs.append(row)
                    print(f"{test_set:7} streams={streams} run {run}/{args.runs}: "
                          f"{stats['mb_per_s']:8.1f} MB/s  setup {stats['setup_ms']} ms  "
                          f"rss sender {sender_rss or '?'} MB / receiver {receiver_rss or '?'} MB")
                raw_rows += runs
                speeds = [r["mb_per_s"] for r in runs]
                summary_rows.append({
                    "timestamp": stamp, "scenario": args.scenario, "link": runs[0]["link"],
                    "test_set": test_set, "streams": streams, "runs": len(runs),
                    "files": runs[0]["files"], "bytes": runs[0]["bytes"],
                    "median_mb_per_s": round(statistics.median(speeds), 2),
                    "min_mb_per_s": min(speeds), "max_mb_per_s": max(speeds),
                    "median_setup_ms": statistics.median(r["setup_ms"] for r in runs),
                    "max_sender_rss_mb": max((r["sender_peak_rss_mb"] for r in runs), default=""),
                    "max_receiver_rss_mb": max((r["receiver_peak_rss_mb"] for r in runs), default=""),
                    **device_info,
                })
    finally:
        if receiver is not None:
            receiver_memory.stop_mb()
            receiver.kill()
            receiver.wait()

    append_rows(args.out_dir / "raw_results.csv", RAW_FIELDS, raw_rows)
    append_rows(args.out_dir / "results.csv", SUMMARY_FIELDS, summary_rows)
    print(f"\nWrote {len(summary_rows)} summary rows to {args.out_dir / 'results.csv'}")


if __name__ == "__main__":
    main()
