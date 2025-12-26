# fleet_index + fleet_sync: Pragmatic Guide (Architecture & Rules)

This document explains what the `fleet_index` and `fleet_sync` crates are responsible for, what they are not responsible for, and the operational rules they must obey.

It is intentionally light on implementation detail. It should remain useful even as the internal structure changes.

---

## Mental model

Think of the system as two layers:

- **`fleet_sync`** is the *doer*: it looks at a checkout on disk, compares it to what the remote says should exist, and (optionally) fixes it.
- **`fleet_index`** is the *memory*: it persists lightweight facts about the checkout and the desired state so future runs can be faster and safer.

A useful way to frame it:

- `fleet_sync` answers: **“How do we make the checkout match the remote state?”**
- `fleet_index` answers: **“What do we already know about this checkout and state?”**

### Key terms (stable, not code-specific)

- **Checkout root**: The folder containing enabled mod folders.
- **Enabled mod**: A mod folder the user has chosen to sync (e.g., `@foo`).
- **Remote state**: The remote’s description of which files exist for each enabled mod, plus their sizes and checksums (and possibly part information for large files).
- **Desired state**: “Which remote repo are we using?” + “Which mods are enabled?” + “Which remote state identifier do those imply?”
- **Baseline**: The expected file set for a particular remote state (used as a reference to detect drift).
- **Verified state**: A marker that the checkout was fully checked and found clean for the current desired state.
- **File-state cache**: Per-file metadata used to avoid re-hashing unchanged files where safe.

---

## `fleet_index` (the memory): concerns and non-concerns

### What `fleet_index` does

`fleet_index` is responsible for storing and retrieving *local, derived facts* that help `fleet_sync` operate efficiently and consistently across runs.

It owns:

- **Persisted identity of the desired state**  
  Enough information to recognize “this checkout is aiming at remote X, with mods {A,B,C}”.

- **Baseline and verification bookkeeping**  
  Whether a baseline exists for a state, and whether the checkout is currently marked as verified for that state.

- **File-state caching**  
  The minimal safe metadata needed to decide whether a file can be treated as unchanged since it was last verified (e.g., timestamps and sizes; exact details are intentionally not specified here).

- **Index durability as a best-effort optimization**  
  The index is allowed to be missing, cleared, or rebuilt. It must never be required for correctness—only for speed and for skip-logic safety.

- **Garbage-collection hooks**  
  The ability to remove no-longer-relevant cached/indexed data for old states and old checkouts (lazily, not on every run).

### What `fleet_index` does *not* do

`fleet_index` does not:

- Interpret remote manifests beyond what is needed to store derived facts.
- Perform file I/O on the checkout (no downloading, patching, or repair actions).
- Decide which mods are enabled or how they are named (that’s upstream configuration).
- Decide remote transport details (auth, HTTP behavior, retries).
- Provide real-time filesystem watching or incremental change tracking (it can cache facts, but it does not “monitor”).

---

## `fleet_sync` (the doer): concerns and non-concerns

### What `fleet_sync` does

`fleet_sync` is responsible for executing *a full pass* over the enabled mods to reach (or confirm) an on-disk state consistent with the remote.

It owns:

- **Verification**  
  Read-only evaluation of the checkout:
  - What’s missing?
  - What’s extra?
  - What’s corrupted or mismatched?
  - What’s unsafe to touch?

- **Repair**  
  Controlled mutation of the checkout to align with the remote, including:
  - Downloading or updating missing / corrupted files.
  - Quarantining unexpected files when configured to do so.
  - Cleaning up empty directories that become irrelevant after quarantine/repair.

- **Planning and ordering**  
  Given remote expectations and local observations, decide what needs to be done first to minimize risk and avoid leaving partial state.

- **Operational event reporting**  
  Emit informational events about progress and issues (useful for UI/logs), without making those events part of the correctness model.

### What `fleet_sync` does *not* do

`fleet_sync` does not:

- Manage user configuration (which repo, which mods, where the checkout lives).
- Provide long-term persistence of state (it delegates that to `fleet_index`).
- Act as a general-purpose file manager (it only touches what it must under the checkout root).
- Attempt to “merge” user modifications into expected files.
- Guarantee perfect cleanup of arbitrary unexpected content (it follows safety rules and caps).

---

## Operational rules

These are the rules the crates “live by”. They are more important than any particular internal design.

### 1) Source of truth

- **Remote manifests are authoritative** for what should exist, where it should exist (relative paths), and how it should validate (sizes/checksums).
- **The local disk is not trusted**; it is only observed and corrected.
- **The local index is not truth**; it is an optimization and a safety aid. If it is missing or cleared, the system must still behave correctly (just slower).

### 2) Full-pass discipline (no partial truth)

- Verify and repair are designed to perform a **full pass across all enabled mods**.
- Do not stop at “first error” except for **hard safety aborts** (see symlink and path safety rules).
- The “verified” marker must only be set when the full pass completes cleanly for the current desired state.

### 3) Path safety is non-negotiable

