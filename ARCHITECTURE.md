# Fleet architecture

## Ownership

The [desktop app](apps/fleet) and [CLI](apps/fleet-cli) use
[fleet-core](crates/core) for profile, sync, and launch workflows. Keep shared
policy in core and presentation in the apps. [fleet-flux](crates/flux) adapts
Swifty inputs to the pinned Flux engine, which checks, verifies, and
materializes target files. [fleet-inventory](crates/inventory) owns persisted
file observations.

## Operations

Pass cancellation through to workers and keep concurrency and queues bounded.
Hold the profile operation lock until workers exit, including on cancellation.
Derive progress from work already performed and report completion from the
operation result.

## Target state

Fleet keeps verified file observations in `<target>/.fleet/observations.sqlite`
with its session lock beside it. Flux keeps its operation lock, staging, and
recovery state in `<target>/.fleet/flux`. Reserve `.fleet` at every depth in the
managed tree. Repository downloads live in the application cache.

Commit target-bound observations atomically. Keep transient plans and progress
separate from reusable evidence. Metadata alone cannot establish byte equality.
Keep SQLite details inside inventory and use SQL transactions and set operations
for relational work.

## Errors

At application and updater boundaries, report failures with authored,
actionable messages and log technical details separately. `ApiError`
constructors take static messages to keep raw diagnostics out of the UI.
