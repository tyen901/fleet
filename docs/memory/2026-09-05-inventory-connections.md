# 2026-09-05 — Inventory connection reuse

Completed:
- Measured baseline: a 3,320-file no-op opens 19,922 SQLite connections.
- Selected r2d2 with r2d2_sqlite 0.32, compatible with rusqlite 0.38.
- Four bounded pooled connections and cached statements replace per-call opens. Dedicated terminal writer preserves callback reentry; outstanding observations retain session exclusion.
- Real registered-profile comparison passed all 12 runs: median no-op 37.593 to 2.524 seconds, repair 38.246 to 3.045 seconds, with exact byte/counter assertions and unchanged target/configuration.

Remaining:
- None for this architectural step. Large-transfer throughput remains a separate measurement.

Validation:
- Workspace build/tests, formatting, strict clippy — passed on the combined Flux/Fleet tree after real comparison.
- Parallel observation visibility, terminal reentry/rollback and writer lock lifetime — passed.

Self-review:
- Compatibility aliases added: none
- Fallback paths added: none
- Public internals introduced: none
- Stale docs left behind: none
