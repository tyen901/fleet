Understood. Below is an updated, concrete overhaul plan that incorporates your refinements: trait-object based `SyncEngine`, a slightly more granular (but still not “fine-grained”) internal repair pipeline, safe wipe semantics for `sync_fresh`, explicit error mapping (including aborts), and explicit notes for `fleet_app`/`fleet_ui` integration.

---

## 1. Updated target architecture

### Guiding constraints (locked in)

* **Public API** exposes only: `check`, `repair`, `sync_fresh`.
* **`plan` and `apply` are not part of the public API** and are not callable directly.
* **Ports & Adapters**: external dependencies go through `ports` traits, including the store.
* **Filesystem safety** centralized.
* **No fine-grained file explosion**: modules are “coarse but coherent.”

---

## 2. Concrete module tree

```text
crates/sync_engine/src/
  lib.rs
  engine.rs            // public facade SyncEngine + command methods
  model.rs             // public request/response/tuning/errors/types
  ports.rs             // public traits: RemoteRepo, EventSink, Checksummer, StateStore

  pipeline/
    mod.rs              // shared internal context/types/helpers (cancel, semaphores, batching)
    check.rs            // internal implementation of `check`
    sync_fresh.rs       // internal implementation of `sync_fresh`
    repair/
      mod.rs            // orchestrator for repair flow
      planner.rs        // internal planner (strategy selection + parts/range plan)
      applier.rs        // internal applier (staging + remote fetch + patch/full)

  fs.rs                // safe path validation + safe ancestry checks + staging + quarantine helpers
  manifest.rs          // validation/normalization of remote manifests
  util.rs              // time, small utilities (TimestampNs, digest helpers, etc.)
```

### Visibility rules

* `engine.rs`, `model.rs`, `ports.rs` are public.
* Everything else is `pub(crate)` (including the pipeline).
* `lib.rs` exports only the facade, model, and ports (no “export everything” behavior).

---

## 3. Public API contract (final)

### `SyncEngine` uses trait objects internally

This avoids generics bleeding into UI/app types while keeping the implementation flexible.

```rust
pub struct SyncEngine {
    remote: std::sync::Arc<dyn RemoteRepo>,
    store:  std::sync::Arc<dyn StateStore>,
    checksummer: std::sync::Arc<dyn Checksummer>,
}

impl SyncEngine {
    pub fn new(
        remote: Arc<dyn RemoteRepo>,
        store: Arc<dyn StateStore>,
        checksummer: Arc<dyn Checksummer>,
    ) -> Self;

    pub async fn check(&self, req: CheckRequest, sink: &dyn EventSink)
        -> Result<CheckReport, EngineError>;

    pub async fn repair(&self, req: RepairRequest, sink: &dyn EventSink)
        -> Result<RepairOutcome, EngineError>;

    pub async fn sync_fresh(&self, req: SyncFreshRequest, sink: &dyn EventSink)
        -> Result<SyncFreshOutcome, EngineError>;
}
```

### Requests and outcomes (public, in `model.rs`)

* `CheckRequest` (renamed from VerifyRequest)
* `RepairRequest`
* `SyncFreshRequest`
* `CheckReport` (renamed from VerifyReport)
* `RepairOutcome` (as today)
* `SyncFreshOutcome` (new; may embed a `RepairReport` but should be named)

### Key behavioral guarantees

* `check` never writes local files; it only reads local disk and updates store state.
* `repair` writes local files and updates store state; aborts on safety issues.
* `sync_fresh` guarantees post-condition “expected files match remote” by forcing full download (and safe handling of non-expected content).

---

## 4. Ports (public traits)

### 4.1 Remote / Events / Checksummer

Keep as-is structurally, but move to `ports.rs`.

### 4.2 `StateStore` trait (new, minimal but sufficient)

The engine needs:

1. Desired-state metadata (so enabled_mods validation can remain intact).
2. Expected baseline management (replace-on-digest-change).
3. Per-file state snapshots and batch apply.
4. Verified marker set/clear.

A concrete minimal trait shape:

```rust
pub trait StateStore: Send + Sync {
    // Desired state the engine is operating against
    fn desired_state_get(&self) -> Result<Option<DesiredState>, StoreError>;

    // Baseline expected files for the desired state
    fn expected_replace_all_if_digest_changed(
        &self,
        state_id: &str,
        rows: Vec<ExpectedFile>,
        digest_hex: &str,
    ) -> Result<(), StoreError>;

    fn baseline_exists(&self, state_id: &str) -> Result<bool, StoreError>;

    // File state read/write for caching and skip logic
    fn file_state_get_all_for_mod(
        &self,
        state_id: &str,
        mod_id: &str,
    ) -> Result<std::collections::HashMap<String, FileState>, StoreError>;

    fn file_state_apply_batch(
        &self,
        state_id: &str,
        upserts: Vec<FileStateUpsert>,
        deletes: Vec<FileStateDelete>,
    ) -> Result<(), StoreError>;

    fn file_state_delete(
        &self,
        state_id: &str,
        mod_id: &str,
        rel_path: &str,
    ) -> Result<(), StoreError>;

    // Verified marker
    fn verified_get(&self) -> Result<Option<VerifiedState>, StoreError>;
    fn verified_set(&self, state_id: &str, verified_at: TimestampNs) -> Result<(), StoreError>;
    fn verified_clear(&self) -> Result<(), StoreError>;
}
```

