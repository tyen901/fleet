# Materialization performance

Fleet uses four bounded r2d2/r2d2_sqlite connections for observation reads and
short observation transactions. Repeated segment and occurrence statements use
rusqlite's statement cache. A separate terminal writer owns the atomic final
transaction: its producer can read observations through the pool without holding
a pool lease across that callback. Observation writers retain the session lock
until they finish or discard their provisional facts. No full inventory or
segment cache is added to application memory.

## Real profile comparison

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
