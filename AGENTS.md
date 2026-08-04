# Repository Guidelines

## Project Structure & Module Organization

Fleet is a Rust workspace with a native Dioxus desktop UI.

- `apps/fleet/`: native desktop app entrypoint.
- `apps/fleet-cli/`: `fleet-cli` command-line tool.
- `apps/fleet/assets/`: UI static assets.
- `crates/core/`: core runtime, launch planning, settings/profile management.
- `crates/core/src/operations/`: operation execution, session lifecycle, and progress events.
- `crates/flux/`: `fleet-flux`, Fleet's Flux integration boundary for Swifty-to-Flux input conversion, store source/profile adapters, and `flux::materialize` execution.
- `crates/inventory/`: SQL-native durable materialization facts database and Flux inventory bridge.
- `crates/download/`: download service.
- `crates/swifty-repo/`: Swifty repo cache management.
- `profile_state/`: per-profile runtime state under Fleet config root (inventory SQLite, repo cache artifacts).

## Build, Test, and Development Commands

- `cargo build`: build all workspace crates.
- `cargo fmt`: format Rust code.
- `cargo clippy --workspace --all-targets -- -D warnings`: lint Rust workspace (CI-style).
- `npm run fmt:css`: format CSS with Prettier.
- `npm run lint:css`: check CSS formatting with Prettier.
- `npm run lint:design`: enforce the UI Design Rules (type roles, weights, tracking, uppercase ownership, spacing scale) against the stylesheets.
- `cargo run -p fleet-cli -- <command>`: run CLI tasks (sync, clean, profile, launch/join). Example: `cargo run -p fleet-cli -- profile check <profile_id>`.
- `cargo run -p fleet`: run the native UI.
- `cargo test`: run Rust unit tests across the workspace.

## Verification Requirement

Always run a build and at least one validation step (tests and/or lint) before reporting work as complete. If a command cannot be run, state that explicitly.

## Automated Native UI Render Flow

Use the automated render flow after UI or CSS changes. It runs Fleet at a fixed 420×560 portrait viewport, drives every view through the native WebView2 debugging endpoint, checks the profile-card, settings Save/Cancel, and additional-mod interactions, and saves PNG captures under `target/ui-render/captures/`.

1. Close other development instances of Fleet, then build the native app with `cargo build -p fleet`.
2. Run `npm run render:ui` from the workspace root; it builds first, because CSS is embedded with `include_str!` and a CSS-only edit will not appear otherwise. Node.js 22 or newer and the WebView2 runtime are required.
3. Inspect every PNG in `target/ui-render/captures/`. The flow captures onboarding, profiles, settings top and bottom, new profile, profile overview, edit profile, the additional-mod list before and after Add, full-sync confirmation and progress, and delete confirmation.

Sync progress is simulated, not real. The runner sets `FLEET_SIMULATE_SYNC=1`, which makes sync and full sync emit a scripted progress sequence and return an up-to-date report without touching the network, repo cache, inventory, or profile destination. `FLEET_SIMULATE_SYNC_HOLD_PERCENT` parks that sequence at a given percentage until the operation is cancelled, so the progress capture is reproducible. Neither variable is set outside the render flow.

The runner creates isolated dummy `settings.json`, `profiles.json`, profile files, and inventory state under `target/ui-render/`; it sets `FLEET_CONFIG_DIR` only for the child process and never reads or writes the normal user config directory. The dummy profile points at the closed loopback endpoint `http://127.0.0.1:9/repo.json`, so the render flow does not depend on a live repository. Override the disposable root with `FLEET_UI_TEST_ROOT` or the debugging port with `FLEET_UI_TEST_CDP_PORT` when required; never point `FLEET_UI_TEST_ROOT` at real Fleet configuration or profile data.

## UI Design Rules

- A container carries even padding on all four sides and spaces its children with `gap`. Do not split one container's padding across its children.
- Spacing uses the `--space-1`..`--space-6` scale (4/8/12/16/24/32). No px gap, padding, or margin outside `tokens.css`.
- Type: four sizes only (`--text-title`, `--text-body`, `--text-label`, `--text-caption`) and two weights (`--weight-regular`, `--weight-medium`). No px font sizes outside `tokens.css`.
- Uppercase and `--tracking-label` belong to the section label declared in `components/typography.css`. Nothing else is uppercased in CSS.
- UI strings are written sentence case in Rust. Casing for display is a CSS concern.
- Button weight: exactly one `Primary` per screen; `Secondary` only for a real alternative beside a primary or a standalone interrupt; `Ghost` for everything else. A disabled primary renders flat, not filled.
- Icons appear only inside `IconButton`, which requires a label used as both `aria-label` and tooltip.
- An outline is what marks something as a control. Every button has one: `Primary` fills, `Secondary` uses `--clr-border-emphasis`, `Ghost` uses the lighter `--clr-border`. A static readout has no outline. Every button carries `--control-inline-padding` horizontally, icon buttons included, and aligns by its border; nothing hangs into the gutter.
- Page chrome is `PageHeader`: a single row holding back, title, status, and the page's right-aligned actions. Actions are quiet by default so the row stays readable at the narrowest width.
- Interactive and non-interactive controls are told apart by colour, not by opacity. An editable control uses `--clr-input-border` and `--clr-text-primary`; a readonly or disabled one uses `--clr-input-border-quiet` and `--clr-input-fg-quiet`. Static readouts use the quiet foreground too.
- A label never outranks the value it labels: `.field__label`, `.form-field__label`, and `.field-row__title` share one treatment (`--text-label`, `--weight-medium`, `--clr-text-secondary`).
- Control text is `--text-label`, matching button labels, so a row mixing an input with a button sits on one size. `--text-body` is for running prose.
- An inline row is `--control-height` tall whether or not it holds a control. A row does not grow because a button landed in it.
- There are two row shapes and no others. A stacked row puts the label above a full-width control, for values that can be long. A `FieldRow` puts the label left and a small trailing affordance right, for toggles, buttons, and short readouts.
- Confirmation is inline beneath the triggering control, never a modal.
- Editing a record is a mode on its page, not a separate screen. Both modes render the same controls; read mode marks them `readonly` rather than substituting a different element, so a row cannot shift when the mode is toggled.
- Show a status only when it is actionable; a healthy profile shows none.

