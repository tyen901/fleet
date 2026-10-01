## Fleet

Fleet is an ARMA 3 modpack syncing and game launching tool that works on Windows and Linux. It is completely compatible with Swifty repositories.

Inspired by:

- Swifty: https://getswifty.net/
- Nimble: https://github.com/vitorhnn/nimble

### Building

The desktop app and CLI enable the `flux` feature by default. With access to the
pinned Flux repository, use `cargo build --workspace --locked` (or explicitly
`cargo build -p fleet --features flux`).

For UI development without Flux access:

```sh
npm run build:no-flux
node scripts/cargo-no-flux.mjs test -p fleet-core state::tests --locked
node scripts/cargo-no-flux.mjs clippy --workspace --all-targets -- -D warnings
```

The binary is in `target/no-flux/debug/`. Cargo resolves even disabled optional
Git dependencies, so the helper generates pruned manifests under `target/`,
links the same sources/assets, and checks that resolution preserves the versions
in `Cargo.lock`. It does not download Flux or substitute a fake backend. Real
file maintenance and launch-time file checks return `backend_unavailable` in
this build.

`npm run render:ui` uses the no-Flux app and disposable configuration with the
existing opt-in operation simulator. The renderer requires Windows WebView2's
CDP endpoint. Simulation never downloads or verifies real profile files.
