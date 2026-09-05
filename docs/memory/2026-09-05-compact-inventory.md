# 2026-09-05 — Compact consumer inventory

Completed:
- Replaced the path-expanded observation model with one target/profile binding, integer content IDs, shared immutable recipes, recipe segment rows, and integer observed-file references.
- Observed scans use an unnamed temporary spool; completed observations remain visible until one durable finish transaction publishes the new file recipe.
- Known goal recipes are registered through FleetInventory::register_manifest(&Manifest) and terminal commit protects current goal recipes while pruning stale unreferenced facts.

Remaining:
- Lead integration of the Fleet branch with the cached-input benchmark harness remains.

Validation:
- `cargo test -p fleet-inventory --all-targets` — passed (9 tests).
- `cargo clippy -p fleet-inventory --all-targets -- -D warnings` — passed.
- `cargo test --workspace --all-targets --locked` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- Final acceptance edits (metadata-only check registration and plain recipe INSERT)
  are formatting/diff-reviewed; the lead's combined focused check remains.

Self-review:
- Compatibility aliases added: none
- Fallback paths added: none
- Public internals introduced: none
- Stale docs left behind: none
