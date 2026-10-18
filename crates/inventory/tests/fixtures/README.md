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
