#!/usr/bin/env python3
import sys
from pathlib import Path


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print("Usage: make_windows_ico.py <input.png> <output.ico>", file=sys.stderr)
        return 2

    src = Path(argv[1])
    dst = Path(argv[2])

    if not src.is_file():
        print(f"Input file not found: {src}", file=sys.stderr)
        return 2

    try:
        from PIL import Image  # type: ignore
    except Exception as e:
        print("Pillow is required (pip install pillow).", file=sys.stderr)
        print(str(e), file=sys.stderr)
        return 2

    dst.parent.mkdir(parents=True, exist_ok=True)

    img = Image.open(src).convert("RGBA")
    img.save(
        dst,
        format="ICO",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    print(f"Wrote {dst}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))

