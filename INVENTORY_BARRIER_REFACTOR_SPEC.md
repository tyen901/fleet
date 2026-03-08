# Inventory Barrier Refactor Spec

## Title

Split `inventory` into clearer bounded components by extracting scanner orchestration responsibilities and removing the Flux-specific SQL bridge from the crate's public identity.

## Purpose

This document is an implementation spec for a technical AI agent. It defines the exact refactor shape, module boundaries, sequencing, acceptance criteria, and constraints for breaking up the current `inventory` crate around two problem areas:

1. `crates/inventory/src/scanner/*`
2. `crates/inventory/src/flux_sqlite.rs`

The goal is not to redesign Fleet broadly. The goal is to reduce responsibility overlap inside `inventory`, improve internal barriers, and make future extraction into dedicated crates straightforward.

## Current Problems

### Scanner area

`crates/inventory/src/scanner/scanner.rs` currently owns too many responsibilities at once:

- disk walk orchestration
- delta planning against persisted state
- stamp accumulation
- worker-pool lifecycle
- Swifty artifact scanning
- progress and cancellation policy
- DB transaction/session lifecycle
- seen-set maintenance
- prune application
- stamp persistence

This means the scanner is not a reusable scan engine. It is a full "baseline refresh pipeline" tightly coupled to `InventoryDb` and `UpdateSession`.

### Flux SQL bridge area

`crates/inventory/src/flux_sqlite.rs` currently mixes:

- public Flux-facing DTOs and trait definitions
- SQL-backed trusted-file and segment lookup logic
- root binding and schema init
- reconcile-specific prune protection logic

This is not inventory-domain behavior. It is reconcile integration infrastructure. Keeping it public from the `inventory` crate keeps the crate responsible for both local-state persistence and reconcile contract adaptation.

## Non-Goals

- Do not replace the current inventory storage engine.
- Do not change on-disk schema or file names.
- Do not introduce a generic plugin system.
- Do not add compatibility shims that preserve the old public `flux_sqlite` API surface under `inventory`.
- Do not broaden Fleet-facing abstractions beyond actual current needs.

## Target End State

`inventory` remains the crate for local baseline storage and scan-related primitives, but it no longer presents itself as:

- a full scan orchestration pipeline
- a Flux-facing public contract surface

The final shape must satisfy:

- `inventory` owns baseline storage, scan policy, scan planning/execution pieces, drift inspection helpers, and DB/session primitives.
- `reconcile` owns the Flux contract adapter.
- any shared storage-facing interface used by `reconcile` is inventory-neutral, not Flux-named.
- `inventory/src/lib.rs` no longer re-exports Flux-specific traits or record types.

## Required High-Level Changes

### 1. Break scanner into explicit internal phases

Refactor `crates/inventory/src/scanner/scanner.rs` into multiple focused internal modules under `crates/inventory/src/scanner/`.

#### Required module split

Create these modules:

- `scanner/plan.rs`
- `scanner/exec.rs`
- `scanner/apply.rs`
- keep `scanner/walk.rs`
- keep `scanner/swifty_map.rs`
- keep `scanner/config.rs`

`scanner/scanner.rs` may remain as a thin orchestrator facade or be replaced with `scanner/mod.rs` glue, but it must no longer contain the full pipeline logic inline.

#### Required responsibilities

`scanner/plan.rs`

- input:
  - `root_path`
  - `ScanPolicy`
  - persisted index snapshot
  - persisted stamp/metrics snapshot
  - delta mode flags
- output:
  - a concrete `ScanPlan`
- owns:
  - walking the filesystem
  - identifying `seen_paths`
  - determining `scan_items`
  - identifying "no changes" and "stamp refresh only" cases
  - accumulating the current quick stamp
- must not open update sessions or mutate the DB

`scanner/exec.rs`

- input:
  - `ScanPlan`
  - worker/runtime settings
  - progress/cancel hooks
- output:
  - deterministic scan results for files/segments
- owns:
  - worker-pool setup
  - concurrency
  - Swifty artifact scanning
  - progress emission for `Scanning`
  - cancellation checks during work dispatch and result collection
- must not talk to `UpdateSession`

`scanner/apply.rs`

- input:
  - `root_id`
  - `ScanPlan`
  - execution results
  - current stamp
- output:
  - `SyncResult`
- owns:
  - `UpdateSession`
  - `begin_seen_set`
  - `mark_seen`
  - `upsert_file`
  - `replace_segments`
  - `prune_unseen`
  - `set_stamp`
  - `commit` / rollback
- must not perform walking or artifact scanning

#### Required new types

Define concrete internal types for the split:

- `ScanPlan`
- `PlannedWalkItem`
- `ScanExecResult` or equivalent
- `AppliedScanStats` or equivalent

