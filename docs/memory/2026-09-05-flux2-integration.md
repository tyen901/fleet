# 2026-09-05 — Flux2 materialization integration

Completed:
- Fast-forwarded `codex/flux2-integration` to `develop` at `70545f7` with no conflicts, then replaced the reverted Flux6/`flux-content` boundary with local Flux2 at `C:/projects/flux2/crates/flux`.
- Fleet now adapts Swifty manifests, streaming PBO validation, source ranges, and a target-bound native SQLite observation inventory to Flux2's `MaterializeRequest` pipeline.
- Fast checks report namespace and length evidence; full validation establishes byte equality. Preparing and Publishing snapshots remain active Sync progress, and only terminal completion finalizes an operation.
- Explicit CLI commands suppress the desktop-only startup auto-check while retaining the persisted desktop setting; the regression test starts a Sync with that setting enabled.
- Verified the registered `larx` profile's matching 92-root manifest scope. A one-byte change to `C:\\pca\\@ace\\addons\\ace_advanced_throwing.pbo` was detected by full validation, repaired through the registered source, and restored to the verified backup SHA-256 and length. The following no-op sync fetched, reused, wrote, and deleted zero bytes or entries.

Remaining:
- Root review of the completed branch.

Validation:
- `cargo fmt --all -- --check` — passed
- `cargo build --workspace --locked` — passed
- `cargo test --workspace --locked` — passed
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- Release profile proof — baseline full validation Clean (242.934s, 243,908,608-byte peak); induced corruption validation Dirty (255.18s, 243,933,184-byte peak); repair sync succeeded (38.849s, 274,829,312-byte peak); no-op sync succeeded (37.586s, 275,464,192-byte peak).

Self-review:
- Compatibility aliases added: none
- Fallback paths added: none
- Public internals introduced: none
- Stale docs left behind: none
