#!/usr/bin/env python3
"""
Normalize Velopack release assets for GitHub Releases.

Why this exists:
- Velopack's default output naming can be awkward for user-facing assets
  (e.g. `*-win-win-Setup.exe`) and can produce inconsistent naming across
  release channels.

What it does:
- Applies a consistent release stamp prefix: `fleet-{VERSION}-{CHANNEL}`.
- Windows:
  - `...-Setup.exe` -> `fleet-{VERSION}-{CHANNEL}-setup.exe`
  - `...-Portable.zip` -> `fleet-{VERSION}-{CHANNEL}-portable.zip`
- Linux:
  - `*.AppImage` -> `fleet-{VERSION}-{CHANNEL}.AppImage`
- Rewrites any JSON filename references (`releases.*.json`, `assets.*.json`,
  and other JSON in `Releases/`) so renamed files stay addressable.
- Validates that all `releases.*.json` `FileName` entries exist on disk.

Environment variables:
- VERSION (required): e.g. "0.9.1"
- CHANNEL (optional): e.g. "win-stable", "linux-stable" (inferred when omitted)
- TARGET (optional): "windows" or "linux" (if omitted, script infers from files)
"""
import json
import os
import sys
from pathlib import Path
from typing import Any, Optional, Tuple


def _require_env(name: str) -> str:
    val = os.environ.get(name)
    if not val:
        raise ValueError(f"Environment variable {name} must be set")
    return val


def _read_json(path: Path):
    with path.open("r", encoding="utf-8") as f:
        return json.load(f)


def _write_json(path: Path, data: Any) -> None:
    with path.open("w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
        f.write("\n")


def _infer_channel() -> str:
    releases = Path("Releases")

    for pat, prefix, suffix in (
        ("releases.*.json", "releases.", ".json"),
        ("assets.*.json", "assets.", ".json"),
    ):
        for f in releases.glob(pat):
            if f.name.startswith(prefix) and f.name.endswith(suffix):
                return f.name[len(prefix) : -len(suffix)]

    for f in releases.glob("RELEASES-*"):
        _, _, channel = f.name.partition("RELEASES-")
        if channel:
            return channel

    return "unknown"


def _rename_single(dst: Path, *glob_pats: str) -> Optional[Tuple[str, str]]:
    releases = Path("Releases")
    names = set()
    matches: list[Path] = []
    for glob_pat in glob_pats:
        for m in releases.glob(glob_pat):
            if m.name in names:
                continue
            names.add(m.name)
            matches.append(m)

    if not matches:
        return None
    if len(matches) > 1:
        joined = ", ".join(sorted(m.name for m in matches))
        raise RuntimeError(f"Expected at most one match, found {len(matches)}: {joined}")

    src = matches[0]
    if src.name == dst.name:
        return None

    src.replace(dst)
    return (src.name, dst.name)


def _infer_target() -> str:
    releases = Path("Releases")
    if (
        any(releases.glob("*-Setup.exe"))
        or any(releases.glob("*-Portable.zip"))
        or any(releases.glob("*-setup.exe"))
        or any(releases.glob("*-portable.zip"))
    ):
        return "windows"
    if any(releases.glob("*.AppImage")) or any(releases.glob("*-linux-*.nupkg")):
        return "linux"
    return "unknown"


def _rewrite_json_filename_refs(rename_map: dict[str, str]) -> None:
    if not rename_map:
        return

    releases_dir = Path("Releases")
    json_files = sorted(releases_dir.glob("*.json"))

    def _rewrite(node: Any) -> tuple[Any, bool]:
        if isinstance(node, dict):
            changed = False
            new_dict = {}
            for k, v in node.items():
                new_v, child_changed = _rewrite(v)
                new_dict[k] = new_v
                changed = changed or child_changed
            return new_dict, changed
        if isinstance(node, list):
            changed = False
            new_list = []
            for item in node:
                new_item, child_changed = _rewrite(item)
                new_list.append(new_item)
                changed = changed or child_changed
            return new_list, changed
        if isinstance(node, str) and node in rename_map:
            return rename_map[node], True
        return node, False

    for jf in json_files:
        data = _read_json(jf)
        rewritten, changed = _rewrite(data)
        if changed:
            _write_json(jf, rewritten)


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
    channel = os.environ.get("CHANNEL") or _infer_channel()
    stamp = f"fleet-{version}-{channel}" if channel != "unknown" else f"fleet-{version}"

    rename_map: dict[str, str] = {}

    if target in ("windows", "unknown"):
        setup_renamed = _rename_single(
            Path(f"Releases/{stamp}-setup.exe"),
            "*-Setup.exe",
            "fleet-setup-windows-*.exe",
        )
        portable_renamed = _rename_single(
            Path(f"Releases/{stamp}-portable.zip"),
            "*-Portable.zip",
            "fleet-portable-windows-*.zip",
        )
        if setup_renamed:
            old, new = setup_renamed
            rename_map[old] = new
        if portable_renamed:
            old, new = portable_renamed
            rename_map[old] = new

    if target in ("linux", "unknown"):
        appimage_renamed = _rename_single(
            Path(f"Releases/{stamp}.AppImage"),
            "*.AppImage",
        )
        if appimage_renamed:
            old, new = appimage_renamed
            rename_map[old] = new

    _rewrite_json_filename_refs(rename_map)

    _validate_release_feeds()
    return 0


if __name__ == "__main__":
    sys.exit(main())