These types must live under `scanner/` and must not be re-exported from the crate root unless needed by tests.

### 2. Split scanner configuration by concern

Refactor `crates/inventory/src/scanner/config.rs` so configuration is no longer one large mixed struct.

#### Required shape

Keep `ScannerConfig` as the call surface only if needed during transition, but internally split it into:

- `ScanRuntimeConfig`
  - `workers`
  - `queue_capacity`
  - `progress_interval`
- `ScanBehaviorConfig`
  - `delta`
  - `delta_index_cache`
  - `policy`
- `ScanObserver`
  - `progress`
  - `cancel`

The orchestrator may assemble these, but planning and execution code should consume the narrowest possible subset.

#### Required rule

No module outside scanner execution should need access to worker-pool tuning and callback hooks unless it is directly involved in runtime orchestration.

### 3. Remove Flux-specific public API from `inventory`

Delete the current public Flux API surface from `inventory/src/lib.rs`:

- `open_flux_inventory`
- `FluxInventoryApi`
- `FinalizedFileRecord`
- `SegmentLoc`
- `SegmentSignature`
- `TrustedFileMeta`
- `TrustedFileRecord`

These names must stop being re-exported from `inventory`.

### 4. Replace `flux_sqlite.rs` with inventory-neutral repository interfaces

The SQL logic currently in `crates/inventory/src/flux_sqlite.rs` must be reworked into inventory-neutral storage access, then used by `reconcile`.

#### Required split

Inside `inventory`, create a module such as:

- `trusted_index.rs`

It must expose inventory-neutral interfaces and types only.

#### Required inventory-neutral interface

Define a trait or concrete repository API with responsibilities equivalent to:

- read trusted file metadata by path
- read segment locations by signature
- test presence of a specific segment location
- batch-read trusted files
- batch-read segment locations
- record finalized file batches
- report protected local-state paths

#### Naming rule

Do not use `Flux` in type names in the `inventory` crate for this interface.

Examples of acceptable naming:

- `TrustedIndexReader`
- `TrustedIndexWriter`
- `TrustedInventoryIndex`
- `SqliteTrustedIndex`

Examples of forbidden naming:

- `FluxInventoryApi`
- `FluxInventorySqlite`
- `open_flux_inventory`

### 5. Move Flux contract adaptation into `reconcile`

`crates/reconcile/src/flux_sqlite.rs` must stop depending on `inventory`'s Flux-named public API. Instead, it should depend on the new inventory-neutral repository interface.

#### Required end state

`reconcile` owns:

- Flux contract DTO conversion
- implementation of `flux_inventory_contract::FluxInventory`
- mapping between Flux signatures/records and inventory-neutral repository types

`inventory` owns:

- SQL-backed trusted index repository
- local protected-path reporting based on DB/root context

### 6. Keep `api.rs` as the only high-level local-state facade inside `inventory`

`crates/inventory/src/api.rs` should remain the main high-level entry point for callers needing local baseline operations.

#### Required behavior

`RootInventory::scan` should become a thin composition method that:

1. creates a scan plan
2. executes it
3. applies it
4. returns `SyncResult`

It must not retain the full scan lifecycle inline.

## Target File/Module Shape

### `crates/inventory/src/`

Expected target shape after refactor:

- `api.rs`
- `db.rs`
- `error.rs`
- `hash.rs`
- `model.rs`
- `policy.rs`
- `stamp.rs`
- `trusted_index.rs`
- `sqlite.rs`
- `sqlite_conn.rs`
- `scanner/config.rs`
- `scanner/plan.rs`
- `scanner/exec.rs`
- `scanner/apply.rs`
- `scanner/mod.rs`
- `scanner/swifty_map.rs`
- `scanner/walk.rs`

`flux_sqlite.rs` must be deleted from `inventory`.

### `crates/reconcile/src/`

Expected retained shape:

- `flux_sqlite.rs`
- `runner.rs`
- `progress.rs`
- `convert.rs`
- `retrieval.rs`

But `reconcile/src/flux_sqlite.rs` must now be an adapter over `inventory::trusted_index::*`, not over Flux-named inventory exports.

## Exact API Constraints

### Scanner public surface

Allowed root-level scanner exports after refactor:

- `ScanError`
- `ScanProgress`
- `ScanStage`
- `ScannerConfig`
- `SyncMode`
- `SyncRequest`
- `SyncResult`
- `Scanner` only if still needed as the public orchestrator facade

Forbidden root-level scanner exports:

- planning internals
- worker-pool internals
- apply/session internals

### Inventory root exports

Allowed:

