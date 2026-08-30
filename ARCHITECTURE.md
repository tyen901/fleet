# Architecture

## Apps -> Core boundary

In `apps/` (UI + CLI), the only workspace crates that may be used for domain behaviour are:

- `fleet-core`
- cross-cutting utilities like `fleet-log`

Everything else (`fleet-domain`, `fleet-inventory`, `fleet-download`,
`fleet-flux`, `swifty-repo`, `fleet-arma3`, etc.) is internal and only used via
`fleet-core`.

`fleet-core` exposes a single public facade (`Core`) for all app interactions
(state subscription, commands, operation session control). Apps should not access
internal layers or shims inside core.

## Operation model (authoritative)

Operation execution is operation-centric and keyed by a single shared type:
`fleet_domain::health::OperationKind`
(`Check`, `Validate`, `Sync`).

- `fleet-core` owns operation lifecycle APIs:
  - `start_operation(profile_id, operation_kind) -> session_id`
  - `cancel_session(session_id)`
- `OperationRuntime` is the only lifecycle owner. It uses Tokio/tokio-util
  primitives for generic mechanics: `CancellationToken` for cancellation,
  `TaskTracker` for spawned operation lifecycle, `watch` for terminal results,
  and `broadcast` for session events.
- `OperationPublisher` is the only operation stage/progress/notice publisher.
  It emits `OperationSessionEvent` and mirrors progress into
  `AppState.profile_runtime_by_id[profile].active.progress`.
- Runtime state is per-profile in `AppState.profile_runtime_by_id`.
- `OperationSessionEvent.operation` carries the domain `OperationKind` directly.
- `Check` concurrently probes repo freshness and compares target file metadata
  with the installed expected manifest and durable file facts. It never reads
  file content or mutates inventory.
- `Validate` reads every managed file through Flux, verifies its bytes against
  the installed expected manifest, and refreshes reusable observed facts.
- `Sync` is the primary self-heal path. Flux holds one target session across
  verification, planning, materialization, deletion, and terminal ownership updates.
- A failed or cancelled `Sync` invalidates prior local-clean runtime evidence.
- Inventory corruption is surfaced by `Check`/`Validate` and repaired by `Sync`.
- Arbitrary unmanaged files are outside Fleet's maintenance scope. `Sync` removes
  only paths previously managed by Fleet and removed from the new manifest.

Removed architectural paths that must stay deleted:

- `crates/core/src/features/flow_ops.rs` shim layer
- `crates/pipeline/`
- duplicate-session retry shims driven by error-string matching
- deleted pre-v1 flow helpers and operation shims

## Operation boundaries

The operation system is split into purpose-built crates with strict dependency rules:

- `fleet-core`: authoritative session lifecycle + UX contracts (operation execution, events, cancellation).
  - Depends on `fleet-flux`, `fleet-inventory`, `fleet-download`, `swifty-repo`, and domain/support crates.
  - Must not expose generic workflow builders.
- `fleet-flux`: the only Fleet crate that converts Fleet/Swifty shapes into Flux materialization input or talks to Flux runtime/services.
  - Owns Swifty-to-Flux input conversion, Swifty content profile/store source wiring, and calls `flux::materialize` directly.
  - Passes Flux's typed progress snapshots into Fleet's operation event model.
- `swifty-repo`: Swifty repo cache, repo metadata sync/probe, and cached repo access.
  - Owns repo freshness checks and cache fallback. Fleet crates should not duplicate Swifty cache logic.
- `fleet-inventory`: durable materialization facts database for the managed target scope.
  - Persists the current managed target-relative path snapshot, reusable local file facts, and reusable segment metadata required for local reuse.
  - The managed-path snapshot may contain paths that do not have reusable file facts.
  - Must not persist run state, staging state, commit state, delete intent, audit history, manifest intent, progress, or recovery journals.
  - SQL-native implementation details remain behind the inventory crate; callers must not use ad hoc SQL access directly.
  - Rust feeds typed facts into SQLite temp tables; SQLite performs set operations, joins, ranking, aggregation, constraints, and mutation.
  - Public API must stay narrow and truth-oriented. Operational planning and runtime bookkeeping belong outside inventory.

### Inventory boundary rules

- Inventory answers: the current managed target-relative path snapshot and reusable local file/segment facts.
- Inventory does not answer: what a current sync intends to do, what a manifest expects in the future, what should be deleted later, or how an interrupted operation should resume.
- There is no initialized baseline state.
- Core/Flux/runtime layers must recompute transient decisions from manifest + disk + materialization inventory truth instead of storing those decisions in inventory.
- Flux produces typed verification and materialization progress snapshots; Fleet
  UI/CLI only render read-only projections.
- Runtime keeps fast-check evidence, byte-validation evidence, and successful
  materialization evidence distinct; the UI derives directly from those reports.
- If a change introduces inventory persistence for run state, staging state, delete intent, audit history, manifest intent, progress, or recovery journals, treat it as an architectural violation unless explicitly approved.

### Dependency graph (enforced by convention)

```
fleet-core
  ├─ fleet-domain
  ├─ fleet-download
  ├─ fleet-inventory
  ├─ swifty-repo
  └─ fleet-flux

fleet-flux        (ONLY Fleet crate with Flux materialization/runtime ownership)
  ├─ fleet-inventory
  ├─ fleet-domain
  ├─ fleet-download
  ├─ swifty-repo
  ├─ flux
  └─ object_store

fleet-inventory
  ├─ flux           (capability-specific inventory traits)
  └─ rusqlite       (private SQL-native implementation)
```
