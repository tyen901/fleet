# Architecture: Implementation Guidance

Use this document to guide design choices, not as a catalog of files, crates,
types, APIs, or current workflows. Code, manifests, tests, and current dependency
contracts are evidence of the repository's actual structure and behavior; they
are not assumed correct. User-intended behavior and relevant contracts govern a
change. Do not restore removed implementation merely to match prose; resolve the
intent from the evidence and revise this guide when its guidance needs to change.

## Boundaries and replacement

Give each behavior one clear owner and keep its public surface as small as the
work requires. Adapters should translate between domains at their boundary,
without duplicating policy or turning a purpose-built path into a generic
framework. This keeps changes local and makes responsibility visible.

When a design is replaced, remove the superseded implementation. Do not retain
duplicate paths, pass-through exports, or compatibility shims unless compatibility
is an explicit requirement. A deleted concept should not be resurrected because
documentation still names it.

## Work that is responsive and bounded

Use bounded parallelism for independent work. Stream work and apply backpressure
instead of accumulating unbounded queues or whole-result collections. Propagate
cancellation through in-flight work promptly and leave each layer able to finish
or abandon its own resources safely. Collect metrics from the work already being
performed; avoid additional scans whose only purpose is reporting.

## Evidence and persistence

Observations must describe what was actually checked or completed. Persist them
atomically so reuse never depends on a partially committed claim. Keep durable,
target-bound reuse evidence separate from transient plans, progress, staging,
or recovery bookkeeping; transient decisions should be recomputed from the
current inputs and observations. This prevents an interrupted run from becoming
false evidence for a later one.

Progress events report observations, not proof of successful terminal completion.
Metadata-only checks cannot establish byte equality.

Keep storage implementation details private to the persistence boundary. For
relational inventory work, model typed facts at the boundary and let SQLite do
set operations, joins, ranking, aggregation, constraints, and mutation. Use
direct `rusqlite` and SQL transactions rather than an ORM, a query-builder layer,
or Rust-side row joins. This preserves transactional truth and lets the database
perform the work it is designed to do.

## Tests and review

Tests should exercise product behavior, Fleet-specific invariants, regressions,
and integration boundaries. They should demonstrate what a change protects, not
only that a dependency behaves as documented.

When reviewing a design change, ask:

- Does one place own the behavior, with a minimal boundary around it?
- Has the replacement removed the old path rather than creating a second one?
- Are concurrency, streaming, backpressure, and cancellation bounded and
  observable under failure?
- Do persisted facts remain truthful, atomic, reusable evidence rather than
  transient execution state?
- Does storage keep its implementation private and let SQL perform relational
  work?
- Do tests cover the intended behavior and the failure or integration boundary
  that makes the change meaningful?
