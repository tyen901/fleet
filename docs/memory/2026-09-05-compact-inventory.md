# 2026-09-05 — Compact consumer inventory

Completed:
- Replaced the path-expanded observation model with one target/profile binding, integer content IDs, shared immutable recipes, recipe segment rows, and integer observed-file references.
- Observed scans use an unnamed temporary spool; completed observations remain visible until one durable finish transaction publishes the new file recipe.
- Known goal recipes are registered through FleetInventory::register_manifest(&Manifest) and terminal commit protects current goal recipes while pruning stale unreferenced facts.
- Async sync and validation register recipes on a blocking worker using shared immutable manifest storage. Metadata checks remain read-only; terminal upserts avoid duplicate observation reads.

Remaining:
- Measure the pinned real profile with independently seeded inventories and account for every table/index payload.

Validation:
- `cargo test -p fleet-inventory --all-targets` — passed (9 tests).
- `cargo clippy -p fleet-inventory --all-targets -- -D warnings` — passed.
- `cargo test --workspace --all-targets --locked` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- Combined workspace build, all-target tests, strict Clippy, formatting and diff checks passed against Flux `8d381e6`.

Self-review:
- Compatibility aliases added: none
- Fallback paths added: none
- Public internals introduced: none
- Stale docs left behind: none
