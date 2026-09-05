# Architecture

## Apps -> Core boundary

`fleet-core` is the app facade for commands, state subscription, and operation
session control. Apps may also use `fleet-domain` value types and pure validation
that their forms need. The operational crates (`fleet-inventory`,
`fleet-download`, `fleet-flux`, `swifty-repo`, and `fleet-arma3`) stay behind
core, which exposes no internal-layer shims.

## Operation model (authoritative)

Operation execution is operation-centric and keyed by a single shared type:
`fleet_domain::health::OperationKind`
(`Check`, `Validate`, `Sync`).

- `fleet-core` owns operation lifecycle APIs:
  - `start_operation(profile_id, operation_kind) -> session_id`
  - `cancel_session(session_id)`
  - `await_finished(session_id)`
- `OperationRuntime` is the only lifecycle owner. It keeps session records and
  per-profile busy ownership in memory, uses `CancellationToken` for
  cancellation, `watch` for terminal results, and `broadcast` for session events.
- `OperationPublisher` is the only operation stage/progress/notice publisher.
  It emits `OperationSessionEvent` and mirrors progress into
  `AppState.profile_runtime_by_id[profile].active.progress`.
- Runtime state is per-profile in `AppState.profile_runtime_by_id`.
- `OperationSessionEvent.operation` carries the domain `OperationKind` directly.
- `Check` concurrently probes repo freshness and compares requested target
  namespace and file lengths with the installed expected manifest. It never
  establishes byte equality or mutates inventory.
- `Validate` reads every managed file through Flux, verifies its bytes against
  the installed expected manifest, and refreshes reusable observed facts.
- `Sync` is the primary materialization path. Flux owns verification, planning,
  materialization, exact-mirror deletion, and terminal observation evidence;
  core owns the Fleet session's terminal result.
- A failed or cancelled `Sync` invalidates prior local-clean runtime evidence.
- Invalid inventory storage is surfaced as unavailable state. A missing
  observation database is created fresh and rebuilt by materialization.
- The destination is an exact mirror of the requested manifest. `Sync` removes
  destination files outside that manifest.

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
  - Owns repo freshness checks and cache fallback. Metadata downloads and parsing overlap with bounded concurrency, and cache publication is atomic. Fleet crates should not duplicate this logic.
- `fleet-inventory`: durable materialization facts database for the managed target scope.
  - Implements Flux's target-bound `Inventory` contract with observed file version/profile evidence and ordered reusable segments.
  - Uses short provisional observation batch transactions and atomic finish/terminal transactions; provisional rows are incomplete observations, not executable run state.
  - Must not persist run state, staging state, commit state, delete intent, audit history, manifest intent, progress, or recovery journals.
  - SQL-native implementation details remain behind the inventory crate; callers must not use ad hoc SQL access directly.
  - Rust feeds typed facts into SQLite temp tables; SQLite performs set operations, joins, ranking, aggregation, constraints, and mutation.
  - Public API must stay narrow and truth-oriented. Operational planning and runtime bookkeeping belong outside inventory.

### Inventory boundary rules

- Inventory answers: current observed version/profile evidence and reusable local file/segment facts for one target.
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
  ├─ fleet-download
  ├─ swifty-repo
  ├─ flux
  └─ object_store

fleet-inventory
  ├─ flux           (public observation and terminal inventory contracts)
  └─ rusqlite       (private SQL-native implementation)
```