## Coding Style & Naming Conventions

- Rust: follow `rustfmt` defaults (4 spaces, snake_case for functions/modules, PascalCase for types).
- CLI flags: kebab-case (e.g., `check-for-updates`).
- Crate naming: `fleet-*` for most crates (e.g., `fleet-cli`, `fleet-core`); `inventory` is the consolidated inventory subsystem crate.
- Storage output must be compact by default. Do not pretty-print JSON or other persisted cache/state files unless the file is explicitly user-facing or hand-edited configuration.

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

- Fleet inventory is the durable materialization facts database for the managed target scope.
- It persists the current managed target-relative path snapshot, reusable local file facts, and reusable segment metadata required for local reuse.
- The managed-path snapshot may contain paths that do not have reusable file facts.
- There is no initialized baseline state.
- Do not persist transient run state, sync progress, staging state, commit state, delete plans, audit history, recovery journals, manifest intent, or future desired state in the inventory.
- Do not add run tables, audit tables, heartbeat metadata, generations, pending-delete markers, staging paths, or similar operational bookkeeping back into the inventory schema.
- Do not treat the inventory as a general runtime state store, workflow cache, or dumping ground for convenience data.
- If a value is derived from the current manifest, current disk scan, or current in-memory operation, it does not belong in the inventory unless it is part of the managed path snapshot or reusable local file facts.
- Operational decisions such as cleanup candidates, materialization planning, remote comparisons, and temporary progress belong in core/Flux/runtime layers and must be recomputed, not persisted in inventory.
- Flux integration must go through a narrow inventory-owned bridge. Do not expose broad public adapter types or public low-level writeback helpers just because another crate might use them.
- Keep SQL implementation details private to `crates/inventory`. Callers must not use ad hoc SQL access, schema-coupled logic, or inventory-internal helper types.
- When changing inventory APIs, prefer making the public surface smaller. Do not preserve legacy exports, pass-through re-exports, or compatibility wrappers without explicit instruction.
- Inventory is SQL-native: feed typed facts into SQLite temp tables and let SQLite perform set operations, joins, ranking, aggregation, constraints, and mutation. Rust should decode final boundary DTOs, not perform inventory row joins or set diffs.
- Use direct `rusqlite` and SQLite features (`Connection::transaction`, `execute_batch`, `prepare_cached`, UPSERT, window functions, temp tables, `EXPLAIN QUERY PLAN`, `PRAGMA optimize`). Do not add ORMs, query builders, repository layers, custom statement caches, or connection wrapper frameworks.

### Inventory Refactor Checklist

- Confirm new persisted fields are materialization facts, not operational state.
- Delete superseded schema columns/tables in the same change; do not leave dormant compatibility data behind.
- Search for callers attempting to store manifest/planning/run/audit data in inventory and move that logic outward.
- Confirm inventory docs and architecture docs still describe managed snapshot and reusable-fact ownership accurately.
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

PRs should pass `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `npm run lint:css`, `npm run lint:design`, and `cargo test`.

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

- `fleet-core` owns the only operation/session runtime; no separate pipeline crate exists.
- Core operation APIs are:
  - `start_operation(profile_id, operation_kind)`
  - `cancel_session(session_id)`
  - `await_finished(session_id)`
- Runtime operation state is per-profile in `AppState.profile_runtime_by_id`.
- Operation events are observational only; terminal completion is read from the core session registry.
- Supported operation kinds are `CheckRepo`, `CheckInventory`, `Sync`, and `CleanupUnexpectedFiles`.
- `CheckInventory` is read-only and reports local state and whether sync or cleanup is required.
- `Sync` is the primary materialization and self-heal path. It calls `fleet_flux::materialize(...)` directly; Flux/prodash owns live materialization progress.
- `CleanupUnexpectedFiles` deletes approved unexpected candidates only, preserves protected root entries, updates inventory, and returns a reassessed inventory report.
- Inventory corruption is surfaced by `CheckInventory` and repaired by `Sync`.
- Check and sync logic may read materialization inventory facts, but must not push operational state back into inventory.
- Flux entry-point ownership:
  - `crates/flux` is the only Fleet crate that should convert Fleet/Swifty shapes into Flux materialization inputs or call `flux::materialize`.
  - `swifty-repo` owns Swifty repo cache/probe/sync behavior. Do not recreate repo cache logic in `fleet-flux`.
  - `fleet-inventory` owns the Flux `LocalInventory` and `InventoryUpdateSink` bridge.
- Removed paths that should not be reintroduced:
  - `crates/pipeline/`
  - `crates/core/src/features/flow_ops.rs`
  - `crates/manifest/`
  - `crates/reconcile/`
  - `FlowOperationKind` alias exports
  - duplicate-session retry shims driven by parsed error strings
