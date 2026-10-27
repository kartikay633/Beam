"""Generate the benchmark test sets (SPEC.md section 12).

    big:    1 x 1 GB
    medium: 10 x 100 MB
    small:  1,000 x 10 KB

Files are random bytes, so compression can't flatter the numbers. Existing
files with the right size are kept, so re-running is cheap.

Usage:
    python bench/generate.py --data-dir D:/beam-bench-data
"""

import argparse
import os
from pathlib import Path

SETS = {
    "big": [("one_gigabyte.bin", 1_000_000_000)],
    "medium": [(f"file_{i:02}.bin", 100_000_000) for i in range(10)],
    "small": [(f"file_{i:04}.bin", 10_000) for i in range(1000)],
}

BLOCK = 4 * 1024 * 1024


def write_random(path: Path, size: int) -> None:
    if path.exists() and path.stat().st_size == size:
        return
    with open(path, "wb") as f:
        remaining = size
        while remaining > 0:
            n = min(BLOCK, remaining)
            f.write(os.urandom(n))
            remaining -= n


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--data-dir", default=str(Path(__file__).parent / "data"),
                        help="where to put the test sets (default: bench/data)")
    parser.add_argument("--sets", nargs="+", choices=SETS, default=list(SETS))
    args = parser.parse_args()

    root = Path(args.data_dir)
    for name in args.sets:
        folder = root / name
        folder.mkdir(parents=True, exist_ok=True)
        files = SETS[name]
        for file_name, size in files:
            write_random(folder / file_name, size)
        total = sum(size for _, size in files)
        print(f"{name:7} {len(files):5} file(s), {total / 1e6:8.1f} MB in {folder}")


if __name__ == "__main__":
    main()
