"""Chart bench/results.csv as PNGs for the README.

Draws one chart per scenario (loopback, lan): median MB/s against the number
of parallel streams, one line per test set. Uses the latest run of each
scenario.

Usage:
    python bench/chart.py
"""

import argparse
import csv
from collections import defaultdict
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

BENCH_DIR = Path(__file__).resolve().parent
SET_LABELS = {
    "big": "1 × 1 GB",
    "medium": "10 × 100 MB",
    "small": "1,000 × 10 KB",
}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, default=BENCH_DIR / "results.csv")
    parser.add_argument("--out-dir", type=Path, default=BENCH_DIR)
    args = parser.parse_args()

    with open(args.results, newline="") as f:
        rows = list(csv.DictReader(f))
    if not rows:
        raise SystemExit(f"{args.results} has no rows; run bench/run.py first")

    by_scenario = defaultdict(list)
    for row in rows:
        by_scenario[row["scenario"]].append(row)

    for scenario, scenario_rows in by_scenario.items():
        latest = max(r["timestamp"] for r in scenario_rows)
        current = [r for r in scenario_rows if r["timestamp"] == latest]
        lines = defaultdict(list)
        for r in current:
            lines[r["test_set"]].append((int(r["streams"]), float(r["median_mb_per_s"])))

        fig, ax = plt.subplots(figsize=(7, 4.2), dpi=150)
        for test_set, points in lines.items():
            points.sort()
            xs, ys = zip(*points)
            ax.plot(xs, ys, marker="o", linewidth=2, label=SET_LABELS.get(test_set, test_set))
            for x, y in points:
                ax.annotate(f"{y:.0f}", (x, y), textcoords="offset points", xytext=(0, 6),
                            ha="center", fontsize=8)
        streams = sorted({x for pts in lines.values() for x, _ in pts})
        ax.set_xscale("log", base=2)
        ax.set_xticks(streams, [str(s) for s in streams])
        ax.set_xlabel("parallel data streams")
        ax.set_ylabel("median throughput (MB/s)")
        devices = current[0]
        detail = ""
        if scenario == "lan":
            detail = (f" — {devices['sender_device']} → {devices['receiver_device']}, "
                      f"{devices['wifi_band_ghz']} GHz, {devices['distance_m']} m")
        ax.set_title(f"Beam {scenario}{detail}\n{devices['build']}, {latest}", fontsize=10)
        ax.set_ylim(bottom=0)
        ax.grid(True, alpha=0.3)
        ax.legend()
        fig.tight_layout()
        out = args.out_dir / f"results_{scenario}.png"
        fig.savefig(out)
        print(f"wrote {out}")


if __name__ == "__main__":
    main()