- `Inventory`
- `RootInventory`
- `InventoryState`
- drift models
- snapshot/metrics models
- scan config/progress/result
- policy types
- storage types only if they are still needed internally by existing in-repo consumers

Forbidden:

- Flux-specific API boundary types
- Flux-specific constructor functions

### Reconcile dependency rule

`reconcile` may depend on:

- inventory-neutral trusted-index interface from `inventory`

`reconcile` must not depend on:

- Flux-specific APIs defined inside `inventory`

## Implementation Sequence

Follow this order exactly.

### Phase 1: Internal scanner split with no behavior change

- create `scanner/plan.rs`
- create `scanner/exec.rs`
- create `scanner/apply.rs`
- move logic out of `scanner/scanner.rs` into those modules
- keep `Scanner::sync_root` public behavior unchanged
- keep tests passing with no caller changes

Acceptance:

- inventory scanner tests still pass unchanged
- no public API behavior changes yet

### Phase 2: Introduce inventory-neutral trusted-index repository

- create `trusted_index.rs`
- move SQL logic from `inventory/src/flux_sqlite.rs` into `trusted_index.rs`
- define inventory-neutral types and traits
- keep SQL behavior unchanged

Acceptance:

- new repository layer has unit coverage for batch lookup, finalized file writes, and protected prune paths

### Phase 3: Update reconcile adapter to the new repository

- refactor `crates/reconcile/src/flux_sqlite.rs` to depend on `inventory::trusted_index::*`
- remove any dependency on `open_flux_inventory` and `FluxInventoryApi`

Acceptance:

- reconcile tests still pass
- Flux sync still compiles and passes existing tests

### Phase 4: Delete `inventory/src/flux_sqlite.rs` and root exports

- delete `inventory/src/flux_sqlite.rs`
- remove its re-exports from `inventory/src/lib.rs`
- update inventory tests that directly import old Flux API names
- rewrite tests instead of preserving aliases

Acceptance:

- no `Flux*` symbols are exported from `inventory`
- no in-repo caller imports deleted symbols

### Phase 5: Clean up scanner configuration barriers

- split config concerns internally
- update `local-state-inventory` and any remaining callers to use the narrowed config flow
- remove dead code and unused exports

Acceptance:

- no dead scanner config fields
- no unused imports or warnings under clippy

## Tests To Add or Rewrite

### Scanner tests

Add or rewrite tests to cover:

- planning detects no-op clean case without scanning
- planning detects stamp-refresh-only case
- execution honors cancellation before and during scan
- apply layer rolls back on execution failure
- orchestrator still returns identical `SyncResult` behavior for clean, delta, and cancel paths

### Trusted index tests

Move or rewrite the old `flux_inventory_api_boundary` coverage to target the new inventory-neutral repository.

Cover:

- finalized file record round-trip
- single and batch trusted-file lookup
- single and batch segment-location lookup
- protected prune path reporting

### Reconcile adapter tests

Keep adapter tests in `reconcile`, not `inventory`.

Cover:

- signature conversion correctness
- batch result mapping correctness
- finalized-file recording path

## Search-and-Destroy Checklist

Before completion, the implementing agent must confirm all of the following:

- no `open_flux_inventory` remains in the repo
- no `FluxInventoryApi` remains in the repo
- no `inventory::FinalizedFileRecord` import remains outside `reconcile`
- no `inventory::SegmentSignature` import remains outside `reconcile`
- no `inventory/src/flux_sqlite.rs` file remains
- `scanner/scanner.rs` is no longer the place where planning, execution, and apply logic all coexist
- `inventory/src/lib.rs` no longer re-exports Flux-facing contract types

## Acceptance Criteria

The work is complete only if all are true:

- `cargo fmt` passes
- `cargo build` passes
- `cargo clippy --workspace --all-targets -- -D warnings` passes
- `cargo test` passes
- no compatibility shims were added to preserve deleted Flux API names in `inventory`
- root `inventory` public API is narrower than before
- scanner orchestration responsibilities are explicitly separated into planning, execution, and apply barriers

## Risks and Required Guardrails

### Risk: accidental semantic drift in scan behavior

Guardrail:

- preserve current scanner tests first
- do Phase 1 as a structural refactor before public API cleanup

### Risk: over-abstracting trusted index APIs

Guardrail:

- keep the new repository interface as narrow as the current reconcile use cases
- do not create a generic storage framework

### Risk: leaking reconcile concepts back into inventory naming

Guardrail:

- forbid `Flux` names in the new inventory-neutral repository module

## Deliverables

The implementing agent must produce:

1. the code refactor
2. updated tests
3. deleted obsolete files and exports
4. a final summary listing:
   - modules created
   - modules deleted
   - public exports removed
   - validation commands run

