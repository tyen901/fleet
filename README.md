## Fleet

Fleet is an ARMA 3 modpack syncing tool that works with Swifty repositories and supports Windows and Linux.
Its main goal is to make syncing and launching modpacks convenient on Linux (including Proton-based setups), while still being usable on Windows.

Inspired by:
- Swifty: https://getswifty.net/
- Nimble: https://github.com/vitorhnn/nimble

## CLI (`fleet-cli`)

Common commands:

```pwsh
# List profiles (shared with the desktop app)
cargo run -p fleet-cli -- profile list

# Add a profile (ID is a unique slug)
cargo run -p fleet-cli -- profile add --id my-server "My Server" https://example.com/repo C:\Mods

# One-time bootstrap if the folder has no local baseline/cache ("Unknown" state in the UI)
# This verifies local files (generates per-mod `.fleet-cache.json`) and persists:
# - `.fleet-local-manifest.json`
# - `.fleet-local-summary.json`
cargo run -p fleet-cli -- repair --profile my-server

# Local integrity check (no network). Compares local files to the persisted baseline.
cargo run -p fleet-cli -- local-check --profile my-server

# Check for updates (fetches remote manifest and compares to local state)
cargo run -p fleet-cli -- check-for-updates --profile my-server

# Sync using a profile
cargo run -p fleet-cli -- sync --profile my-server

# Scan a local mods folder to a manifest JSON
cargo run -p fleet-cli -- scan C:\Mods --output manifest.json

# Launch the game using mods from a profile's local folder (reads local repo.json)
cargo run -p fleet-cli -- launch --profile my-server --exe arma3_x64.exe
```
