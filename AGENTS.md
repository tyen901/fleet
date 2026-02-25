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
- `cargo run -p fleet-cli -- <command>`: run CLI tasks (sync, repair, clean, profile, launch/join). Example: `cargo run -p fleet-cli -- profile check <profile_id>`.
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

### Enforcement Checklist (Required For Refactors)

- Search for old identifiers/classes and remove remaining references.
- Confirm no unused imports/warnings (`cargo clippy --workspace --all-targets -- -D warnings`).
- Confirm formatting/lint state (`cargo fmt`, `npm run lint:css` when CSS changed).
- Confirm runtime compile health with at least one build/test validation command.

## Testing Guidelines

- Unit tests live alongside Rust modules (e.g., `crates/app/...` with `mod tests`).
- Name test functions descriptively to match behavior (e.g., `repairs_corrupt_inventory`).
- Do not add tests that only verify third-party/library behavior (e.g., serde roundtrips without app-specific logic, std `trim`, basic boolean operators, direct assignment/match passthroughs).
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

- Flow execution is operation-centric and uses one shared kind type:
  `fleet_domain::health::OperationKind`.
- Core operation APIs are:
  - `start_operation(profile_id, operation_kind)`
  - `send_operation_input(session_id, FlowInput)`
  - `cancel_session(session_id)`
- Runtime operation state is per-profile in `AppState.operations_by_profile`.
- Keep remote checks (`CheckRemote`) supported.
- Keep both dashboard delete pathways (`PendingSync` and `UnexpectedReview`) unless explicitly changed.
- Removed paths that should not be reintroduced:
  - `crates/core/src/features/flow_ops.rs`
  - `FlowOperationKind` alias exports
  - `flows/operation::run_check_flow` wrapper
  - start-operation duplicate-session retry shims
