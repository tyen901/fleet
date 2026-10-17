# Flux sync throughput proposal and implementation record

## Outcome and evidence

Flux now constructs eligible remote content directly in its private destination stage while the response body is arriving. It avoids a durable artifact and second copy for keys whose remaining changed placements belong to one destination. Network admission and blocking file work have separate limits, and standard Tokio/futures APIs supply task, queue, and wakeup mechanics.

The reviewed production baseline is c522b54. Fleet pins production implementation 5ca5a01. Flux HEAD bc3aa3c contains the recorded performance results and has no production source difference from that pinned implementation. The measured candidate executable was built from 3c76054, which adds fixture changes to 5ca5a01; the measured baseline uses unchanged old production code with the common harness in d8c0d86.

All 48 matched HTTP trials passed exact verification. The actual PCA target is complete and all 85.7 GB passed content validation. The first large PCA transfer stalled near completion and was canceled; the successful retry does not explain that stall or establish uninterrupted full-run throughput.

## Why this change was necessary

The PCA manifest contains 3,320 files, 79.84 GiB, 463,953 placements, and 434,502 unique content keys. Exactly 421,089 keys occur once, leaving 13,413 shared keys. Singleton keys are 96.91% of unique keys; this is a key-count percentage, not byte coverage. The median unique key is 2,118 bytes and 81% are at most 64 KiB.

The old remote path validated and wrote each key into a separate artifact, synchronized it, renamed it, synchronized its directory, and committed its redb checkpoint. Only after all acquisitions completed did assembly reread those artifacts into the destination. Completed artifacts were retired synchronously by the coordinator. This imposed persistence and copying costs proportional to tiny-key count.

The old Limits.active_work ceiling was shared by planning, source selection, acquisition, assembly, and publication. It also restricted admitted files, while one planner and a blocked selected-run queue head could prevent otherwise independent work from starting. A one-chunk channel and one blocking dispatch per chunk added overhead.

These are observations from the code, not claims about the original author's intent. Eligible local single-destination content already had a direct assembly path. Existing held-get_opts tests already proved some independent progress; they did not prove destination construction before its own response body reached EOF.

## Fixed source decisions and staging

Planning classifies direct remote keys before reserving workspace bytes. It uses existing integer demand IDs and manifest references, without another index of cloned content identities.

| Evidence for a missing key | Treatment |
| --- | --- |
| A validated completed artifact exists | Reuse the artifact. |
| All changed placements belong to this destination and an eligible fresh local source exists | Direct local assembly. |
| All changed placements belong to this destination and no fresh local source exists | Direct remote construction. |
| Shared placements, endangered source content, or a local source unsafe for direct assembly | Durable artifact acquisition. |

Keys needed solely to preserve a source that publication would destroy remain durable dependencies. Direct classification fixes the source class; supplied sources are resolved in registration order. A missing or failed selected source is an error. There is no replacement-source fallback or conversion to an artifact path after a direct write fails.

Reserve the destination length plus the missing durable dependencies owned by that file. Direct remote inputs do not reserve another artifact copy. An all-direct three-byte output therefore fits a three-byte staging limit.

## Destination construction and publication

Extend the existing private StageFile rather than introduce a second output-file implementation. It lazily opens the private destination, writes bounded slices at fixed offsets, charges the existing reservation, and records validated direct coverage with rangemap.

Each acquisition retains a per-key validator. A key's direct coverage becomes complete only after all bytes and placements have been written and validation finishes successfully. Repeated occurrences within one destination share one acquisition and validation, with bytes written to every fixed placement.

AssemblyInput::Staged identifies already written coverage. Sealing verifies that coverage, skips its second copy, copies remaining local/artifact inputs, checks complete destination coverage, and synchronizes the output before publication. The installed destination remains unchanged while its private stage is incomplete or invalid.

Active writers share Arc<Mutex<StageFile>>. The mutex protects synchronous seek/write and coverage updates inside blocking work. It is never held over network reads, channel waits, source lookup, or progress callbacks. After all acquisitions complete, the engine recovers sole ownership for sealing and publication.

Downloaded measures actual remote bytes. Written and Patched for direct construction follow successful stage writes and include repeated placements. Fetched outcome bytes count each remote key once; written outcome bytes count completed destination coverage.

