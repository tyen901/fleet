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
  - Owns SQL schema access and exposes a public Flux inventory API boundary (`open_flux_inventory` + `FluxInventoryApi`).
  - SQL-backed implementation details are internal to `inventory`; callers (including `fleet-flux`) must not use SQL implementation types directly.

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
