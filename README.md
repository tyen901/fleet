## Fleet

Fleet is an ARMA 3 modpack syncing tool that works with Swifty repositories and supports Windows and Linux.
Its main goal is to make syncing and launching modpacks convenient on Linux (including Proton-based setups), while still being usable on Windows.

Inspired by:
- Swifty: https://getswifty.net/
- Nimble: https://github.com/vitorhnn/nimble

## Auto-updates (Velopack)

- Release builds: push a tag like `v1.2.3` (see `.github/workflows/release.yml`).
- Updates are hosted on GitHub Releases at `https://github.com/tyen901/fleet/releases/latest/download`.
- Override the update feed base URL via `FLEET_UPDATE_URL`.

## CLI (`fleet-cli`)

Common commands:

```pwsh
# List profiles (shared with the desktop app)
cargo run -p fleet-cli -- profile list

# Add a profile (ID is a unique slug)
cargo run -p fleet-cli -- profile add --id my-server "My Server" https://example.com/repo C:\Mods

# Check (dry-run) using a profile
cargo run -p fleet-cli -- check --profile my-server

# Sync using a profile
cargo run -p fleet-cli -- sync --profile my-server

# Scan a local mods folder to a manifest JSON
cargo run -p fleet-cli -- scan C:\Mods --output manifest.json

# Launch the game using mods from a profile's local folder (reads local repo.json)
cargo run -p fleet-cli -- launch --profile my-server --exe arma3_x64.exe
```
