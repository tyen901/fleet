# Fleet architecture

## Ownership

The [desktop app](apps/fleet) and [CLI](apps/fleet-cli) use
[fleet-core](crates/core) for profile, sync, and launch workflows. Keep shared
policy in core and presentation in the apps. [fleet-flux](crates/flux) adapts
Swifty manifests into ordered recipes and repository ranges. Fleet scans local files, verifies Swifty MD5 parts, and owns check results and the inventory. Flux consumes Fleet’s verified local recipes to plan and publish files.

## Operations

Pass cancellation through to workers and keep concurrency and queues bounded.
Hold the profile operation lock until workers exit, including on cancellation.
Derive progress from work already performed and report completion from the
operation result.

## Target state

Fleet stores stamped recipes in `<target>/.fleet/swifty-inventory-v1.redb`. A fast check compares the namespace and current handle-derived stamps with cached recipes. Validation reads local payloads. Sync scans unknown and changed files, supplies available extents to Flux and records stamps returned by publication. When the folder already matches, Fleet finishes sync directly. A cold scan commits completed, stamped files in batches. Restarting sync reuses those observations and rereads the unfinished batch. Explicit byte verification always reads the requested files. Repository metadata stays in the application cache.

Flux owns the target lock, staged outputs and the recovery files needed to preserve local bytes during cyclic replacements. Fleet protects its private directory at every depth.

## Errors

At application and updater boundaries, report failures with authored,
actionable messages and log technical details separately. `ApiError`
constructors take static messages to keep raw diagnostics out of the UI.
