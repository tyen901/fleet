## Fleet

Fleet is an ARMA 3 modpack syncing and game launching tool that works on Windows and Linux. It is completely compatible with Swifty repositories.

Inspired by:

- Swifty: https://getswifty.net/
- Nimble: https://github.com/vitorhnn/nimble

### Building

The core, desktop app and CLI enable the `flux` feature by default. Flux is pinned
to a Git commit. Use `cargo build --workspace --locked` or `npm run build`.

For local Flux development, run `npm run build -- --local-flux`. This explicit
flag patches Flux to `../flux/crates/flux` for that command only and restores the
Git lockfile afterward. Local development is off by default. The same wrapper
accepts Cargo commands, for example `node scripts/cargo.mjs check --local-flux`.
Do not run concurrent Cargo commands in this workspace during a local build.

`cargo build -p fleet --no-default-features --locked` explicitly builds the optional
UI-only app without the Flux backend.

`npm run build:no-flux` runs the optional build directly with Cargo. File
maintenance and launch-time file checks return `backend_unavailable` in this build.

`npm run render:ui` uses the Flux-enabled app and disposable configuration with the
existing opt-in operation simulator. The renderer requires Windows WebView2's
CDP endpoint. Simulation never downloads or verifies real profile files.

### Operation progress

Sync shows separate download and patch bars. Download progress counts remote
payload bytes; local reuse is excluded. Patch progress counts changed file output
as it is rebuilt. Both totals are fixed after inventory. Task metrics report
network throughput and logical disk reads plus writes, with an estimated time
remaining once throughput stabilizes.

Checking for updates uses an inline spinner. Verification starts from profile
settings and returns to the home view. Mismatched files automatically continue
into sync, retaining the same profile lock. Editing stays locked until workers
finish, including cancellation. Sync and validation also block launching;
Launch and Join cancel an active update check before starting the game.
Progress smoothly collapses after
completion or cancellation; failures remain inline on the affected profile.
