# 2026-09-05 — Compact consumer inventory

Completed:
- Replaced the path-expanded observation model with one target/profile binding, integer content IDs, shared immutable recipes, recipe segment rows, and integer observed-file references.
- Observed scans use an unnamed temporary spool; completed observations remain visible until one durable finish transaction publishes the new file recipe.
- Known goal recipes are registered through FleetInventory::register_manifest(&Manifest) and terminal commit protects current goal recipes while pruning stale unreferenced facts.
- Async sync and validation register recipes on a blocking worker using shared immutable manifest storage. Metadata checks remain read-only; terminal upserts avoid duplicate observation reads.
- Pinned real PCA trials pass exact repair outcomes and full-file SHA-256 checks. Median no-op 2.395 / 1.608 seconds and repair 2.819 / 1.745 seconds, baseline / candidate. All-table/index payload falls 98,124,293 / 33,248,503 bytes; allocated DB 118,661,120 / 43,630,592 bytes.
- Full 85,732,470,520-byte verification seeded the compact inventory; it is installed as active observations under session exclusion. A following sync kept all 3,320 files without mutation. Profile/settings hashes are unchanged.

Remaining:
- Per-file durable observations and shared acquisition/publication retain costs. General WAN and cold-disk throughput remain unmeasured.

Validation:
- `cargo test -p fleet-inventory --all-targets` — passed (9 tests).
- `cargo clippy -p fleet-inventory --all-targets -- -D warnings` — passed.
- `cargo test --workspace --all-targets --locked` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- Combined workspace build, all-target tests, strict Clippy, formatting and diff checks passed against Flux `8d381e6`.
- Three alternating no-op and repair trials per build — passed exact counters, pinned revision and restored SHA-256; complete resource/setup evidence in `docs/materialization-performance.md`.

Self-review:
- Compatibility aliases added: none
- Fallback paths added: none
- Public internals introduced: none
- Stale docs left behind: none
