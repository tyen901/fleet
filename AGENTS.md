# Repository Guidelines

## Project Structure & Module Organization

Fleet is a Rust workspace with a native Dioxus desktop UI.

- `apps/fleet/`: native desktop app entrypoint.
- `apps/fleet-cli/`: `fleet-cli` command-line tool.
- `apps/fleet/assets/`: UI static assets.
- `crates/core/`: core runtime, launch planning, settings/profile management.
- `crates/flow/`: sync flows and manifest handling.
- `crates/flux/`, `crates/inventory/`, `crates/download/`: inventory, download, and sync subsystems.
- `crates/swifty-repo/`: Swifty repo cache management.
- `profile_state/`: per-profile runtime state under Fleet config root (inventory SQLite, repo cache artifacts).

## Build, Test, and Development Commands

- `cargo build`: build all workspace crates.
- `cargo fmt`: format Rust code.
- `cargo clippy --workspace --all-targets -- -D warnings`: lint Rust workspace (CI-style).
- `npm run fmt:css`: format CSS with Prettier.
- `npm run lint:css`: check CSS formatting with Prettier.
- `cargo run -p fleet-cli -- <command>`: run CLI tasks (sync, clean, profile, launch/join). Example: `cargo run -p fleet-cli -- profile check <profile_id>`.
- `cargo run -p fleet`: run the native UI.
- `cargo test`: run Rust unit tests across the workspace.

## Verification Requirement

Always run a build and at least one validation step (tests and/or lint) before reporting work as complete. If a command cannot be run, state that explicitly.

## Coding Style & Naming Conventions

- Rust: follow `rustfmt` defaults (4 spaces, snake_case for functions/modules, PascalCase for types).
- CLI flags: kebab-case (e.g., `check-for-updates`).
- Crate naming: `fleet-*` for most crates (e.g., `fleet-cli`, `fleet-core`); `inventory` is the consolidated inventory subsystem crate.

## Legacy Code & Code Rot Policy

- Prefer hard deletion over soft deprecation when replacing code in scope.
- Do not keep compatibility shims, duplicate paths, or legacy branches unless explicitly requested.
- Remove dead code in the same change set: unused selectors, props, helper functions, imports, stale files, and obsolete comments.
- Do not leave commented-out code or placeholder TODOs for removed behavior.
- If a refactor renames or replaces a concept, remove the old identifier usage from Rust and CSS in the same pass.
- Keep one authoritative implementation path per behavior.
- UI policy: never add container wrapper elements for UI content unless explicitly requested.
- UI policy: never add background fill to UI panels/sections unless explicitly requested.

## Inventory Enforcement Rules

- The inventory is authoritative finalized local file truth only.
- Persist only finalized on-disk file facts and segment metadata required for trust and retrieval.
- Do not persist transient run state, sync progress, staging state, commit state, delete plans, audit history, recovery journals, manifest intent, or future desired state in the inventory.
- Do not add run tables, audit tables, heartbeat metadata, generations, pending-delete markers, staging paths, or similar operational bookkeeping back into the inventory schema.
- Do not treat the inventory as a general runtime state store, workflow cache, or dumping ground for convenience data.
- If a value is derived from the current manifest, current disk scan, or current in-memory operation, it does not belong in the inventory unless it becomes finalized trusted file truth.
- Operational decisions such as delete candidates, reconcile planning, remote comparisons, and temporary progress belong in flow/reconcile/runtime layers and must be recomputed, not persisted in inventory.
- Flux integration must go through a narrow inventory-owned bridge. Do not expose broad public adapter types or public low-level writeback helpers just because another crate might use them.
- Keep SQL implementation details private to `crates/inventory`. Callers must not use ad hoc SQL access, schema-coupled logic, or inventory-internal helper types.
- When changing inventory APIs, prefer making the public surface smaller. Do not preserve legacy exports, pass-through re-exports, or compatibility wrappers without explicit instruction.

### Inventory Refactor Checklist

- Confirm new persisted fields are finalized-truth fields, not operational state.
- Delete superseded schema columns/tables in the same change; do not leave dormant compatibility data behind.
- Search for callers attempting to store manifest/planning/run/audit data in inventory and move that logic outward.
- Confirm inventory docs and architecture docs still describe finalized-only ownership accurately.
- Confirm no unused public exports remain after the change.

### Enforcement Checklist (Required For Refactors)

- Search for old identifiers/classes and remove remaining references.
- Confirm no unused imports/warnings (`cargo clippy --workspace --all-targets -- -D warnings`).
- Confirm formatting/lint state (`cargo fmt`, `npm run lint:css` when CSS changed).
- Confirm runtime compile health with at least one build/test validation command.

## Testing Guidelines

- Unit tests live alongside Rust modules (e.g., `crates/app/...` with `mod tests`).
- Name test functions descriptively to match behavior (e.g., `repairs_corrupt_inventory`).
- Project tests must never exist solely to verify external crate, standard library, or framework behavior.
- Do not add tests that only verify third-party/library behavior (e.g., serde roundtrips without app-specific logic, std `trim`, basic boolean operators, direct assignment/match passthroughs).
- If a test would still be valid in the same form after replacing Fleet code with a direct call to an external crate API, it does not belong in this project.
- Prefer behavior-focused tests that cover Fleet-specific logic, regressions, invariants, integration boundaries, or previously broken paths.
- Delete or avoid legacy/coincidental tests that pass without asserting current behavior.

## Linting & Formatting

PRs should pass `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `npm run lint:css`, and `cargo test`.

## Commit & Pull Request Guidelines

- Commits are short, imperative summaries; optional prefixes like `refactor:` are used occasionally.
- PRs should include: purpose, major changes, testing notes (`cargo test`, lint/format checks), and screenshots for UI changes.
- Link relevant issues or discussions when applicable.

## Configuration & Local State

- Per-profile runtime state is stored under `<config_root>/profile_state/<profile_key>/` (inventory SQLite, repo cache, artifacts).
- User configuration is stored in the platform config directory as `settings.json` and `profiles.json`. You can override the config directory via `FLEET_CONFIG_DIR`.

## Launch System Notes

- Launch args are assembled per-profile, falling back to defaults in settings.
- Enabled mods are sourced from the Swifty repo cache under profile state (`<config_root>/profile_state/<profile_key>/repo_cache`) and translated into `-mod=...`.
- Linux launch methods use Proton-style mod paths when applicable.
- Custom launch commands are only used when the launch method is set to `custom`.

## Operation Flow Notes

- Pipeline execution is operation-centric and uses one shared kind type:
  `fleet_domain::health::OperationKind`.
- Core operation APIs are:
  - `start_operation(profile_id, operation_kind)`
  - `cancel_session(session_id)`
- Runtime operation state is per-profile in `AppState.profile_runtime_by_id`.
- Assess operations are unified under `OperationKind::Assess(Local|Remote)`.
- `Assess` is read-only and should stay fast. It reports local state and whether sync or recovery is required.
- `Sync` is the primary reconcile and self-heal path, and may delete truly unexpected residue after manifest-aware inventory stabilization and audit.
- Inventory corruption is surfaced by `Assess` and repaired by `Sync`.
- Assess and sync logic may read finalized inventory truth, but must not push operational state back into inventory.
- Keep remote assessment supported through `Assess(Remote)`.
- Keep both dashboard delete pathways (`PendingSync` and `UnexpectedReview`) unless explicitly changed.
- Removed paths that should not be reintroduced:
  - `crates/core/src/features/flow_ops.rs`
  - `FlowOperationKind` alias exports
  - `flows/operation::run_check_flow` wrapper
  - duplicate-session retry shims driven by parsed error strings