Where `DesiredState`, `ExpectedFile`, `FileState`, `VerifiedState`, `FileStateUpsert/Delete`, `TimestampNs` are public types in `model.rs` (or a `store_types` submodule inside `model.rs`).

### Adapter

* `FleetIndexStore` lives outside the public API surface (e.g., in `crates/fleet_app` or an internal `sync_engine::adapters` module if you want it colocated).
* Recommendation: keep it **in fleet_app** to prevent `sync_engine` from re-depending on `fleet_index` directly.

---

## 5. Filesystem safety consolidation (`fs.rs`)

Single boundary for:

* `validate_mod_id`, `validate_rel_path`
* `safe_join_mod_file`
* ancestry safety check (symlink/reparse)
* staging file creation + commit
* quarantine operations
* “safe wipe” helpers (see next section)

### Async vs blocking

* If ancestry checks rely on `std::fs::symlink_metadata`, provide:

  * `fn ensure_no_symlink_ancestors_blocking(...)`
  * `async fn ensure_no_symlink_ancestors(...)` that calls `spawn_blocking` internally
    So pipeline code stays consistently async without accidentally blocking.

---

## 6. `sync_fresh` semantics (updated, safe wipe)

### The requirement

Avoid “wipe the whole mod directory” unless explicitly configured, and even then avoid deleting unknown files.

### Proposed `SyncFreshRequest`

```rust
pub struct SyncFreshRequest {
    pub repo_name: String,
    pub checkout_root: PathBuf,
    pub enabled_mods: Vec<String>,
    pub tuning: SyncFreshTuning,
}

pub struct SyncFreshTuning {
    pub concurrency: RepairTuning,               // reuse existing knobs
    pub safe_wipe: SafeWipePolicy,               // new
    pub unknown_paths: UnknownPathPolicy,        // new
}
```

### Safe wipe policy (recommended)

* **Default**: delete only the set of files known to belong to the mods (from store baseline and/or current manifest).
* Never `rm -rf mod_root` by default.

```rust
pub enum SafeWipePolicy {
    None,                       // do not delete anything before download (overwrite expected)
    ExpectedFromStoreBaseline,   // delete only paths in previous baseline for the desired state
    ExpectedFromRemoteManifest,  // delete only expected paths from fetched manifests
    ExpectedUnion,               // union(baseline, fetched manifest) - recommended for robustness
}
```

### Unknown paths handling

During `sync_fresh`, anything not in expected sets is handled explicitly:

```rust
pub enum UnknownPathPolicy {
    Keep,
    Quarantine,  // recommended default for sync_fresh
    Delete,      // only if user explicitly opts in
}
```

### Quarantine spec (concrete)

* Quarantine root: `{checkout_root}/.fleet/quarantine/{state_id-or-timestamp}/`
* For an unexpected path under `{checkout_root}/{mod_id}/...`, move it to:

  * `.fleet/quarantine/<id>/<mod_id>/<rel_path>`
* Use `rename` when possible; if cross-device rename fails, fallback to copy+delete (bounded and careful).

### `sync_fresh` algorithm (deterministic)

1. Validate enabled_mods.
2. Fetch manifests + capabilities.
3. Update baseline in store.
4. Compute expected-set per mod.
5. **Safe wipe** expected paths only (based on policy).
6. Apply full download for all expected files (internally force `RepairStrategy::Full`).
7. Run unexpected-paths sweep:

   * If `UnknownPathPolicy::Quarantine`, quarantine them.
   * If `Keep`, just report via events.
   * If `Delete`, delete bounded by cap (reuse your existing cap logic).
8. Verify checksums after downloads (same as applier does today).
9. Update store file states and verified marker.

This gives a “fresh sync” without destructive deletion of unknown user content.

---

## 7. Repair pipeline internal split (per your suggestion)

Inside `pipeline/repair/`:

### `planner.rs`

Responsibilities:

* Compute `RepairStrategy` (Skip / Patch / Full)
* Compute patch fetch ranges (coalescing, min range expansion, caps)
* Produce cache hints for store updates
* No async; runs in `spawn_blocking` where it hashes local files.

### `applier.rs`

Responsibilities:

* Staging + atomic replace
* Fetch full file / range(s) from remote
* Writes to staged temp; fsync based on durability
* Final verification (full checksum or part verification)
* Cancellation-aware loops

### `mod.rs` (orchestrator)

Responsibilities:

* Per-mod orchestration: case fix, plan, apply, store updates
* Merge failures, handle aborts, cancellation propagation
* Run unexpected path handler

This separation maintains cohesion without a “dozens of tiny files” outcome.

---

## 8. Cancellation and abort semantics (final)

### Replace atomic “stop scheduling” with a cancellation token

