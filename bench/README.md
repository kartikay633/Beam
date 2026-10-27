# Beam benchmarks

Scripts that measure Beam end to end with the real `beam-cli` (SPEC.md
section 12). The latest results are in [results.csv](results.csv), with every
individual run in [raw_results.csv](raw_results.csv) and a chart for each
scenario (for example [results_loopback.png](results_loopback.png)).

## Setup

```bash
python -m venv .venv            # or anywhere, e.g. D:/beam-venv
.venv/Scripts/pip install -r bench/requirements.txt   # Linux/macOS: .venv/bin/pip
cargo build --release -p beam-cli
```

`psutil` is optional. Without it, the peak-memory columns are left empty.

## 1. Generate the test sets

```bash
python bench/generate.py --data-dir D:/beam-bench-data
```

| Set | Contents |
|---|---|
| `big` | 1 × 1 GB |
| `medium` | 10 × 100 MB |
| `small` | 1,000 × 10 KB |

The files are random, so compression can't flatter the numbers. Existing
files are reused.

## 2. Run

**Loopback** (sender and receiver on this machine). This measures the
software: crypto, hashing and disk, not the network.

```bash
python bench/run.py --data-dir D:/beam-bench-data --work-dir D:/beam-bench-work
```

**LAN** (receiver on another machine):

1. On the receiving machine, start a receiver:
   `beam-cli receive --port 9000 --dir ./inbox`
2. On this machine, run the command below. It first prints a
   `beam-cli trust add …` line; run that on the receiver so it accepts the
   benchmark sender without asking.

```bash
python bench/run.py --scenario lan --to 192.168.1.20:9000 \
    --sender-device "ThinkPad X1 Carbon" --receiver-device "Desktop, Intel AX210" \
    --wifi-band 5 --distance-m 3 --data-dir D:/beam-bench-data
```

Each test set × stream count (1, 2, 4, 8) is sent 5 times by default.

- `raw_results.csv` gets one row per run: MB/s, peak MB/s, setup time,
  reconnects, and peak memory of the sender and (loopback) receiver.
- `results.csv` gets one row per scenario with the median, min and max.

Always fill in the device models, Wi-Fi band and distance for LAN runs.
Honest numbers only.

## 3. Chart

```bash
python bench/chart.py
```

This writes `results_<scenario>.png` from the latest run of each scenario in
`results.csv`.

## Reading the numbers

- **MB = 1,000,000 bytes.** Throughput is measured by the sender from the
  receiver's ACCEPT to TRANSFER_COMPLETE, so it includes the end-to-end hash
  checks but not the user's decision time.
- **Loopback is a ceiling, not a Wi-Fi number.** On 5 GHz Wi-Fi, expect the
  link to be the limit; the target is ≥ 25 MB/s (SPEC.md section 13).
- **Small files** are bound by per-file costs: creating, closing and renaming
  each file. On Windows with real-time antivirus scanning, that's about
  1.3 ms per file on the test machine, so the stream count makes no
  difference there. See docs/LEARNINGS.md.