- All expected paths must be **relative**, **normalized**, and **confined to the mod root**.
- Any path that would escape the checkout root (absolute paths, `..`, platform tricks) must be rejected and treated as a remote/data error.
- The engine must never write outside the checkout root/mod roots.

### 4) Symlinks and reparse points: strict handling

Symlinks/reparse points are a major safety boundary. Treat them consistently across verify, skip logic, quarantine, and repair.

- **Do not follow symlinks/reparse points while scanning.**
- If the **final expected path** is a symlink/reparse point (i.e., the thing that should be a regular file isn’t), treat it as **not-a-regular-file**:
  - Verify should report it as a problem.
  - Repair should not try to “hash through” it or mutate through it.
- If **any ancestor directory** on the path to an expected file is a symlink/reparse point:
  - Verify should report it as **unsafe on disk**.
  - Repair must **abort** the operation (do not proceed), because the engine can no longer guarantee it is operating within the intended directory tree.
- Quarantine should **not attempt to move** symlinks/reparse points.

### 5) Repair staging and replacement safety

- All repairs must be done in a way that avoids leaving partially written expected files behind.
- A safe rule of thumb: **build the correct bytes somewhere safe, validate them, then replace**.
- Validation occurs before replacement, using the remote-provided expectations (size/checksum).

### 6) Skip-repair is allowed only when it is provably safe

Skip-repair exists to make “no-op” runs fast, but it must be conservative.

Skip-repair can only happen when:

- The current desired state matches the last known verified state.
- A baseline exists for that state.
- A local check indicates nothing has changed for expected files, using cached file-state metadata.
- The cache coverage is complete for the expected file set (no “unknowns”).
- All safety gates that would apply to repair also pass (especially ancestor symlink checks).

If any of those conditions fail, the engine must do the normal verify/repair workflow.

### 7) Quarantine behavior is bounded and explicit

When configured to quarantine unexpected content:

- Unexpected files/dirs under enabled mod roots may be moved aside into a quarantine area under the checkout.
- Symlinks/reparse points are not quarantined (see safety rules).
- Quarantine should be **bounded** (e.g., a max total bytes limit). If the bound is hit:
  - Record/emit the condition.
  - Leave remaining unexpected content in place rather than risking uncontrolled movement.

### 8) Empty directory cleanup is allowed (and should be predictable)

After quarantine and/or repair, the engine may delete empty directories that are no longer relevant.

Constraints:

- Never delete the mod root directories themselves.
- Do not chase symlinks/reparse points to “prove” emptiness.

### 9) Lazy garbage collection

- Cleanup of old index data and old auxiliary on-disk data should be **lazy**.
- GC must never remove data required to complete a verify/repair of the current desired state.
- GC is allowed to be incomplete; correctness must not depend on it.

### 10) Events are informational only

- Emitted events (progress, problems, counters) are for logs/UI/observability.
- They must not be used as a control plane for correctness.
- Event schemas should prefer stability: additive changes over breaking changes.

---

## What we intentionally do not support

This is the “non-goals” list that prevents accidental scope creep.

- Supporting arbitrary user edits to expected files (the system validates against the remote, it does not merge).
- Following symlinks/reparse points to accommodate unusual folder topologies.
- Being a generic backup/restore tool.
- Optimizing network transport beyond what is needed for correctness (transport tuning can evolve independently).
- Strong guarantees about preserving unexpected local content beyond the quarantine rules and bounds.

---

## Extension guidelines (how to add capabilities without breaking invariants)

When extending either crate, prefer changes that strengthen safety and keep correctness independent of the index.

1) **If you add new index data, keep it derived and optional**  
   - It must be derivable from remote expectations + local observation.  
   - Behavior must remain correct if it is absent or cleared.

2) **If you add new fast paths, make them conservative**  
   - New “skip” decisions should fail closed: if unsure, do the full work.

3) **If you add a new repair strategy, keep the staging/validation contract**  
   - Build/patch bytes in a safe location, validate, then replace.

4) **If you change scanning or quarantine, keep safety rules aligned**  
   - Verify, skip logic, quarantine, and repair must agree about how symlinks/reparse points are treated.

5) **If you evolve remote manifest capabilities, validate defensively**  
   - Treat the remote as potentially buggy or malicious. Validate path shapes, sizes, and internal consistency.

---

## Common pitfalls

- Treating cached file-state metadata as “truth” rather than a hint.
- Allowing verify or repair to return early, then accidentally marking the state as verified.
- Handling symlinks differently in verify vs repair vs skip logic.
- Accepting remote-provided paths that escape mod roots.
- Over-quarantining (moving too much) without a strict cap and clear reporting.

---

## Glossary

- **Checkout root**: The parent folder containing enabled mods.
- **Enabled mod**: A selected mod folder that participates in verify/repair.
- **Relative path**: A normalized path within a mod (never absolute, never `..`).
- **Desired state**: The target remote repo + enabled mods and their implied remote state identifier.
- **Baseline**: Expected file set for a remote state.
- **File-state cache**: Metadata used to conservatively infer “unchanged”.
- **Verified state**: Marker that a full pass succeeded cleanly for the desired state.
- **Quarantine**: Moving unexpected content aside (bounded and safety-checked).
