#!/usr/bin/env python3
"""
Normalize Velopack release assets for GitHub Releases.

Why this exists:
- Velopack's default output naming can be a bit awkward for user-facing assets
  (e.g. `*-win-win-Setup.exe`) and can include duplicated platform strings for
  linux nupkg names when your app id includes `-linux`.

What it does:
- Windows: rename installer/portable assets to stable user-friendly names:
  `fleet-setup-windows-{VERSION}.exe` and `fleet-portable-windows-{VERSION}.zip`.
- Validates that all `releases.*.json` asset filenames exist on disk after renames.

Environment variables:
- VERSION (required): e.g. "0.9.1"
- TARGET (optional): "windows" or "linux" (if omitted, script infers from files)
"""
import json
import os
import sys
from pathlib import Path


def _require_env(name: str) -> str:
    val = os.environ.get(name)
    if not val:
        raise ValueError(f"Environment variable {name} must be set")
    return val


def _read_json(path: Path):
    with path.open("r", encoding="utf-8") as f:
        return json.load(f)


def _write_json(path: Path, data) -> None:
    tmp = path.with_suffix(path.suffix + ".tmp")
    with tmp.open("w", encoding="utf-8") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write("\n")
    tmp.replace(path)


def _rename_single(glob_pat: str, dst: Path) -> bool:
    matches = list(Path("Releases").glob(glob_pat))
    if not matches:
        return False
    if len(matches) > 1:
        raise RuntimeError(f"Expected at most one match for {glob_pat}, found {len(matches)}")
    matches[0].replace(dst)
    return True


def _infer_target() -> str:
    releases = Path("Releases")
    if any(releases.glob("*-Setup.exe")) or any(releases.glob("*-Portable.zip")):
        return "windows"
    if any(releases.glob("*.AppImage")) or any(releases.glob("*-linux-*.nupkg")):
        return "linux"
    return "unknown"


def _validate_release_feeds() -> None:
    releases_dir = Path("Releases")
    for feed in releases_dir.glob("releases.*.json"):
        data = _read_json(feed)
        assets = data.get("Assets") if isinstance(data, dict) else None
        if not isinstance(assets, list):
            continue
        for asset in assets:
            if not isinstance(asset, dict):
                continue
            fn = asset.get("FileName")
            if not isinstance(fn, str) or not fn:
                continue
            if not (releases_dir / fn).exists():
                raise RuntimeError(f"{feed} references missing asset: {fn}")


def main() -> int:
    try:
        version = _require_env("VERSION")
    except ValueError as e:
        print(str(e), file=sys.stderr)
        return 2

    target = os.environ.get("TARGET") or _infer_target()

    if target in ("windows", "unknown"):
        _rename_single("*-Setup.exe", Path(f"Releases/fleet-setup-windows-{version}.exe"))
        _rename_single("*-Portable.zip", Path(f"Releases/fleet-portable-windows-{version}.zip"))

    _validate_release_feeds()
    return 0


if __name__ == "__main__":
    sys.exit(main())
