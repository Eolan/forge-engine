# Demo: `task-bench` — the job system measured

Run: `cargo run --release -p task-bench` (options `--workers N`, `--frames N`, `--pin`, `--quick`).
Machine: Ryzen 7 9800X3D (8 cores / 16 threads), Windows 11, Rust 1.98, 2026-09-24.
Research behind it: [research/task-system.md](../research/task-system.md).

## What it measures

| Benchmark | What it shows | Result (6 workers) |
|---|---|---|
| Data-parallel map, 16 M elements, grain 4096 | wide `par_chunks_mut` vs serial vs rayon | serial 12.2 ms → **2.04 ms (×5.98)**; rayon 2.25 ms (×5.43) |
| Fork-join `fib(30)` via `join`, cutoff 16 | per-job overhead of nested scopes | serial 1.33 ms → 0.26 ms (×5.1), 7 980 jobs, **~195 ns per job including work**; rayon 0.23 ms |
| 300 k detached tiny jobs from the main thread | injector throughput | **5.25 M jobs/s**, 190 ns per spawn+run |
| Single-job round trip, workers allowed to park | cold-path latency for input-driven work | spawn→start **p50 4.5 µs, p99 10.6 µs**, max 19 µs; round trip p50 5.1 µs |
| Task graph, 10 000 nodes, 100 layers, 2 deps each, ~5 µs per node | continuation-based DAG scheduling | serial 67 ms → **13.1 ms (×5.15)**; ideal at 6 workers 11.2 ms |

## The frame-pacing demonstration

600 frames of `A (par) → B (graph, High) → C (par)` with endless Low-priority background jobs
(200 µs each, the shape of chunk generation) and a stand-in real-time thread that wakes every
2.9 ms and does 200 µs of work, like an audio callback.

| Pool | frame p50 | frame p99 | frame max | audio late p99 | audio late max | misses > 1 ms |
|---|---|---|---|---|---|---|
| **client rule: 6 workers** (physical − 2) | 1.90 ms | **2.17 ms** | 2.27 ms | 0.05 ms | 0.14 ms | **0 / 407** |
| every hardware thread: 16 workers | 1.73 ms | 5.00 ms | **34.3 ms** | **26.7 ms** | 30.9 ms | **103 / 471** |

The median frame is slightly faster with 16 workers and everything else is worse: the audio
thread misses a fifth of its deadlines and the frame tail reaches 34 ms, which is the stall the
previous project measured at 15–28 ms with rayon's default pool. This is why
`PoolConfig::client()` leaves two physical cores free and why the real-time threads stay off the
pool. Measure the distribution, never the mean.

## Design notes confirmed by the numbers

- Work stealing with recursive splitting keeps steals rare on wide loops (153 steals for 16 M
  elements) and lets idle workers take the largest remaining piece.
- ~190 ns per job is dominated by the `Box` allocation and the counter atomics; an inline job
  storage (fixed-size job slots) is the next optimisation if a profile ever shows it.
- Continuations (`spawn_after`, task graphs) cost nothing while waiting: no worker blocks, so
  the 10 k-node graph runs at 5.15× on 6 workers although every node has two dependencies.
- Background jobs must stay short: with 200 µs Low jobs the High-priority frame work waits at
  most one job length per worker. A 2 ms background job would push frame p99 by 2 ms.

## Known limits / next steps

- No inline job storage yet (every job is a `Box`).
- Thread pinning is opt-in (`--pin`); on this machine it made no measurable difference while the
  machine was otherwise idle.
- The real-time stand-in runs at normal OS priority. The engine's audio thread will use a
  time-critical priority (MMCSS "Pro Audio" on Windows), which further insulates it.
- Tracy zones (`--features profiling`) are wired but unmeasured here.