Publication remains serialized because namespace mutation and source preservation require it. Before admitting publication, the engine computes exhausted durable keys from placement counts. The blocking publication job records durable publication evidence, removes those checkpoints in one redb transaction, unlinks artifacts, and synchronizes the workspace. Coordinator completion performs bookkeeping rather than artifact filesystem/database cleanup.

## Resource API and execution policy

Limits.active_work is removed. The replacement fields are network_requests, file_workers, buffer_chunks, and the existing buffer_bytes, lookup_batch, candidates_per_key, and staging_bytes. Defaults are eight network jobs, four file workers, eight chunks, and 64 KiB per chunk.

One FuturesUnordered remains the runtime work set. Source lookup and remote acquisition use the source/network admission class. Standalone file jobs use the file class. A shared Tokio semaphore, acquired asynchronously immediately before dispatch, bounds executing blocking closures across all jobs, including remote consumers.

No file permit is held while awaiting network bytes or a queue. The coordinator admits bounded futures and keeps polling; it does not block awaiting a semaphore. The logical file-job limit bounds queued work, while the semaphore bounds actual blocking execution. Those are distinct responsibilities.

Remove the global job-count ceiling, serialized planner, arbitrary admitted-file-count gate, and queue-front conflict stop. Staging reservations and existing compact manifest/demand bookkeeping bound admitted output. Multiple plans recheck ownership at admission. A conflicting local run remains queued while independent ready runs may proceed.

Keep actual source-preservation and namespace dependencies. Add no scheduling delays, custom fairness policy, replacement admitted-files knob, or duplicate transport limiter. The existing peak_active_work counter is aggregate work, not an executing-file-worker count.

## Community mechanisms and remaining ownership

The implementation adopted three concrete reductions after inspecting upstream APIs and installed dependency sources:

