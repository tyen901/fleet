# Swifty remote fixture

`swifty-remote.json` pins the complete `@rosche` addon from the locally validated
Swifty source repository: 1,024,514,039 bytes across nine files. Its 11,768 Swifty parts
contain 11,750 unique content keys; their median length is 1,905 bytes and 75.6%
are at most 64 KiB. This exercises the small-part sync workload,
including shared content, three large PBOs, signatures, and a key file.

The standalone Swifty repository lives at the `repo_url` in the descriptor.
Its storage prefix is `fixtures/swifty/14cd7a20d77e626f`, uploaded using the
existing `deltasync:` rclone remote.
It does not change the live source repository. Payloads remain in R2, outside Git.
The addon is a sync fixture; launching it in Arma is outside this test.

Run from the Fleet workspace:

```powershell
cargo test --release -p fleet-inventory --test swifty_remote --locked -- --ignored --nocapture
```

The test creates disposable target and metadata directories, with no access to
the installed source folder or user profiles. It downloads real metadata and bytes
over public HTTPS through Fleet's production adapters and Flux. Endpoint failure
fails the test; there is no local source substitution. The explicit ignore keeps
the 1 GB network transfer out of ordinary offline test runs.

Assertions cover:

- Cold output: exact file set, lengths and independently recorded whole-file
  SHA-256 hashes; exact physical unique-content fetch and logical output bytes.
- Warm sync: all nine files kept, zero bytes fetched or written.
- Repair: corrupt one byte inside the largest PBO, delete `meta.cpp`, and add an
  unmanaged file. Require selective remote fetching, exactly two reconstructed
  files, removal of the extra file, and exact SHA-256 output again.
- Full validation through Fleet's real Swifty profile after repair.

The test prints measured metadata, cold, warm and repair times, and saves
`target/swifty-fixture/results.json`. Timings exclude SHA-256 verification and
post-repair validation. WAN timings are observations, not deterministic CI
performance thresholds; deterministic scheduler regressions live in Flux's tests.
Do not run builds, benchmarks or another sync concurrently with a measured run.

## Artifact provenance

The original files were copied from `<source>/@rosche` to a separate staging tree.
Swifty metadata was generated using the consumed `swifty-artifacts` v0.2.1 CLI,
revision `2e5bc131`, with deterministic ticks:

```powershell
cargo install --git https://github.com/tyen901/swifty-artifacts.git --tag v0.2.1 --features cli --locked --root target/swifty-fixture/tools
target/swifty-fixture/tools/bin/swifty-artifacts.exe target/swifty-fixture/repository --name "Swifty remote fixture" --ticks 639278784000000000
```

Whole-file hashes in the descriptor come from the staged source bytes, not the
downloaded output. Its version prefix is the first 16 hexadecimal characters of
SHA-256 over the compact JSON file-record array (sorted paths, fields in
`path,length,sha256` order). Upload payloads and `mod.srf` first, publish
`repo.json` last, and check the uploaded tree with `rclone check`. Use a new
prefix and descriptor for a changed fixture rather than replacing this fixture.


## Observed endpoint result

The optimized test passed against the pinned HTTPS endpoint on 2026-10-18:

| Phase | Operation seconds | Remote bytes |
| --- | ---: | ---: |
| Cold sync | 13.277 | 1,010,555,489 |
| Warm sync | 0.026 | 0 |
| Repair | 5.287 | 145,252 |

Cold output was 1,024,514,039 bytes, with shared keys fetched only once; observed
remote throughput was 76.1 MB/s. Metadata setup took 0.542 seconds. All nine
whole-file SHA-256 checks passed after cold sync and repair, and full profile
validation passed. The complete test, including those verification reads and
intermediate metadata requests, took 24.17 seconds. These are one-run observations
with uncontrolled network and filesystem cache state, not guaranteed rates.


## Controlled old/current comparison

Both controls start from Fleet `3504fe2` and use the identical test above,
fixture descriptor, SQLite inventory, HTTP configuration and source selection.
The historical control only restores the old Flux source-lookup return shape
and accepts its obsolete progress event; it does not substitute transport or
change old Flux production code. The current control sets four acquisition slots
instead of the production default of eight.

| Control | Fleet revision | Flux revision |
| --- | --- | --- |
| Old | `f2cfd2bcdf304efae0f847896a051061d6de7e13` | `7b76b613276538c4b792f727350464f4af9c37c3` |
| Current | `9ab5688bacec57fb807a3a265dc316a5d4428ecd` | `09651421b87757d0154761697f58e1ef554b5c7e` |

Each control has a separate checkout and Cargo target directory. Build both with
`cargo test --release --locked -p fleet-inventory --test swifty_remote --no-run`.
Then invoke each generated test executable directly with
`--ignored --exact swifty_remote_cold_warm_and_repair --nocapture`, in the order
old/current/current/old/old/current. Every invocation creates an empty target and
metadata cache on the same C: volume and runs the complete cold/warm/repair test.
No build, other test, benchmark or sync runs concurrently with timed trials.

The compiler and release settings match, and the lockfiles differ only in Flux's
Git revision. Both use 128-key lookups, four local candidates, 64 KiB chunks,
64 GiB staging, the same public HTTPS endpoint and object_store defaults. The
old shared four-job limit and current separate four-file/four-acquisition limits
are different execution models. The queue changes from one chunk to eight as
part of the implementation. Therefore these results compare the combined change,
not an isolated change to one scheduler or buffer setting. WAN and cache state
remain uncontrolled; alternating order reduces, but cannot eliminate, that bias.

### Deterministic serialization guards

Flux's ordinary Windows/Linux CI runs these non-ignored behavior tests:

- `remote_output_is_constructed_before_http_eof`: the HTTP server withholds EOF
  until actual private destination-stage construction occurs. Historical Flux
  fails because it waits for acquisition before destination construction.
- `singleton_part_count_does_not_create_durable_artifacts`: singleton remote
  content must not create per-key durable checkpoints. Historical Flux fails
  with three checkpoints where zero are required.
- `saturated_network_lookups_leave_file_workers_available`: two pending network
  lookups must coexist with independent local publication. Temporarily restoring
  the old shared admission gate in current Flux makes this regression fail.

The temporary gate was removed and the complete current Flux all-target test
suite and strict Clippy passed afterward. These guards assert required progress
and persistence semantics; WAN throughput is not used as a flaky CI threshold.


### Results (2026-10-18)

All six complete endpoint trials passed exact namespace, file lengths and SHA-256,
work counters, selective repair, zero-transfer warm sync and final full profile
validation. Each cold sync constructed 1,024,514,039 bytes and fetched the same
1,010,555,489 bytes of unique content. Each repair fetched exactly 145,252 bytes.

| Median of three runs | Old Flux | Current Flux | Speedup |
| --- | ---: | ---: | ---: |
| Cold sync | 77.278 s | 11.884 s | 6.50x |
| Effective fetched throughput | 13.077 MB/s | 85.036 MB/s | 6.50x |
| Repair | 8.351 s | 5.713 s | 1.46x |
| Warm sync | 0.034 s | 0.032 s | Both effectively idle |

Effective throughput is validated unique-content bytes divided by materialization
wall time, including staging, assembly, publication and terminal persistence; it
is not a wire-rate measurement. Metadata setup and independent verification
reads are measured outside that interval. All individual samples, binary hashes,
compiler details, lockfile identities, limits and regression negative controls
are recorded in [throughput-results.json](throughput-results.json). Medians are
observations from the alternating trials, not CI thresholds or guaranteed WAN
performance. No trials were discarded.
