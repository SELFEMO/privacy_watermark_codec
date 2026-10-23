"""Measure pinned media once each without logging plaintext or key contents."""

import argparse
import json
from pathlib import Path
import struct
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("key", type=Path)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/examples/inspect_image"))
    parser.add_argument("--seconds", type=int, choices=(10, 60), default=10)
    parser.add_argument("--unmarked", type=Path)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    paths = sorted(args.directory.glob("*.png"))
    if args.unmarked:
        paths.append(args.unmarked)
    rows = []
    for path in paths:
        dimensions = None
        with path.open("rb") as source:
            header = source.read(24)
            if header[:8] == b"\x89PNG\r\n\x1a\n":
                dimensions = list(struct.unpack(">II", header[16:24]))
        started = time.monotonic()
        # 每个固定输入仅解码一次，不重编码或换密钥重试，避免把随机成功当成算法保证。
        # Decode each pinned input once without re-encoding or changing keys, avoiding accidental success being treated as an algorithmic guarantee.
        try:
            result = subprocess.run(
                [str(args.binary.resolve()), str(path), str(args.key), str(args.seconds)],
                capture_output=True, text=True, timeout=args.seconds + 5,
            )
            row = dict(file=path.name, size=dimensions, ok=result.returncode == 0,
                       seconds=round(time.monotonic() - started, 3), result=result.stdout.strip())
        except subprocess.TimeoutExpired:
            row = dict(file=path.name, size=dimensions, ok=False,
                       seconds=round(time.monotonic() - started, 3), result="external timeout")
        rows.append(row)
        print(json.dumps(row, ensure_ascii=False), flush=True)
    if args.json:
        args.json.write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