- Tokio join! owns producer/consumer completion and waits for both branches, replacing a separately spawned producer and JoinHandle plumbing. [Tokio join](https://docs.rs/tokio/latest/tokio/macro.join.html)
- tokio_util::io::StreamReader plus AsyncReadExt::read replaces the manual transport-chunk cursor and splitting loop. Ordinary read returns available data; read_exact would incorrectly wait for a held tail. One bounded copy remains. [StreamReader](https://docs.rs/tokio-util/0.7.19/tokio_util/io/struct.StreamReader.html)
- Bounded Tokio mpsc and recv_many supply queue ownership, wakeups, and ready batching. The cleared batch Vec returns from blocking work for reuse. recv_many does not wait for a full batch. [Tokio receiver](https://docs.rs/tokio/latest/tokio/sync/mpsc/struct.Receiver.html#method.recv_many)

An acquisition retains at most 2 * buffer_chunks * buffer_bytes of engine chunks: a consumer batch plus producer queue, with a channel slot reserved before reading. Assembly buffers are counted separately. A transport-owned chunk is outside this bound. BufferCharge measures retained bytes; Tokio enforces queue capacity. There is no custom buffer pool or byte scheduler.

| Retained code | Concrete responsibility |
| --- | --- |
| AcquisitionDestination enum | Dispatch between durable artifact completion and validated direct placement coverage; it is not the futures Sink protocol. |
| DirectOutput offsets/cursor and standard Arc/Mutex | Represent repeated placements and protect concurrent seek/write into one file. |
| StageFile and target publication | Confined filesystem access, complete coverage, durability, and atomic installation. |
| DiskLedger and Reservation | Account for reserved, written, recovered, and retired workspace bytes across file lifetimes. |
| Demand/readiness bookkeeping | Fixed source choices, preservation dependencies, staging reservations, and namespace conflicts. |
| ContentProfile validator | Validate the consumer's content identity semantics. |

Replacing the two-variant destination enum with futures::sink::unfold or forward would not supply placement or durability rules. The inspected forward error path can drop a sink owning a running blocking task, requiring additional drain ownership. It would add machinery here. [Sink unfold](https://docs.rs/futures/latest/futures/sink/fn.unfold.html), [blocking-task cancellation](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)

SyncIoBridge or blocking_recv would park an OS worker on network input. tokio::fs also uses blocking dispatch; Tokio recommends batching ordinary file operations where appropriate. Bytes slices share their backing allocation, so slicing arbitrary transport frames does not itself establish a physical memory bound. LimitStore owns a per-store semaphore and does not cover ContentSource::lookup, so it does not replace the chosen global source admission. [SyncIoBridge](https://docs.rs/tokio-util/0.7.19/tokio_util/io/struct.SyncIoBridge.html), [Tokio filesystem guidance](https://docs.rs/tokio/latest/tokio/fs/), [Bytes](https://docs.rs/bytes/latest/bytes/struct.Bytes.html), [LimitStore](https://docs.rs/object_store/0.13.2/object_store/limit/struct.LimitStore.html)

## Review corrections and recovery semantics

The diff review found and corrected two concrete issues. Remote-only select_sources previously dispatched an empty local inspection through the file semaphore; it now skips that dispatch entirely. Direct writes previously invoked progress callbacks while holding the destination mutex; callbacks now run after unlocking and report aggregate placement bytes. Sink was renamed AcquisitionDestination to state its actual responsibility.

An unfinished direct remote stage is disposable. Cancellation or failure can require refetching already received valid prefixes on restart. There is deliberately no durable per-key resume guarantee for this path. Shared content and endangered local source content retain durable artifacts and source-free recovery.

Preserve root confinement, selected-source freshness checks, per-key validation, complete coverage, atomic publication receipts, exact namespace reconciliation, and terminal inventory confirmation. Source errors and cancellation close the bridge, cancel siblings, and drain admitted work before target ownership is released. No compatibility mode preserves the superseded singleton artifact policy.

## Deterministic regression evidence

Tests use real materialization, target files, and controlled gates. Timeouts detect failed progress; they are not performance thresholds.

| Regression | Decisive observation |
| --- | --- |
| Real HTTP prefix before EOF | Server withholds the tail until actual destination-stage prefix bytes exist; Patched alone is insufficient. Old production fails the gate. |
| Destination-only staging | Exact output succeeds with staging equal to output length. Old production fails with ResourceLimit. |
| Singleton checkpoint count | Enough space is available for either implementation; old production emits three checkpoints where direct output emits zero. |
| Remote lookup and byte delivery with file permit held | Real selection and acquisition deliver a byte before the only file permit is released. Restoring the empty local dispatch makes the test fail. |
| Corrupt tail and cancellation after actual HTTP prefix | Partial output never installs; cancellation permits a fresh successful retry. |
| Repeated placements with mixed inputs | Local A, shared B, and direct repeated C construct ABCC plus a second B destination with exact fetch/write/reuse counters. |
| Saturated async lookups | Independent local work publishes while source slots remain occupied. |
| Shared recovery and independent progress | Existing artifact recovery, cancellation/drain, source-preservation, and held-request tests remain. |

The latest 49 materialization tests and the private boundary regression passed. Flux workspace checks and strict Clippy passed. Fleet build, relevant tests, strict Clippy, and formatting checks passed.

The full two-opened-HTTP-body schedule under held file work, a single-thread validator/heartbeat schedule, and a callback-gated same-stage writer test were considered but are not delivered coverage. They are not needed to invent an explanation for the unreproduced PCA stall. Existing held-get_opts tests are not falsely described as baseline-failing tests.

## Matched real HTTP experiment

The common harness uses real loopback HTTP ranges through object_store::http, real file output, and exact post-run verification. Four files each contain 2 MiB. Small-key fixtures use 1,024 parts of 2 KiB per file; large-part download uses 16 parts of 128 KiB.

Downloads fetch/write 8 MiB. Warm repair fetches 2 MiB and writes 4 MiB. Shared repair fetches one 2 KiB key once and writes 4 MiB. All 48 timed trials passed exact bytes, namespace, transport bytes, work counters, and resource bounds.

Each configuration has three trials per revision in alternating process order, with identical compiler, lock digest, fixtures, and relevant limits. N denotes the old shared active_work=N versus separate candidate file_workers=N and network_requests=N. Queue capacity changes from one to eight; chunk size remains 64 KiB. Equal N does not imply equal aggregate concurrency. No builds, tests, other benchmarks, or live sync ran concurrently.

| Four-worker workload | Baseline median seconds | Candidate median seconds | Speedup |
| --- | ---: | ---: | ---: |
| 8 MiB small-key download | 29.214 | 0.255 | 114.50x |
| Warm repair | 9.967 | 0.326 | 30.55x |
| Shared repair | 0.668 | 0.464 | 1.44x |
| 8 MiB large-part download | 0.734 | 0.276 | 2.66x |

One-worker small-key download improved from 50.031 to 0.360 seconds, a 139.14x median speedup. Four-worker small-key process writes fell from 244,162,931 to 8,638,848 bytes; these counters include metadata/recovery writes. Shared-repair write volume remained unchanged.

Loopback is not a WAN benchmark; OS cache state is uncontrolled. Setup and exact verification are outside operation wall, while terminal durability is inside it. Speedups compare sample medians and are not CI thresholds. All individual samples, executable identities, precise baseline failures, reproduction commands, and limitations are recorded in [Flux performance results](https://github.com/tyen901/flux/blob/03cbc59ca6cd6b3a605d9368c0d56f3f43a92d99/docs/performance.md). Raw reports are under Flux target/performance/http-matched-small-keys and http-matched-large-parts.

## Actual PCA result and unresolved limitation

The initial release run transferred approximately 43.47 GB during a 603-second bulk interval, about 72 MB/s. It reached 43,474,376,431 downloaded bytes of a 43,477,250,251-byte plan, then made no observed progress for over two minutes and was canceled.

That run's INFO log did not identify the waiting boundary. Static review found no semaphore/channel/mutex dependency cycle, but cannot exclude blocked synchronous work or establish transport stalling. Established sockets alone do not establish active response waits. The cause remains unresolved.

The reviewed implementation completed a diagnostic retry in 5.456 seconds: 3,319 files kept, 69,735,152 bytes fetched, 117,847,344 bytes constructed, and 48,112,192 bytes reused. Preparation took 1.105 seconds and terminal work 0.704 seconds. The retry log records publication, terminal inventory, run_success, and Fleet Clean/UpToDate/Materialized status.

Full content validation of all 85.7 GB then passed in 52.611 seconds. A subsequent warm PCA sync completed in 3.251 seconds with zero fetch or patch work and a clean result. Evidence is in target/pca-sync, including run-2-debug.log.

This establishes completed correct materialization, fast observed bulk transfer, and a fast clean rerun. It does not establish uninterrupted full-PCA completion time or diagnose the canceled run.

## HTTP configuration decision

Fleet disables object_store's default 30-second whole-request timeout, which includes response consumption. ClientOptions in object_store 0.13.2 exposes no idle-read timeout. No timeout adapter was added to explain the unreproduced stall.

If later evidence warrants idle-read timeout, reqwest already supplies ClientBuilder::read_timeout, and object_store already implements HttpService for its reqwest 0.12 Client. A small HttpConnector factory can return HttpClient::new(configured_client.clone()); no custom request/body/range/retry implementation is necessary. Fleet's reqwest 0.13 client is a different type. Preserve object_store's HTTP/1 and disabled-compression choices when configuring a replacement client. [ClientOptions](https://docs.rs/object_store/0.13.2/object_store/client/struct.ClientOptions.html), [HttpService](https://docs.rs/object_store/0.13.2/object_store/client/trait.HttpService.html), [reqwest read timeout](https://docs.rs/reqwest/0.12.28/reqwest/struct.ClientBuilder.html#method.read_timeout)

## Implementation ownership and deleted policy

| Owner | Result |
| --- | --- |
| Flux api.rs | Independent resource limits and explicit bounded chunk capacity. |
| Flux engine.rs | Fixed direct classification, reservation/admission, stage ownership, concurrent planning, readiness, and publication retirement scheduling. |
| Flux engine/acquisition.rs | Streaming validation and destination dispatch using standard joins, bounded queues, StreamReader, and ready batches. |
| Flux target.rs and state.rs | Validated prewritten coverage, one destination seal, confined publication, and batched durable artifact retirement. |
| Flux tests and example harness | Exact regressions, shared recovery coverage, real HTTP comparison, and recorded results. |
| Fleet consumer and dependency pin | Adopt the breaking limits API and production revision; validate actual PCA output. |

Deleted policy includes mandatory remote singleton artifacts, duplicate artifact reservations, their second assembly copy and per-key checkpoints, one blocking dispatch per chunk, the shared active_work API/gate, serialized planning, the arbitrary admitted-file gate, queue-head blocking, and coordinator-side artifact retirement. No fallback path or compatibility shim retains those policies.