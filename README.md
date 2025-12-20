## Fleet

Fleet is an ARMA 3 modpack syncing tool that works with Swifty repositories and supports Windows and Linux.
Its main goal is to make syncing and launching modpacks convenient on Linux (including Proton-based setups), while still being usable on Windows.

## Build and Run

- Build everything: `cargo build --workspace`
- Run Fleet (no args launches the GUI): `cargo run -p fleet`
- CLI help: `cargo run -p fleet -- --help`

## CLI Examples

### One-shot sync (no profiles)

```bash
fleet sync \
  --repo-url https://cdn.deltasync.io/data/modpack_test/ \
  --path /path/to/arma3/mods
```

### Print JSON event stream

```bash
fleet sync \
  --repo-url https://cdn.deltasync.io/data/modpack_test/ \
  --path /path/to/arma3/mods \
  --json-events
```

### Profile workflow

```bash
# Initialize registry (optional; auto-created on first use)
fleet profile init

# Add and select a profile
fleet profile add --name "Modpack Test" \
  --repo-url https://cdn.deltasync.io/data/modpack_test/ \
  --path /path/to/arma3/mods

# List profiles
fleet profile list

# Sync selected profile
fleet sync

# Launch Arma 3 using a profile (requires Steam setup)
fleet launch --profile <profile-id>
```

## Development Commands

- Tests: `cargo test --workspace`
- Lint: `cargo clippy --workspace --all-targets`
- Format: `cargo fmt`

### Live CLI sync tests (opt-in)

The live integration tests are network-dependent and gated behind an environment flag:

```bash
FLEET_LIVE_TESTS=1 cargo test -p fleet --test cli_live -- --nocapture
```

Inspired by:
- Swifty: https://getswifty.net/
- Nimble: https://github.com/vitorhnn/nimble
