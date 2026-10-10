# Materialization performance

Fleet uses four bounded r2d2/r2d2_sqlite connections for observation reads and
short observation transactions. Repeated segment and occurrence statements use
rusqlite's statement cache. A separate terminal writer owns the atomic final
transaction: its producer can read observations through the pool without holding
a pool lease across that callback. Observation writers retain the session lock
until they finish or discard their temporary spool. The SQLite inventory binds a
target/profile once, interns content and immutable recipes by integer ID, and
references one recipe from each observed file. No full inventory or segment
cache is added to application memory.

## Real profile comparison

For replacement schemas, use the `fleet-inventory` example
`cached-materialization` and `scripts/compare-cached-materialization.ps1`.
The example loads an explicitly supplied cached Swifty release and invokes the
Fleet adapter; it never refreshes repository metadata or edits registered
profiles. Give each binary a separately prepared inventory for the same target.
Its `verify` mode seeds an empty inventory with a full scan; record that setup
cost separately. Run all warm no-op trials before corruption trials so one
binary's repair does not invalidate another binary's warm file evidence.

The script alternates clean release executables, records process and operation
time, CPU time, peak working set, cached revision and actual outcomes, and
verifies the original file SHA-256 after every trial. A verified external backup
restores any unsuccessful repair. These adapter measurements omit CLI startup
and repository refresh; do not combine them with CLI timings below. Build and
record both source revisions and executable hashes before timing.
Process time includes cached input loading and immutable recipe registration;
the separately reported operation time starts after that setup.

`cargo run --release -p fleet-inventory --example inventory-size -- <database>`
uses SQLite `dbstat` in read-only mode to report cell payload and pages for every
table and index. Include both index and table payload when reporting compaction;
allocated file size alone does not measure stored representation.

`scripts/compare-materialization.ps1` compares two release CLI binaries against
the same registered profile. Full byte validation must establish a clean baseline
before running it. Choose a payload byte inside a known Swifty piece, supply the
original file SHA-256 and expected missing piece size, and keep results outside
the target. The script deliberately corrupts that byte and uses actual profile
sync to repair it. A verified external backup restores the file if repair fails.

Example for the verified local PCA profile:

```powershell
./scripts/compare-materialization.ps1 `
  -Baseline <baseline-release.exe> -Candidate <candidate-release.exe> `
  -ConfigDir "$env:APPDATA/fleet/manager/config" -ProfileId larx -Target C:/pca `
  -RepairPath '@ace/addons/ace_advanced_throwing.pbo' -Offset 58372 `
  -ExpectedSha256 fd13b93183112da1a11e7abfde76aabcc0eb4845a8852ad833d29456c44d3ffb `
  -ExpectedFetchedBytes 2387 -OutputDir target/real-performance/new-run -Trials 3
