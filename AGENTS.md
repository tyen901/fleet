# Contributor Guide

## Start with the code

Use the code, workspace manifests, tests, and current dependency contracts to find
the relevant entrypoint, callers, and coverage before changing behavior. They
define the repository's current structure and behavior. This guide is a working
workflow, not a reason to restore code that has been removed.

Use [ARCHITECTURE.md](ARCHITECTURE.md) for engineering decision guidance. Resolve
an apparent conflict by investigating the code and the intended behavior, then
update stale documentation as part of the change.

## Build and verify

- `cargo build --workspace --locked` builds the workspace.
- `cargo test` runs Rust tests.
- `cargo fmt` formats Rust; use `cargo fmt --check` when checking a change.
- `cargo clippy --workspace --all-targets -- -D warnings` is required for
  refactors and is the CI-style Rust lint.
- `npm run fmt:css`, `npm run lint:css`, and `npm run lint:design` format and
  check stylesheets and design rules.
- `cargo run -p fleet-cli -- <command>` runs CLI workflows; `cargo run -p fleet`
  starts the desktop app.

Before reporting work complete, run a build and at least one relevant test or
lint. Run the strict Clippy command for refactors. Use the checks that cover the
changed surface, and state any check that could not run.

## UI changes

Follow the existing tokens and components. Containers use even padding and
`gap`; use the spacing scale rather than ad hoc values. Keep the established
four type roles, two weights, sentence-case strings, and label tracking rules; a
label never outranks its value. Do not add wrapper elements for content or
background fills to panels and sections unless the request calls for them.

Keep controls recognisable and consistent: buttons retain their outlines and
their intended hierarchy. Use exactly one primary button per screen; a disabled
primary is flat. Use secondary only for a genuine alternative or a standalone
interrupt, and ghost for other actions. Icon buttons have an accessible label
and tooltip; editable, readonly, and static values have distinct established
treatments.
Use stacked fields for long values and the existing compact field row for short
values or trailing actions. Keep confirmation inline, retain the same controls
when switching between read and edit modes, and show status only when it is
actionable.

After UI or CSS work, build the native app and run `npm run render:ui`. Inspect
every capture under `target/ui-render/captures/`. The render run must use its
disposable, isolated configuration and must never point at real user data or
attach to an existing development app's debugging endpoint.

## Implementation and review

Finish replacement work by deleting superseded code, selectors, helpers, imports,
comments, and compatibility shims unless compatibility is explicitly required.
Keep persisted output compact unless it is deliberately user-facing or hand-edited
configuration.

Write behavior-focused tests for Fleet-specific invariants, regressions, and
integration boundaries. Avoid tests that only demonstrate framework or standard
library behavior.

Use `feature/*` branches for new work. Make commits short and imperative. Pull
requests should explain the resulting behavior, the material changes, and the
validation performed; include UI captures when the UI changed.
