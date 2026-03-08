# Architecture

## Apps -> Core boundary

In `apps/` (UI + CLI), the only workspace crates that may be used for domain behaviour are:

- `fleet-core`
- cross-cutting utilities like `fleet-log`

Everything else (`fleet-domain`, `inventory`, `fleet-download`, `fleet-manifest`,
`swifty-repo`, `fleet-arma3`, etc.) is internal and only used via `fleet-core`.

`fleet-core` exposes a single public facade (`Core`) for all app interactions
(state subscription, commands, flow session control). Apps should not access
internal layers or shims inside core.

## Operation model (authoritative)

Flow execution is operation-centric and keyed by a single shared type:
`fleet_domain::health::OperationKind`
(`Assess(Local|Remote)`, `Sync`, `RebuildInventory`, `Clean`).

- `fleet-core` owns operation lifecycle APIs:
  - `start_operation(profile_id, operation_kind) -> session_id`
  - `send_operation_input(session_id, FlowInput)`
  - `cancel_session(session_id)`
- Runtime state is per-profile and stored in
  `AppState.operations_by_profile` (no global sync slot).
- `FlowSessionEvent.operation` carries the domain `OperationKind` directly.
- There is no compatibility alias `FlowOperationKind` and no duplicate
  operation-kind type in `fleet-flow`.
- `Assess(Local)` validates local state and returns the canonical assessment.
- `Assess(Remote)` extends that assessment with remote freshness.
- `Sync` is the only non-destructive self-heal path.
- `Clean` is the only destructive path for unexpected-file deletion.
- `RebuildInventory` is reserved for inventory corruption recovery.

Removed architectural paths that must stay deleted:

- `crates/core/src/features/flow_ops.rs` shim layer
- duplicate-session retry shims driven by error-string matching
- `run_check_flow` wrapper in `flows/operation` (checks use assess runner directly)

## Sync pipeline boundaries

The sync pipeline is split into purpose-built crates with strict dependency rules:

- `fleet-flow`: orchestration + UX contracts (flow execution, prompts, step semantics).
  - Depends on `fleet-manifest` and `fleet-flux`.
  - Must not depend on any `flux-*` crates directly.
- `fleet-manifest`: manifest gathering + transformation.
  - Converts profile sources (Swifty/local) into a desired manifest.
  - Owns manifest stats helpers so `fleet-flow` does not touch `flux-manifest` types.
- `fleet-flux`: the only crate that talks to Flux runtime/services.
  - Owns desired-manifest → Flux desired state conversion, progress bridge, retriever adapter, sync runner, and prune-only execution.
  - Emits progress through the canonical contract `fleet_domain::sync::SyncProgress` (no Flux-specific event wrapper types at the flow boundary).
- `inventory`: storage + scanning only.
  - Flux-agnostic; no `flux-*` dependencies.
  - Owns SQL schema access, scan orchestration, and the inventory-neutral trusted index repository used by reconcile.
  - SQL-backed implementation details remain behind `inventory::trusted_index`; callers must not use ad hoc SQL access directly.

### Dependency graph (enforced by convention)

```
fleet-flow
  ├─ fleet-domain
  ├─ fleet-download
  ├─ inventory
  ├─ fleet-manifest
  └─ fleet-flux      (no direct flux-* deps in flow)

fleet-manifest
  ├─ fleet-domain
  ├─ fleet-download
  ├─ swifty-repo
  └─ flux-manifest   (data + validation only)

fleet-flux   (ONLY place with flux runtime deps)
  ├─ inventory
  ├─ fleet-domain
  ├─ flux-api
  ├─ flux-provider
  ├─ flux-segment-cache
  ├─ flux-types
  ├─ flux-inventory-contract
  └─ retriever
```