```

Each trial runs both no-op and repair. Baseline/candidate order alternates to
reduce order bias. Builds, other tests, and other profile operations must finish
before timing. These runs retain the OS cache and durable observations; they are
not cold-start measurements. Record the compiler and revision alongside the
report when building binaries. `identity.json` records binary hashes, machine,
profile source, input hash and workload arguments. `runs.json` records each wall
time, peak process working set sampled every 25 ms, and actual Flux work counters.
Raw stdout/stderr and the verified original are retained beside the report.

The harness rejects missing outcome counters, unexpected deletes or file counts,
data work during a no-op, non-minimal repair retrieval, and any restored full-file
hash/length mismatch. Timing is descriptive, not a flaky pass/fail threshold.

This live profile test complements Flux's representative-cardinality, cold
inventory, incremental shared-content, and interrupted-resume workloads. The small
PBO repair proves incremental selection through the real network source; it does
not measure a 40 GB download, bandwidth saturation, cold disk performance, or
whole-modpack byte validation speed. Keep the fixture and raw artifacts ignored.

## Recorded comparison — 2026-09-05

Baseline: Fleet `18b0e39` with Flux `94cdf08`. Candidate: this connection reuse
change with Flux `5fa6feb`. Both use Fleet's unchanged release settings
(`opt-level=z`, fat LTO, one codegen unit), Rust 1.96.0 (ac68faa20), x86_64-pc-windows-msvc. Three alternating trials per scenario,
registered `larx` profile, 3,320 files and 434,502 unique Swifty keys. No builds or
other test runs overlapped measurement. Values below are process wall seconds.

| Scenario | Baseline min / median / max | Candidate min / median / max |
| --- | --- | --- |
| No-op | 37.439 / 37.593 / 37.693 | 2.474 / 2.524 / 2.575 |
| One-piece corruption repair | 37.929 / 38.246 / 38.628 | 3.013 / 3.045 / 3.077 |

Every no-op kept all 3,320 files with zero reused/fetched/written/deleted work.
Every repair kept 3,319 files, reused 129,117 bytes, fetched 2,387 bytes, wrote
131,504 bytes, deleted nothing and restored the full original SHA-256 and length.
Maximum observed working set was 276,160,512 bytes for baseline and 281,608,192
bytes for candidate. Pooling trades a small bounded cache increase for connection
and statement reuse. The repaired PBO and profile/settings remain unchanged.

Raw identity, binary hashes, logs and all 12 measurements are ignored under
`C:/projects/fleet-worktrees/inventory-reuse/target/real-performance/comparison`.
These measurements establish large-manifest incremental overhead; they do not
establish large-transfer throughput.

## Compact inventory comparison — 2026-09-05

Baseline `b8b441d` uses Flux `eb63338`; candidate `63058bf` uses Flux `8d381e6`.
Both clean release adapter binaries use Rust 1.96.0, x86_64-pc-windows-msvc and
Fleet's unchanged release settings (optimization level z, fat LTO, one codegen
unit). The explicitly cached release is
`a6692a65fe2b7902c9774bbe11708d29a03ac107` from the registered PCA source. No
metadata refresh selects another release. These adapter measurements include
cached input loading and recipe registration but omit CLI startup/repo refresh;
they are separate from the earlier CLI comparison.

The baseline uses a consistent SQLite snapshot of the previously byte-validated
inventory. The candidate starts empty and verifies all 85,732,470,520 target
bytes: 51.770 seconds process time, comprising 3.529 seconds of input/catalog
setup and 48.228 seconds of verification. This is separate setup, not a paired
cold-scan result. Both independently seeded inventories then serve the same
physical target. All no-op trials precede repairs; executable order alternates.
No builds, tests or other heavy work overlaps timing.

| Scenario | Baseline min / median / max seconds | Candidate min / median / max seconds | Median CPU seconds, baseline / candidate |
| --- | --- | --- | --- |
| No-op | 2.385 / 2.395 / 5.676 | 1.569 / 1.608 / 2.725 | 3.047 / 2.313 |
| One-piece repair | 2.815 / 2.819 / 2.933 | 1.706 / 1.745 / 1.759 | 3.000 / 2.234 |

All three trials per build are included. The first no-op pair is slower than the
later pairs; OS caches are uncontrolled. Median operation-only times are 1.703 /
0.980 seconds for no-op and 2.183 / 1.091 seconds for repair. Highest sampled RSS
is essentially unchanged: 274,518,016 / 274,391,040 bytes. These runs do not show
a meaningful whole-process memory reduction despite the smaller database.

Every no-op keeps 3,320 files with no byte or deletion work. Every repair keeps
3,319 files, reuses 129,117 bytes, fetches exactly 2,387 bytes, writes 131,504 bytes
and deletes nothing. All six repairs restore
`C:/pca/@ace/addons/ace_advanced_throwing.pbo` to its original length and SHA-256
`fd13b93183112da1a11e7abfde76aabcc0eb4845a8852ad833d29456c44d3ffb`.

| Inventory representation | Baseline bytes | Candidate bytes |
| --- | ---: | ---: |
| SQLite cell payload, all tables and indexes | 98,124,293 | 33,248,503 |
| Allocated database | 118,661,120 | 43,630,592 |
| Segment occurrence table and lookup index payload | 97,597,658 | 11,843,962 |
| Candidate content table plus uniqueness index | — | 20,599,219 |

The total payload reduction is 66.1%; allocated size falls 63.2%. The content
table/index is included in that total. Candidate counts are 3,320 observed files,
3,291 immutable recipes, 463,924 recipe-segment rows and 434,502 content identities.
Baseline has 463,953 per-file segment rows. File paths and physical evidence are
stored per file, profile binding per database, and recipe occurrences refer to
integer IDs. Known recipes and arbitrary observed recipes share the same tables.
Installed files keep their recipe references across goal registration; successful
terminal commit prunes unreferenced facts.

The complete candidate inventory was installed as the profile's active
`observations.sqlite` under Fleet's `fmutex` session exclusion. A subsequent
pinned sync kept all 3,320 files with zero writes, fetches or deletes. The
registered profile/settings hashes remain unchanged. No schema migration or
compatibility store was added; legacy `inventory.db` was not used.

The real repair is still a small-file workload. Bulk local assembly evidence is
in Flux's `docs/performance.md`: the matched 128 MiB/four-worker median changes
from 19.522 to 0.446 seconds, with exact shared-fetch and byte assertions. Cold
per-file observation commits and shared acquisition/publication durability remain
costs; no WAN saturation or cold-disk throughput claim follows from these runs.

Raw binaries, source/build identities, twelve trials, verified backup, full-scan
setup report, counts and all-table `dbstat` payload reports are ignored under
`C:/projects/fleet-worktrees/compact-performance/target/real-performance/compact`.