* Internally: `tokio_util::sync::CancellationToken`
* Abort conditions:

  * `UnsafeOnDisk` (symlink ancestor, outside root)
  * other “fatal safety” conditions
* When abort occurs:

  * cancel token fires
  * scheduler stops issuing new tasks
  * in-flight tasks periodically check token and exit early (best-effort)

### Guarantee

* After a safety abort is detected, **no further remote fetches** should be initiated (beyond unavoidable in-flight calls).

---

## 9. Error mapping (explicit spec)

### `EngineError` (public)

Use `thiserror` and provide structured variants. Key requirement: UI must distinguish retryable vs fatal safety aborts.

```rust
#[derive(thiserror::Error, Debug)]
pub enum EngineError {
    #[error("invalid request: {0}")]
    InvalidInput(String),

    #[error("remote error: {0}")]
    Remote(#[from] anyhow::Error), // or a dedicated RemoteError if you want

    #[error("store error: {0}")]
    Store(#[from] StoreError),

    #[error("filesystem safety abort: {0:?}")]
    Abort(AbortReason),

    #[error("operation cancelled")]
    Cancelled,

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}
```

### Rule

* If a safety condition triggers `AbortReason`, the command returns:

  * `Ok(Outcome { aborted: Some(...) })` for `repair`/`sync_fresh` (so UI can show partial progress)
  * and/or `Err(EngineError::Abort(...))` depending on how you want the facade semantics.

Recommendation for consistency:

* **Return outcomes** for repair-like commands:

  * `repair` and `sync_fresh` return `Ok(Outcome)` with `aborted: Some(...)`
* `check` returns `Ok(CheckReport)` even with issues (not an error), unless the process fails structurally (remote/store failure).

---

## 10. fleet_app / fleet_ui integration changes

### Current pain

`fleet_app` is doing too much orchestration and passing DB paths.

### New integration shape

* `fleet_app` constructs:

  * `Arc<dyn StateStore>` via `FleetIndexStore::open(path)` (adapter around fleet_index)
  * `Arc<dyn RemoteRepo>` from current remote implementation
  * `Arc<dyn Checksummer>` from current checksum implementation
* Then:

  * `let engine = SyncEngine::new(remote, store, checksummer);`
* UI layers hold an `Arc<SyncEngine>` and call `engine.check/repair/sync_fresh(...)`.

Net effect:

* UI no longer depends on `fleet_index` directly.
* SQLite path concerns remain in app initialization, not in the engine.

---

## 11. Timestamp consistency (hard requirement)

### Fix `now_ns()`

Your current `now_ns()` returns seconds. This must be corrected and made unambiguous:

* Introduce `pub struct TimestampNs(pub i64);`
* All `mtime_ns` and verified timestamps use `TimestampNs`.
* `file_mtime_ns` returns `TimestampNs` (or `Option<TimestampNs>`).

### Store schema note

The store adapter must ensure it stores timestamps as 64-bit integers. With rusqlite and `i64`, this is typically fine, but the adapter should enforce it.

---

## 12. Execution plan (revised, including Phase 0)

### Phase 0: Baseline tests and invariants

* Ensure existing tests pass (engine + integration).
* Add 2–3 “must hold” integration assertions now (so refactor can target them):

  * Safety abort stops further remote calls.
  * Patch-vs-full behavior remains consistent.
  * Skip logic correctness when cache matches.

### Phase 1: Introduce new public surface without changing behavior

* Add `model.rs`, `ports.rs`, `engine.rs` with facade methods delegating to current flows temporarily.
* Change `lib.rs` exports to only these new modules (stop re-exporting internals).

### Phase 2: Consolidate filesystem safety into `fs.rs`

* Merge safe_path, safe_fs, staging (+ quarantine helpers even if unused yet).
* Update call sites to use `fs::*`.

### Phase 3: Move implementations into `pipeline/`

* Implement `pipeline/check.rs` by moving logic out of flows.
* Implement `pipeline/repair/` (planner + applier + orchestrator).
* Keep behavior identical at first, then fix blocking-in-async issues.

### Phase 4: Implement `sync_fresh` with safe wipe + quarantine

* Add request + tuning knobs.
* Reuse applier to force full downloads.

### Phase 5: Remove old modules and harden

* Delete old public modules (`flows`, `apply`, `plan`, etc.) and any re-exports.
* Enforce cancellation and ensure no blocking FS in async loops.
* Expand tests for:

  * quarantine behavior
  * cancellation propagation
  * timestamp correctness

---

## 13. What “done” looks like (acceptance criteria)

1. `sync_engine` public API is only:

   * `SyncEngine`, `model::*`, `ports::*`
2. No external crate can call planner or applier directly.
3. No `std::fs` calls in async hot paths (except inside `spawn_blocking`).
4. Timestamp units are correct and consistent (`TimestampNs` everywhere).
5. `sync_fresh` does not destructively delete unknown files by default; it quarantines or reports.
6. `fleet_app`/`fleet_ui` can hold `Arc<SyncEngine>` without generics.
