# Architecture

## Apps -> Core boundary

In `apps/` (UI + CLI), the only workspace crates that may be used for domain behaviour are:

- `fleet-core`
- cross-cutting utilities like `fleet-log`

Everything else (`fleet-domain`, `inventory`, `fleet-download`, `fleet-manifest`,
`swifty-repo`, `fleet-arma3`, etc.) is internal and only used via `fleet-core`.

`fleet-core` exposes a single public facade (`Core`) for all app interactions
(state subscription, commands, pipeline session control). Apps should not access
internal layers or shims inside core.

## Operation model (authoritative)

Pipeline execution is operation-centric and keyed by a single shared type:
`fleet_domain::health::OperationKind`
(`Assess(Local|Remote)`, `Sync`).

- `fleet-core` owns operation lifecycle APIs:
  - `start_operation(profile_id, operation_kind) -> session_id`
  - `cancel_session(session_id)`
- Runtime state is per-profile in `AppState.profile_runtime_by_id`.
- `PipelineSessionEvent.operation` carries the domain `OperationKind` directly.
- `Assess(Local)` is a read-only local probe and returns the canonical assessment.
- `Assess(Remote)` remains read-only and extends that assessment with remote freshness.
- `Sync` is the primary self-heal path and may delete truly unexpected residue after manifest-aware inventory stabilization and audit.
- Inventory corruption is surfaced by `Assess` and repaired by `Sync`.

Removed architectural paths that must stay deleted:

- `crates/core/src/features/flow_ops.rs` shim layer
- duplicate-session retry shims driven by error-string matching
- deleted pre-v1 flow helpers and operation shims

## Pipeline boundaries

The operation pipeline is split into purpose-built crates with strict dependency rules:

- `fleet-pipeline`: orchestration + UX contracts (operation execution, events, cancellation).
  - Depends on `fleet-manifest`, `fleet-reconcile`, and `fleet-inventory`.
  - Must not expose generic workflow builders.
- `fleet-manifest`: manifest gathering + transformation.
  - Converts profile sources (Swifty/local) into a desired manifest.
  - Owns manifest freshness mapping and cache fallback.
- `fleet-reconcile`: the only Fleet crate that talks to Flux runtime/services.
  - Owns desired-manifest reconciliation, Flux progress bridging, and prune-only execution.
- `fleet-inventory`: authoritative finalized local managed-folder truth.
  - Owns persisted finalized file facts and segment metadata used for trust and retrieval.
  - Must not persist run state, staging state, commit state, delete intent, audit history, manifest intent, progress, or recovery journals.
  - SQL-backed implementation details remain behind the inventory crate; callers must not use ad hoc SQL access directly.
  - Public API must stay narrow and truth-oriented. Operational planning and runtime bookkeeping belong outside inventory.

### Inventory boundary rules

- Inventory answers: what finalized files are trusted on disk, and where their trusted segments are.
- Inventory does not answer: what a current sync intends to do, what a manifest expects in the future, what should be deleted later, or how an interrupted operation should resume.
- Empty but initialized inventory state may be persisted only when required to distinguish “never established” from “established and currently empty”.
- Pipeline/reconcile/runtime layers must recompute transient decisions from manifest + disk + finalized inventory truth instead of storing those decisions in inventory.
- If a change introduces inventory persistence for anything other than finalized truth, treat it as an architectural violation unless explicitly approved.

### Dependency graph (enforced by convention)

```
fleet-pipeline
  ├─ fleet-domain
  ├─ fleet-download
  ├─ fleet-inventory
  ├─ fleet-manifest
  └─ fleet-reconcile

fleet-manifest
  ├─ fleet-domain
  ├─ fleet-download
  ├─ swifty-repo
  └─ flux-manifest   (data + validation only)

fleet-reconcile   (ONLY Fleet crate with flux runtime deps)
  ├─ fleet-inventory
  ├─ fleet-domain
  ├─ flux-api
  ├─ flux-segment-cache
  ├─ flux-types
  ├─ flux-inventory-contract
  └─ retriever
```
