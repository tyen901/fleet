## Fleet

Fleet is an ARMA 3 modpack syncing and game launching tool that works on Windows and Linux. It is completely compatible with Swifty repositories.

Inspired by:

- Swifty: https://getswifty.net/
- Nimble: https://github.com/vitorhnn/nimble

### Building

The core, desktop app and CLI enable the `flux` feature by default. Flux is pinned
to a Git commit. Use `cargo build --workspace --locked` or `npm run build`.

`cargo build -p fleet --no-default-features --locked` explicitly builds the optional
UI-only app without the Flux backend.

`npm run build:no-flux` runs the optional build directly with Cargo. File
maintenance and launch-time file checks return `backend_unavailable` in this build.

`npm run render:ui` uses the Flux-enabled app with disposable configuration and
real operation failures against an unavailable local repository. The renderer
requires Windows WebView2's CDP endpoint. It never uses real profile files.

Both binaries accept `--config-dir <path>` to select their settings, profiles and
sync state explicitly. Runtime environment overrides and simulated operations are
not supported.

### Operation progress

Sync shows separate download and patch bars. Download progress counts remote
payload bytes against the total Flux computes after resolving local reuse. Patch progress counts changed file output as it is rebuilt. Task metrics
report network throughput and logical disk reads plus writes, with an estimated
time remaining while output is being written.

Checking for updates uses an indeterminate progress bar that clears as soon
as the check finishes. **Validate files** rescans and hashes local files.
Missing content or mismatches automatically continue into sync under the
same profile lock. Editing stays locked until workers finish,
including cancellation. Profile cards hide Launch and Join during checks,
sync, and verification. **Repair** syncs detected local damage, and
**Update** syncs a newer remote revision. When a profile is ready to launch,
Join requires a server. Progress clears after completion or cancellation,
and failures remain inline on the affected profile.
