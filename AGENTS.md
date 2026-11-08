# Fleet contributor guide

See [README.md](README.md) for setup and build variants. Use
[ARCHITECTURE.md](ARCHITECTURE.md) when changing shared workflows, sync,
inventory, or error handling at app boundaries.

## Verification

- For Rust changes, run the affected tests and `cargo fmt --check`. For
  cross-crate changes, use `cargo build --workspace --locked` and
  `cargo clippy --workspace --all-targets -- -D warnings`.
- For CSS changes, run `npm run lint:css` and `npm run lint:design`.
- For desktop UI or CSS changes, run `npm run render:ui` on Windows. It builds
  Fleet and launches a separate instance with disposable configuration. Use that
  instance and its debugging endpoint, then inspect every image in
  `target/ui-render/captures/`.

## Desktop UI

Reuse Fleet's components and tokens for spacing and typography. Follow existing
button and field patterns, confirm actions inline, and give icon buttons an
accessible label and tooltip. Keep copy to action names, needed values, and
messages users can act on.

## Git

Use `feature/*` for new branches and short, imperative commit subjects.
In PRs, describe the behavior and verification. Include captures for UI changes.
