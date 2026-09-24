# Task system: job scheduling, frame pipelining, and ECS scheduling for Forge

Forge's first system is the task manager, and everything after it (ECS, render graph, streaming, netcode) will be shaped by the choices made here. This file collects the canonical engine job systems (Naughty Dog, Bungie, Frostbite, Unity, Unreal), the theory and data structures they rest on (Blumofe–Leiserson, Chase–Lev, Lê et al., Vyukov), the Rust crates that implement those pieces today, the frame architecture that keeps a render thread and a real-time audio thread alive next to a saturated worker pool, and the ECS crates whose executors Forge will either drive or replace. The previous in-house project's lesson ("a worker on every core is a worker too many"; "measure the frame's distribution, not its mean") is treated as a hard requirement, not an anecdote: `rayon`'s 16 default workers plus `bevy_tasks`' 16 default threads plus `tokio` on a 16-thread CPU is exactly the oversubscription that produced 15–28 ms stalls, and this document is written to make that structurally impossible in Forge.

> **State of the art in five sentences.** Every serious engine since 2014 runs almost the whole frame as a graph of small jobs on a fixed pool of one worker per physical core, with the main and render threads reduced to thin orchestration and a real-time audio thread kept off the pool entirely. Jobs synchronise through atomic dependency counters, never through blocking primitives; Naughty Dog and Our Machinery let a job wait mid-body by parking its fiber, while Bungie, Unreal's Tasks System and Sebastian Aaltonen's 2022 design forbid in-job waiting and express "after" as a continuation job, which is the only model Rust can offer safely (RFC 230 removed green threads; stackful coroutines are unsafe FFI). Under the hood the winning data structure is unchanged since 2005: a per-worker Chase–Lev deque with Lê et al.'s weak-memory fences (which `crossbeam-deque` implements) plus a global injector per priority, scheduled by randomized work stealing whose T₁/P + O(T∞) bound (Blumofe–Leiserson) makes the frame's critical path the thing to measure. Frames are pipelined so simulation N+1, render recording N and GPU execution N−1 overlap, with data crossing stages as an immutable per-frame packet, timeline semaphores bounding frames in flight, and command buffers recorded from per-thread pools into a render graph. In Rust, the honest position in 2026 is that no crate ships this whole design: `rayon` is a batch fork-join pool, `bevy_tasks` is an `async-executor` wrapper with the right pool split but the wrong defaults, `switchyard` is the closest prior art but a one-person crate, and `bevy_ecs` 0.19's multi-threaded executor is hard-wired to its own pool — so Forge should build a ~2–3k-line scheduler on `crossbeam-deque`/`async-task` and drive `bevy_ecs` systems through public `run_unsafe` + access sets rather than fork it.

---

## A. Canonical engine job systems

**Christian Gyrling. "Parallelizing the Naughty Dog Engine Using Fibers." GDC, 2015.** [talk] [foundational]
https://gdcvault.com/play/1022186/Parallelizing-the-Naughty-Dog-Engine · https://media.gdcvault.com/gdc2015/presentations/Gyrling_Christian_Parallelizing_The_Naughty.pdf · https://archive.org/details/GDC2015Gyrling_201508
The talk that made "job system + fibers" the default AAA answer: to get The Last of Us Remastered to 60 fps on PS4, Naughty Dog moved the whole frame onto a job system in which every job runs on a fiber, a worker thread is pinned per core, and jobs wait on atomic counters. A waiting job's fiber is parked and the worker picks up another fiber, so a wait costs a register save/restore rather than a thread context switch. The rest of the talk covers the "frame-centric" engine design (per-frame allocation, frame-tagged memory) and lock strategies.
*Bearing:* The counter-based dependency model is the one Forge should adopt verbatim; the fiber part is not portable to Rust (see RFC 0230 and `corosensei` below), so the same effect has to come from continuations or stackless futures.

**Barry Genova. "Multithreading the Entire Destiny Engine." GDC, 2015.** [talk] [foundational]
https://www.gdcvault.com/play/1022164/Multithreading-the-Entire-Destiny · https://www.youtube.com/watch?v=v2Q_zHG3vqg
Bungie turned nearly all of Destiny (AI, physics, rendering, networking) into a job graph with only limited thread-level preemption, and enforced programmer-facing rules about which data a job may touch so that subsystems could not interfere with data another job was using. The talk is unusually candid about the engineering-culture side: the rules, not the scheduler, are what stopped the crashes.
*Bearing:* Declared read/write access per job, checked in debug builds, is the Destiny/Unity/Aaltonen consensus and maps one-to-one onto ECS component access sets; Forge should make access declaration part of the job API rather than a convention.

**Natalya Tatarchuk. "Destiny's Multithreaded Rendering Architecture." GDC, 2015.** [talk] [foundational]
https://www.gdcvault.com/play/1021926/Destiny-s-Multithreaded-Rendering · https://advances.realtimerendering.com/destiny/gdc_2015/ (slides, 25 MB PDF) · https://archive.org/details/GDC2015Tatarchuk
The renderer side of the same engine: simulation, render-data generation and GPU submission are pipelined a frame apart; the renderer briefly locks the simulation only to copy out a minimal, immutable per-frame data set (positions, orientations, visibility results — the slides' per-frame render packet), then releases the lock and does the expensive work (batching, draw-call generation) as jobs off the copy. The abstract explicitly frames this as moving from "system-on-a-thread" (Halo: Reach) to fine-grained task and data parallelism.
*Bearing:* This is the frame shape for Forge: extract-then-release, never hold the world while recording. Copying transforms is cheaper than computing them under a lock, so the packet stores inputs, not derived matrices, where that shortens the critical section.

**Johan Andersson. "Parallel Futures of a Game Engine (v2.0)." Stockholm Game Developer Forum / DICE, 2010 (v1 2009).** [talk] [foundational]
https://www.ea.com/frostbite/news/parallel-futures-of-a-game-engine-v2-0 · https://www.slideshare.net/slideshow/parallel-futures-of-a-game-engine-v20/4345460
Frostbite's early statement of "everything is a job": levels of parallelism in the engine, the PS3 SPU lessons that forced small, self-contained jobs with explicit inputs/outputs, and the argument that reducing coupling between systems is the prerequisite for task parallelism, not an optimisation after it.
*Bearing:* The engine-structure advice still holds: if Forge's simulation crates are written as functions over explicit slices rather than methods over shared objects, the job system becomes almost mechanical.

**Yuriy O'Donnell. "FrameGraph: Extensible Rendering Architecture in Frostbite." GDC, 2017.** [talk] [still-current]
https://www.gdcvault.com/play/1024612/FrameGraph-Extensible-Rendering-Architecture-in · https://www.youtube.com/watch?v=1Sb3s7Xie4M
Render passes and their resources are declared into a graph every frame; a compile step culls unused passes, computes resource lifetimes for memory aliasing, inserts barriers and can place work on the async compute queue; execution then runs the pass callbacks. Widely credited as the first shipped AAA render graph.
*Bearing:* The render graph is the GPU-side twin of the CPU job graph and the two should share the dependency vocabulary; pass "execute" callbacks are natural jobs recording into per-thread command pools (Section F).

**Tiago Sousa, Jean Geffroy. "The Devil is in the Details: idTech 666." SIGGRAPH Advances in Real-Time Rendering, 2016.** [talk] [still-current]
https://advances.realtimerendering.com/s2016/ · https://www.slideshare.net/TiagoAlexSousa/siggraph2016-the-devil-is-in-the-details-idtech-666
Doom 2016's renderer: clustered forward shading, lighting and particle details, post-processing, and the use of asynchronous compute on consoles to overlap post-processing with graphics work. Note that this is a rendering talk; the CPU job system of id Tech 6 is not described in it, so the "Doom job system talk" from the brief does not exist in this form.
*Bearing:* Useful for the async-compute overlap pattern only; do not cite it for CPU scheduling.

**Tobias Persson. "Fiber based job system." Our Machinery blog, 2017.** [web] [still-current]
https://ruby0x1.github.io/machinery_blog_archive/post/fiber-based-job-system/index.html (archive of the defunct ourmachinery.com)
The smallest complete description of the Naughty Dog design: under 300 lines, two entry points (`run_jobs` returning an atomic counter, `wait_for_counter`), four fixed-size ring buffers (job queue, sleeping fibers, free fibers, free counters). A waiting fiber is yielded and the worker grabs a free fiber; each worker-loop iteration first checks whether any sleeping fiber's counter is satisfied and resumes it before taking new jobs. They deliberately skipped priority queues and variable stack sizes.
*Bearing:* Shows exactly what scheduler state a "wait inside a job" model needs (a sleeping-job queue polled every iteration). In Forge the same state is the counter's waiter list, and "resume" is "re-enqueue the continuation", with no stack to keep.

**Stefan Reinalter. "Job System 2.0: Lock-Free Work Stealing," parts 1–5. Molecular Musings, 2015–2016.** [web] [foundational]
https://blog.molecular-matters.com/2015/08/24/job-system-2-0-lock-free-work-stealing-part-1-basics/ · https://blog.molecular-matters.com/2015/09/25/job-system-2-0-lock-free-work-stealing-part-3-going-lock-free/ · https://blog.molecular-matters.com/2015/11/09/job-system-2-0-lock-free-work-stealing-part-4-parallel_for/ · https://blog.molecular-matters.com/2016/04/04/job-system-2-0-lock-free-work-stealing-part-5-dependencies/
A worked implementation: per-thread ring-buffer job allocator (no `new`/`delete` in the frame), cache-line-sized job structs with inline payload, a lock-free work-stealing deque with the memory-ordering traps spelled out, `parallel_for` by recursive splitting, then parent/child counting and continuation dependencies.
*Bearing:* Forge's design is this series translated: 128-byte `Job` structs from a per-frame bump arena, `crossbeam-deque` instead of the hand-rolled deque, and part 5's continuations instead of waiting.

**Sebastian Aaltonen. Thread: a lock-free job system API draft (`LaunchJob`). X/Twitter, April 2022.** [web] [recent]
https://threadreaderapp.com/thread/1517842249803051009.html
Jobs declare `ReadOnly`/`ReadWrite` access to 64-bit resource handles; read-write is exclusive per resource, so the scheduler derives ordering from declared access rather than explicit edges. There are no mutexes or semaphores in user code and a job never stalls: if it needs different access it spawns a continuation job with a new access set. Execution is lock-free work stealing with one thread per core and lazy binary splitting for parallel loops; jobs and dependency records live in a ring-buffer bump allocator with zero allocations. (The thread does not name HypeHype; it predates his public HypeHype material.)
*Bearing:* The single best fit for a Rust engine: resource handles are component tables or ECS access sets, "never stalls" is the borrow checker's world-view, and continuations replace fibers. Forge's API sketch (Recommendation) is a Rust rendering of this thread.

**Unity Technologies. "Job system overview" / "C# Job System"; "Burst User Guide."** [web] [still-current]
http://docs.unity3d.com/Manual/job-system-overview.html · https://docs.unity3d.com/2021.2/Documentation/Manual/JobSystemOverview.html · https://docs.unity3d.com/Packages/com.unity.burst@1.8/manual/index.html
Unity keeps "only enough threads to match the capacity of the CPU cores", passes each job a copy of blittable data or a `NativeContainer`, and wraps containers in a safety system that checks the job dependency graph and indexing at runtime; `JobHandle` expresses dependencies and `Complete()` returns ownership to the main thread; static data bypasses the safety system entirely. Burst compiles the HPC# subset through LLVM for the job bodies.
*Bearing:* Rust gets the safety system mostly at compile time (`Send`/`Sync`, borrows, `std::thread::scope`); the only runtime check Forge needs is aliasing of ECS table access between concurrently scheduled jobs, which is what access sets give.

**Epic Games. "Tasks Systems in Unreal Engine"; "Parallel Rendering Overview for Unreal Engine." UE5 documentation.** [web] [still-current]
https://dev.epicgames.com/documentation/unreal-engine/tasks-systems-in-unreal-engine · https://dev.epicgames.com/documentation/en-us/unreal-engine/parallel-rendering-overview-for-unreal-engine
UE5's Tasks System is a DAG job manager sharing the TaskGraph backend: launch with a callable and prerequisites, retrieve results, nest tasks (a parent completes only when nested tasks complete), chain "pipes", signal with task events. The rendering doc documents the three-thread pipeline: game thread on frame N+1 while the render thread processes N, and an RHI thread that can lag further, with parallel command-list recording on both frontend and backend.
*Bearing:* Even UE keeps dedicated render/RHI threads; Aaltonen calls that a dual-core-era legacy, and Forge should keep only a thin submit thread while recording happens on the pool. The nested-task-parent semantics are worth copying for `frame_scope`.

**UXL Foundation / Intel. "task_arena." oneAPI / oneTBB specification 1.3.** [web] [still-current]
https://oneapi-spec.uxlfoundation.org/specifications/oneapi/v1.3-rev-1/elements/onetbb/source/task_scheduler/task_arena/task_arena_cls
An arena bounds concurrency (`max_concurrency`, `reserved_for_masters`); tasks spawned in one arena are never executed by another; `this_task_arena::isolate` restricts a thread that is waiting to running only tasks from the isolating scope. The spec warns that an arena with no worker slots gives no execution guarantee for enqueued tasks.
*Bearing:* Two ideas to steal: a capped background arena for procgen/streaming so it can never take more than N cores, and isolation as the reason in-job blocking waits are dangerous (a waiting thread that helps may run an unrelated job and deadlock on a lock it holds).

**Tsung-Wei Huang, Dian-Lun Lin, Chun-Xun Lin, Yibo Lin. "Taskflow: A Lightweight Parallel and Heterogeneous Task Graph Computing System." IEEE TPDS 33(6), 2022.** [paper] [recent]
https://tsung-wei-huang.github.io/papers/tpds21-taskflow.pdf · https://dl.acm.org/doi/10.1109/TPDS.2021.3104255 · https://github.com/taskflow/taskflow · https://github.com/taskflow/work-stealing-queue
A task-graph programming model with static, dynamic (subflow), conditional and composable tasks over a work-stealing executor, including CPU–GPU heterogeneous graphs; the group's standalone `work-stealing-queue` repo is a compact Lê et al. implementation. The paper's benchmarks against TBB and OpenMP are a good calibration for DAG scheduling overhead per task.
*Bearing:* Conditional tasks show how to express loops and retries inside a DAG without any waiting; Forge's 10k-job DAG benchmark should be comparable to Taskflow's numbers as a sanity check.

**Rust project. RFC 0230 "Remove runtime" (green threading removal). 2014.** [web] [foundational]
https://github.com/rust-lang/rfcs/blob/master/text/0230-remove-runtime.md
Rust dropped its runtime and green threads: `std::io` was tied to native threads, and the RFC argued the "least common denominator" API between green and native threading hurt both. There has been no first-class M:N or fiber facility in the language since; `async`/`await` (stackless state machines) is the sanctioned replacement.
*Bearing:* A fiber-based job system in Rust means hand-written assembly context switches under which thread-locals, `Send` reasoning, unwinding and debuggers are all on their own. This is why Forge chooses continuations/futures.

**Amanieu d'Antras. `corosensei` — stackful coroutines for Rust.** [code] [recent]
https://github.com/Amanieu/corosensei · https://docs.rs/corosensei (0.3.x)
The best available stackful-coroutine crate: assembly context switching per architecture, guard-paged `DefaultStack`, suspend from any call-stack depth, values passed both ways, `no_std`-capable. It is safe in the narrow sense that the API prevents obvious misuse, but nothing makes a suspended coroutine's borrows or thread-local references valid on a different worker thread, and panics/backtraces across stacks need care.
*Bearing:* If Forge ever needs a stackful escape hatch (scripting, long-running procedural generators with deep recursion), this is the crate; it must not be the core job model.

## B. Theory and data structures

**Robert D. Blumofe, Charles E. Leiserson. "Scheduling Multithreaded Computations by Work Stealing." Journal of the ACM 46(5), 1999.** [paper] [foundational]
https://dl.acm.org/doi/pdf/10.1145/324133.324234 · https://www.csd.uwo.ca/~mmorenom/CS433-CS9624/Resources/Scheduling_multithreaded_computations_by_work_stealing.pdf
The Cilk result: randomized work stealing executes a fully strict computation in expected time T₁/P + O(T∞) with space at most S₁·P and communication bounded by O(P·T∞·(1+n_d)·S_max), i.e. near-linear speedup whenever the critical path T∞ is short relative to T₁/P.
*Bearing:* For a frame, T∞ is the longest chain of dependent jobs; when speedup stalls, shorten that chain (split serial phases) before adding workers. The 10k-job DAG benchmark must report T₁, T∞ and T_P together.

**David Chase, Yossi Lev. "Dynamic Circular Work-Stealing Deque." SPAA, 2005.** [paper] [foundational]
https://dl.acm.org/doi/10.1145/1073970.1073974 · https://www.dre.vanderbilt.edu/~schmidt/PDF/work-stealing-dequeue.pdf
Fixes the fixed-array overflow of the Arora–Blumofe–Plaxton deque with a growable circular array: the owner pushes/pops at the bottom without CAS in the common case; thieves steal from the top with a single CAS; growth copies without blocking thieves.
*Bearing:* The per-worker queue. Owner-side LIFO gives cache locality for freshly spawned children; thief-side FIFO steals the oldest (largest) work, which is what makes recursive `parallel_for` splitting cheap.

**Nhat Minh Lê, Antoniu Pop, Albert Cohen, Francesco Zappa Nardelli. "Correct and Efficient Work-Stealing for Weak Memory Models." PPoPP, 2013.** [paper] [foundational]
https://dl.acm.org/doi/10.1145/2442516.2442524 · https://fzn.fr/readings/ppopp13.pdf
The first proof of an optimised Chase–Lev deque on ARM/POWER memory models, plus a portable C11 version with the minimal set of fences and measurements of what each barrier costs on x86, ARM and POWER.
*Bearing:* `crossbeam-deque` is this paper; a hand-rolled deque in Forge would be re-deriving these fences without the proof. Don't.

**Dmitry Vyukov. "Bounded MPMC queue." 1024cores.** [web] [foundational]
https://sites.google.com/site/1024cores/home/lock-free-algorithms/queues/bounded-mpmc-queue
Array-based multi-producer/multi-consumer queue with a per-slot sequence number: one CAS per enqueue/dequeue, no dynamic allocation, causal FIFO, fails (rather than blocks) on overflow, producers and consumers touch disjoint cache lines while the queue is non-empty. Ported into Netty, Akka, and most game job systems' injection queues.
*Bearing:* Forge's global injectors (one per priority) are bounded Vyukov rings; overflow is a bug signal (a system spawned more jobs than the frame budget), not a case to grow around.

**crossbeam-rs. `crossbeam-deque`, `crossbeam-channel`, `crossbeam-utils::CachePadded`.** [code] [still-current]
https://docs.rs/crossbeam-deque · https://docs.rs/crossbeam-channel (0.5.17) · https://docs.rs/crossbeam-utils/latest/crossbeam_utils/struct.CachePadded.html
`crossbeam-deque` is documented as a hybrid of Chase–Lev and the Lê et al. improvements: `Worker` (FIFO or LIFO, single owner), cloneable `Stealer`, and a shared `Injector` FIFO as the entry point for new tasks. `crossbeam-channel` gives bounded/unbounded/zero-capacity channels and `select!`. `CachePadded` aligns to 128 bytes on x86-64/aarch64 (to defeat adjacent-line prefetch) and is `#[repr(C)]` so the padded value keeps its address.
*Bearing:* These three crates are the entire lock-free substrate Forge needs; the scheduler on top is policy (priorities, counters, arenas), not data structures.

**Amanieu d'Antras. `parking_lot`.** [code] [still-current]
https://docs.rs/parking_lot (0.12.5, Aug 2026)
Smaller, faster `Mutex`/`RwLock`/`Condvar`/`Once` than `std`, with fair variants, `ReentrantMutex`, mapped guards and an experimental deadlock detector.
*Bearing:* Used only where Forge genuinely needs a lock: the worker sleep/wake condvar, the resource registry, tooling. Never inside a job body on the frame path; the deadlock detector is worth enabling in debug.

**Rust std. `std::thread::scope`. Stabilised in Rust 1.63.** [web] [still-current]
https://doc.rust-lang.org/std/thread/fn.scope.html
Scoped threads may borrow non-`'static` data; every thread spawned in the scope is joined before `scope` returns, and a panic in an auto-joined thread propagates.
*Bearing:* The type-level model for Forge's `frame_scope`: jobs spawned in a frame borrow frame data (`&'scope`) without `Arc`, and the scope's end is the one legitimate blocking point (where the main thread helps run jobs instead of sleeping).

**Microsoft. "SetThreadAffinityMask"; "SetThreadPriority"; "Multimedia Class Scheduler Service." Win32 documentation.** [web] [still-current]
https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadaffinitymask · https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadpriority · https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service
Affinity masks are per logical processor and the docs warn that "in most cases, it is better to let the system select an available processor"; `SetThreadPriority` gives ±2 levels plus `TIME_CRITICAL` (base 15), with explicit warnings that base priority above 11 interferes with the OS and that background IO threads should use `THREAD_MODE_BACKGROUND_BEGIN` rather than a low CPU priority. MMCSS registers a thread for a task ("Pro Audio" runs at 23–26, "Games" exists as a class) via `AvSetMmThreadCharacteristics`, and reserves `SystemResponsiveness` (default 20%) of CPU for everything else.
*Bearing:* The audio thread goes on MMCSS "Pro Audio", not on a pinned core and not on `TIME_CRITICAL`; workers stay `NORMAL`; streaming/IO threads use background mode; pinning of workers is a measured option, not a default assumption.

**Linux man-pages. `sched_setaffinity(2)`.** [web] [still-current]
https://man7.org/linux/man-pages/man2/sched_setaffinity.2.html
`sched_setaffinity(pid, cpusetsize, mask)` (use `pthread_setaffinity_np` from pthreads); `isolcpus` and cgroup `cpuset` restrictions are applied silently by the kernel on top of the requested mask.
*Bearing:* Server deployments in containers may have fewer CPUs than `available_parallelism` suggests; Forge's worker count must be derived from the effective affinity mask, not from the hardware.

**Elzair. `core_affinity` crate.** [code] [still-current]
https://docs.rs/core_affinity · https://github.com/Elzair/core_affinity_rs
`get_core_ids()` + `set_for_current(id)` on Windows, Linux and macOS; IDs are logical processors.
*Bearing:* Adequate for pinning; Forge must derive the SMT sibling mapping itself (on Windows, logical 2k and 2k+1 normally share a core) and pin a worker to a sibling pair, not to a single logical CPU.

**Wikipedia. "Zen 5" — Ryzen 7 9800X3D entry.** [web] [recent]
https://en.wikipedia.org/wiki/Zen_5
9800X3D: 8 cores / 16 threads, 96 MB L3 (64 MB of it 3D V-Cache), a single CCD plus one IOD, 4.7/5.2 GHz, 120 W, launched November 2024; Zen 5 cores carry 1 MB L2 and 48 KB L1D each.
*Bearing:* Single CCD means no cross-CCD L3 penalty and no NUMA on the dev box, so the only topology questions are SMT (8 physical, not 16) and keeping per-job working sets near the 1 MB L2. The formula below must still detect dual-CCD parts (9950X3D) and prefer the V-Cache CCD for frame workers.

## C. Rust job and async ecosystem

**rayon-rs. `rayon` / `rayon-core` 1.12: `ThreadPoolBuilder`, `in_place_scope`, `yield_now`, the sleep protocol, issue #642.** [code] [still-current]
https://docs.rs/rayon/latest/rayon/struct.ThreadPool.html · https://github.com/rayon-rs/rayon/blob/main/FAQ.md · https://github.com/rayon-rs/rayon/blob/main/rayon-core/src/sleep/README.md · https://github.com/rayon-rs/rayon/issues/642
Rayon defaults to one worker per logical CPU (16 here; `RAYON_NUM_THREADS` or `ThreadPoolBuilder::num_threads` override), and is a fork-join pool: `join`, `scope`, parallel iterators, with `in_place_scope` to spawn from a non-pool thread and `yield_now`/`yield_local` to let a foreign thread help. The sleep README documents that idle workers search other queues and the injector for "a certain number of rounds" before getting sleepy, and are woken through a jobs-event counter; issue #642 records ~30% CPU on a 4-core box from that searching when little work is posted. There are no priorities and no dependency counters beyond scope/join nesting.
*Bearing:* Rayon is a fine batch tool and a poor frame scheduler: the prior stalls came from 16 searching workers competing with render and audio. If Forge uses it at all (offline tools, asset cooking), it gets its own small pool and never runs inside the frame loop.

**Bevy Engine. `bevy_tasks` 0.19.1: `TaskPool`, `TaskPoolBuilder`, `ComputeTaskPool` / `AsyncComputeTaskPool` / `IoTaskPool`, `TaskPoolOptions`.** [code] [recent]
https://docs.rs/bevy/latest/bevy/tasks/index.html · https://docs.rs/bevy_tasks/latest/bevy_tasks/struct.TaskPoolBuilder.html · https://docs.rs/bevy/latest/bevy/app/struct.TaskPoolOptions.html · https://github.com/bevyengine/bevy/blob/main/crates/bevy_app/src/task_pool_plugin.rs
An `async-executor`-based pool with `scope()` (calling thread participates), and three global pools: compute (must finish this frame), async compute (may not), IO (mostly parked). `TaskPoolBuilder` exposes `num_threads` (default: logical cores), `stack_size`, `thread_name`, `on_thread_spawn`/`on_thread_destroy`. `TaskPoolOptions::default()` gives IO 25% (1..4 threads), async compute 25% (1..4), and compute `percent: 1.0` of whatever remains — on 16 logical threads that is 4 + 4 + 8 = 16 threads.
*Bearing:* The three-way split is the right taxonomy and `on_thread_spawn` is the hook for pinning/naming/MMCSS, but the defaults count logical CPUs and sum to the whole machine; combined with rayon's 16 this is the documented cause of the old stalls. Forge sets every count explicitly.

**Connor Fitzgerald. `switchyard`.** [code] [still-current]
https://github.com/BVE-Reborn/switchyard · https://docs.rs/switchyard (0.3.1, Aug 2026)
"Real-time compute-focused async executor with job pools, thread-local data, and priorities": tasks are spawned into named pools, run high-to-low priority, and `spawn_local` gives access to per-thread `!Send` data; built for rend3, which later removed its dependency on it.
*Bearing:* The closest existing Rust prior art for "async tasks as frame jobs with priorities" and worth reading end-to-end; as a dependency it is a one-maintainer crate whose main consumer left, so Forge should learn from it, not build on it.

**smol-rs. `async-executor` 1.14 (and `async-task`); Joshua Barretto, `pollster` 1.0.** [code] [still-current]
https://docs.rs/async-executor · https://docs.rs/pollster
`async-executor` describes itself as reference executors "that trade performance for functionality" over `async-task` and `concurrent-queue`, and recommends custom executors for specialised needs; `pollster` is a ~100-line, dependency-free `block_on` that parks instead of spinning.
*Bearing:* `async-task` is the reusable piece: it turns a future into a raw, allocation-once task with a caller-supplied `schedule` closure, so Forge can push woken tasks onto its own crossbeam deques and keep full control of policy. `pollster` is enough for the few places (asset loads in tools) where a sync wait on a future is acceptable.

**Tokio contributors. `tokio` 1.53; DataDog, `glommio`.** [code] [still-current]
https://docs.rs/tokio · https://github.com/DataDog/glommio · https://www.datadoghq.com/blog/engineering/introducing-glommio/
Tokio's docs are explicit: the multi-thread scheduler is a work-stealing IO runtime, blocking work goes through `spawn_blocking`, and CPU-bound work "should use a separate thread pool". Glommio is a cooperative thread-per-core runtime on `io_uring` (Linux ≥ 5.8, 512 KiB locked memory) with no helper threads.
*Bearing:* Tokio runs Forge netcode on one or two worker threads and nothing else; its blocking pool must be capped. Glommio is a possible Linux-server IO option later, not a client one.

**Rust async book. "Under the hood: the Future trait."** [web] [still-current]
https://rust-lang.github.io/async-book/02_execution/02_future.html
`poll` advances a future as far as it can and returns `Ready` or `Pending`; the `Waker` (via `Context`) lets the executor identify which task to re-schedule; combinators compose into allocation-free state machines without per-task stacks.
*Bearing:* This is the Rust-native replacement for fibers: a job that must wait for a counter is an `async` job whose `await` on the counter registers a waker that re-enqueues the continuation. The stack is gone; the state machine is the "fiber".

## D. Frame architecture and measurement

**Alen Ladavac. "The Elusive Frame Timing: A Case Study for Smoothness Over Speed." GDC (Advanced Graphics Techniques Tutorial), 2018.** [talk] [still-current]
https://www.gdcvault.com/play/1025407/The-Elusive-Frame-Timing-A
Micro-stutter comes from queueing between CPU submission and GPU presentation rather than from average throughput; Ladavac argues for measuring and pacing to consistency, with prototypes that trade peak frame rate for stable frame-to-frame delta.
*Bearing:* The formal backing for "measure the distribution, not the mean": Forge's acceptance metrics are p99/p50 ratios and max-over-run, and frames in flight are capped at two.

**Bartosz Taudul (wolfpld). Tracy Profiler; Simonas Kazlauskas (nagisa). `tracy-client` / `tracing-tracy`.** [code] [still-current]
https://github.com/wolfpld/tracy · https://docs.rs/tracy-client · https://github.com/nagisa/rust_tracy_client
A nanosecond-resolution hybrid frame/sampling remote profiler with GPU zones for Vulkan among others, frame marks, and fiber/coroutine instrumentation (`TRACY_FIBERS`, exposed as the `fibers` feature of `tracy-client`); the Rust bindings are third-party but the ones Bevy and most Rust engines use.
*Bearing:* Every Forge job gets a Tracy zone named after the job, async jobs use the fiber API so a continuation chain reads as one lane across workers, and frame marks feed Tracy's per-zone histograms. This is how the "distribution" is looked at during development.

**Embark Studios. `puffin`.** [code] [still-current]
https://github.com/EmbarkStudios/puffin
Scope-macro instrumentation profiler at ~1 ns disabled / 50–200 ns enabled, with `puffin_egui` for an in-game flame graph and `puffin_http` for remote viewing.
*Bearing:* The always-on in-game overlay (frame-time histogram, per-system bars) for the demo build; Tracy remains the deep-dive tool.

## E. ECS scheduling interplay

**Bevy Engine. Bevy 0.19 (19 June 2026); `bevy_ecs` 0.19.1 schedule module and `MultiThreadedExecutor` source.** [code] [recent]
https://bevy.org/news/bevy-0-19/ · https://docs.rs/bevy_ecs/latest/bevy_ecs/schedule/index.html · https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/schedule/executor/multi_threaded.rs
0.19's headline ECS change is resources stored as components on singleton entities (so hooks, observers and relationships apply to them), plus self-referential relationships, observer run conditions and the render graph re-expressed as ECS schedules. The multi-threaded executor calls `ComputeTaskPool::get_or_init(TaskPool::default).scope_with_executor()`, marks a system ready when its remaining-dependency count hits zero and its `FixedBitSet` of conflicting systems has none running, runs exclusive systems on the scope's main thread via `spawn_on_scope`, and applies deferred commands after each exclusive system and at the end; `ConflictingSystems` and ambiguity warnings are public. The executor is selected through an `ExecutorKind` enum, so a custom executor cannot be injected without forking.
*Bearing:* Forge cannot hand its job system to Bevy's executor, but it does not need to: `System::run_unsafe`, `component_access_set` and `UnsafeWorldCell` are public, which is all the ~600-line executor uses. Forge writes its own executor over its job graph (see Recommendation) and initialises `ComputeTaskPool` itself with a pinned, correctly-sized pool so `par_iter` remains available as a fallback.

**Sander Mertens. Flecs manual, "Systems" (multithreading); Indra-db, `flecs_ecs` Rust bindings.** [code] [recent]
https://www.flecs.dev/flecs/Systems.html · https://github.com/SanderMertens/flecs · https://github.com/Indra-db/Flecs-Rust
Systems flagged `multi_threaded` are run on all worker threads with matched entities sliced into contiguous ranges (1000 entities / 4 threads → 250 each), and the same entity stays on the same thread until the next sync point; `ecs_set_threads` creates workers, while `ecs_set_task_threads` plus `os_api` task callbacks let an external job system run the slices instead. Multithreaded systems are single-phase and cannot use custom scheduling. `flecs_ecs` (0.1.1, alpha) binds Flecs 4.0.1 with soundness work still open; multithreading is managed by Flecs, not by sharing the world across Rust threads.
*Bearing:* Flecs is the one mature ECS explicitly designed to be driven by an external task system, and its "same entity, same thread until sync" rule is a determinism aid worth copying. The Rust binding's alpha status and the C FFI surface rule it out for Forge's shared sim crates today.

**Benjamin Saunders (Ralith). `hecs`.** [code] [still-current]
https://github.com/Ralith/hecs · https://docs.rs/hecs
A minimalist archetype ECS that is "a library, not a framework": no system abstraction, queries from ordinary code, per-archetype dense component arrays exposed through `World::archetypes()`.
*Bearing:* The natural partner for a "job graph over tables" design because there is no scheduler to fight; the price is no change detection, relationships, hooks or observers, which Forge's tooling will want.

**Amethyst Foundation, `legion` (archived 2022); Tudor Lechintan, `sparsey`; Ryan Johnson (rj00a), `evenio`.** [code] [legion: dead; sparsey/evenio: still-current]
https://github.com/amethyst/legion · https://github.com/LechintanTudor/sparsey · https://github.com/rj00a/evenio
Legion (Unity-ECS-inspired archetypes with a specs-like API) lives under the Amethyst organisation archived in April 2022 and should be treated as unmaintained. Sparsey is a sparse-set ECS with "grouped storages" that pack related components densely for slice access; it is small and single-maintainer. Evenio is an archetype ECS where systems are event handlers and structural changes are themselves events, with targeted events, generational indices, Rayon-parallel queries and `no_std`; it was written for the Valence Minecraft server rewrite.
*Bearing:* None is a fit for Forge's core, but evenio's event-driven control flow is a good model for server-side gameplay logic layered on top of whatever ECS is chosen.

**Richard Fabian. "Data-Oriented Design." Self-published, 2018.** [book] [foundational]
https://www.dataorienteddesign.com/dodbook/
The book-length case for relational, table-shaped game state: component-based objects as tables, existential processing, and optimisation chapters on tables, cache and SIMD.
*Bearing:* The textbook for the "no ECS framework, just SoA tables plus a job graph" option in the comparison; Forge's `bevy_ecs` archetype tables are already this shape per component, which is why that option is deferred rather than rejected.

**Mike Acton. "Data-Oriented Design and C++." CppCon, 2014.** [talk] [foundational]
https://www.youtube.com/watch?v=rX0ItVEVjHc
Insomniac's engine-director statement of DOD: the purpose of code is to transform data, "where there is one there are many", and the cost model is memory latency, not instruction count.
*Bearing:* The framing that makes job granularity obvious: a job is a transform over a contiguous range of one table, sized so its inputs fit L2.

## F. GPU side: queues, command recording, render graph

**James Jones (NVIDIA, Vulkan WG). "Vulkan Timeline Semaphores." Khronos blog, 15 January 2020.** [web] [still-current]
https://www.khronos.org/blog/vulkan-timeline-semaphores
Timeline semaphores (core in Vulkan 1.2) are monotonically increasing 64-bit counters that both device and host can wait on and signal, allow wait-before-signal submission order, need no reset, and support many waits per signal — superseding binary semaphores and fences for most uses.
*Bearing:* One timeline per queue with value = frame index; the CPU waits for value N−2 before reusing frame N's command pools and staging memory, and job dependencies that cross into the GPU are expressed as timeline values, not fences.

**Khronos Group. Vulkan Guide, "Threading"; Vulkan Samples, "Multi-threaded recording with multiple render passes."** [web] [code] [still-current]
https://docs.vulkan.org/guide/latest/threading.html · https://docs.vulkan.org/samples/latest/samples/performance/multithreading_render_passes/README.html
Command pools and descriptor pools are externally synchronised, so the guide's pattern is one pool per recording thread (per frame in flight) with a light submission thread; the sample compares single-threaded recording, multi-threaded primary command buffers and multi-threaded secondary command buffers across a shadow pass and a main pass, reporting frame time and CPU utilisation.
*Bearing:* Forge's render-graph pass jobs record into `pools[worker][frame_in_flight]`; whether to use secondaries or per-pass primaries is decided by the sample's measurement, not by convention.

**Stephan Hodes (AMD). "Leveraging asynchronous queues for concurrent execution." GPUOpen, 1 December 2016.** [web] [still-current]
https://gpuopen.com/learn/concurrent-execution-asynchronous-queues/
Pair passes with complementary bottlenecks on the graphics and compute queues (ALU/LDS-heavy compute next to depth-only or bandwidth-bound graphics), and account for cross-queue synchronisation cost and contention for caches, bandwidth and registers when workloads are badly matched.
*Bearing:* The render graph's async-compute placement heuristic; on a huge procedural world the obvious candidates are terrain/vegetation compute and GPU culling overlapping the shadow pass.

**Hans-Kristian Arntzen. "Render graphs and Vulkan — a deep dive." 2017.** [web] [still-current]
https://themaister.net/blog/2017/08/15/render-graphs-and-vulkan-a-deep-dive/
A complete implementation walk-through of a Frostbite-style render graph in Granite: pass declaration, dependency and lifetime analysis, barrier generation, transient memory aliasing, and async-compute scheduling.
*Bearing:* The implementation roadmap for Forge's render graph once the CPU job system exists; its compile step is itself a job.

---

## Comparison table: Rust job and ECS options for Forge (September 2026)

| Option | Role | Thread-count control | Priorities | Dependencies | Waiting model | Frame-latency fitness | Determinism aids | Maintenance | Verdict |
|---|---|---|---|---|---|---|---|---|---|
| `rayon` 1.12 | fork-join data parallelism | explicit, but defaults to all logical CPUs | none | `join`/`scope` nesting only | blocking `join`; workers spin-search before sleeping | poor (searching workers, no priorities) | ordered `collect` only | large, stable | tools/offline only |
| `bevy_tasks` 0.19 | async pools (compute / async compute / IO) | explicit via builder; defaults sum to all logical CPUs | per-pool only | futures | `scope` (caller helps) | fair, if counts are set | none | Bevy-backed | keep for `bevy_ecs` compatibility, sized by Forge |
| `switchyard` 0.3 | prioritised async executor | explicit | yes | futures | futures | good on paper | none | single maintainer, main user gone | study, don't depend |
| `async-executor`/`async-task` | generic executor / raw task | n/a (you own threads) | none (build your own) | wakers | futures | as good as your scheduler | none | smol-rs, active | `async-task` as building block |
| `tokio` 1.53 | IO runtime | explicit | none | futures | futures + `spawn_blocking` | not for compute | none | very large | netcode only, 1–2 threads |
| **Forge job system** (crossbeam-deque + Vyukov rings + counters + `async-task`) | frame scheduler | explicit, physical-core based, pinned | 3 queues + background arena | counters + continuations + declared access | never inside jobs; main thread helps at frame end | designed for it | fixed-order merge, per-item seeds, access sets | ours (~2–3k lines) | **build** |
| `bevy_ecs` 0.19 | ECS + schedule | pool hard-wired (`ComputeTaskPool`) | none | system ordering + access conflicts | executor-internal | good with correct pool size | change detection, deterministic system order, `ConflictingSystems` | large, fast-moving (breaking every ~4 months) | **adopt storage/queries; replace executor** |
| `flecs_ecs` 0.1 (Flecs 4.0.1) | ECS + pipeline | `ecs_set_threads` / external task API | none | pipeline phases | slices per thread, sync points | good | same-entity-same-thread rule | alpha binding, C FFI | not for shared sim crates yet |
| `hecs` | minimal archetype ECS | n/a | n/a | none | n/a | n/a | none built in | stable, small | fits "tables + jobs"; lacks tooling features |
| `evenio` | event-driven archetype ECS | Rayon for queries | none | event flow | n/a | untested at engine scale | none | one maintainer | pattern source for server logic |
| `sparsey` | sparse-set ECS with groups | n/a | n/a | none | n/a | n/a | none | one maintainer | niche |
| `legion` | archetype ECS | rayon-based | none | schedule | n/a | n/a | none | archived (Amethyst, 2022) | no |
| Custom SoA tables + job graph (Fabian / Our Machinery style) | storage + scheduling | full | full | full | full | best possible | full | ours (large) | defer; revisit if `bevy_ecs` storage limits show in profiles |

---

## Recommendation for Forge

**Build the job system; do not configure one.** Nothing on crates.io combines physical-core-aware pools, priorities, dependency counters with continuations, declared access, and a main thread that helps rather than sleeps. The pieces exist (`crossbeam-deque`, `crossbeam-utils::CachePadded`, a Vyukov ring, `async-task`, `core_affinity`, `parking_lot` for the one condvar) and the policy layer is 2–3k lines. Everything below is a concrete first cut to be revised by the benchmarks at the end.

**Threads on the 9800X3D (8C/16T, one CCD).** Let `P` = physical cores from the *effective* affinity mask (not `available_parallelism`, which returns 16). Reserved threads: main (window, input, orchestration; joins as a worker while waiting for the frame to finish), render-submit (1, `ABOVE_NORMAL`, unpinned), audio (1, MMCSS "Pro Audio", unpinned, never touches the pool), net IO (tokio, `worker_threads(1)`, blocking pool capped at 2), asset IO (1, `THREAD_MODE_BACKGROUND_BEGIN`). Frame workers `W = clamp(P − 2, 2, P)` = **6**, pinned to physical cores 0–5 as sibling-pair masks (`0b11 << 2k`), so the OS can still move a worker to the sibling but never onto cores 6–7, which are left for main/render/audio/driver threads. Background arena `B = 2` threads at `BELOW_NORMAL`, unpinned, floating into SMT headroom for procgen and streaming. Steady-state runnable threads: 6 + 2 + main + submit + audio + net ≈ 12 ≤ 16, and never more compute workers than physical cores. On a dual-CCD X3D part, workers pin to the V-Cache CCD. Server (8 threads, topology unknown until deployed): `W = max(2, P − 1)`, no pinning by default (cpuset restrictions are silent), one tokio thread.

**Queues and priorities.** Per worker: one `crossbeam-deque::Worker` (LIFO pop for locality, FIFO steal). Global: three bounded Vyukov rings (High / Normal / Low). Search order per worker loop: local deque → High ring → steal from a random victim (up to 3 tries) → Normal ring → Low ring → yield once → park on a condvar (no spin-searching beyond one yield; the prior project's stalls are the reason). High is the frame's critical path (physics islands, visibility, transform propagation), Normal is everything else in the frame, Low is frame-optional work that the background arena may also pull. Jobs are `CachePadded` 128-byte structs from a per-frame bump arena (reset only once the previous frame's counter reaches zero) with 64 bytes of inline closure storage; larger closures are boxed once. Debug builds assert that a frame-pool job body exceeds 2 ms only if it is marked `long`, which forces it into the background arena.

**Dependencies: counters and continuations, futures as sugar.** A `Counter` is an atomic `u32` plus an intrusive lock-free stack of waiting continuation jobs. A job completing decrements its counters; a counter reaching zero pushes its waiters onto the appropriate ring. **No job ever blocks** — no `wait` inside a job body, no mutex on the frame path — exactly Aaltonen's and Bungie's rule. The only blocking point is `frame_scope`'s end on the main thread, which runs jobs itself until the frame counter is zero (rayon's `install` semantics, TBB's isolation caveat respected by never taking locks around it). For code that reads better as a sequence, `async` jobs are supported through `async-task` with `counter.wait().await` registering a waker that re-enqueues the state machine; this costs one allocation per async job and is used for streaming/procgen pipelines, not for the hot frame graph. Every job declares an access set (`Read(table)`/`Write(table)`, 64-bit handles, same vocabulary as `bevy_ecs` component access); debug builds check that two concurrently executing jobs never hold conflicting sets.

**Determinism** (client and server must agree tick-for-tick). Job *execution* order is free; job *results* are not. `parallel_for` writes results by index into pre-sized output slices; reductions combine chunk results in chunk order in a single continuation, never "first finisher wins". ECS commands are buffered per worker and applied sorted by (system order, entity index). Per-item randomness is `Pcg32::seed(hash(world_seed, tick, entity.index()))`; `thread_rng`, `HashMap` iteration and time-based seeds are lint-banned in sim crates. Flecs's "same entity, same thread until sync" is adopted for `parallel_for` slicing so cache behaviour is also stable run to run. A 1000-tick hash comparison between `W = 1` and `W = 6` is a CI test.

**Frame pipeline.** Simulation N+1 ∥ render extract/compile/record N ∥ GPU N−1, frames in flight capped at 2 (Ladavac). Extraction is a job phase that copies the render-relevant inputs into an immutable frame packet, after which simulation for the next tick may start; the render graph compiles from the packet (a job), pass execution records into `pools[worker][frame_in_flight]` from per-thread command pools, and the submit thread submits with a timeline semaphore signalled to the frame index; pool reuse waits on value N−2. Async compute placement follows the GPUOpen complementary-bottleneck rule.

**API sketch (20 lines).**

```rust
pub struct Jobs { /* workers, rings, arenas */ }
pub struct Counter(/* AtomicU32 + waiter stack */);
#[derive(Clone, Copy)] pub enum Prio { High, Normal, Low }
pub struct Decl<'a> { pub name: &'static str, pub prio: Prio, pub access: &'a [Access], pub after: &'a [&'a Counter] }
pub struct Ctx { pub worker: usize, pub frame: u64 }               // handed to every job body

impl Jobs {
    pub fn new(cfg: &Config) -> Self;                                   // cfg.workers = P-2, pin masks, names, mmcss
    pub fn spawn(&self, d: Decl, f: impl FnOnce(&Ctx) + Send + 'static) -> Counter;
    pub fn parallel_for<T: Sync, R: Send>(&self, d: Decl, items: &[T], grain: usize,
        out: &mut [R], f: impl Fn(&Ctx, usize, &T) -> R + Sync) -> Counter;   // writes out[i]; deterministic
    pub fn then(&self, after: &[&Counter], d: Decl, f: impl FnOnce(&Ctx) + Send + 'static) -> Counter;
    pub fn spawn_async(&self, d: Decl, fut: impl Future<Output = ()> + Send + 'static) -> Counter; // async-task
    pub fn frame_scope<'s, R>(&'s self, f: impl FnOnce(&Scope<'s>) -> R) -> R;  // main thread helps; joins at end
    pub fn background(&self) -> &Arena;                                 // capped arena for procgen / streaming
    pub fn run_main_thread_jobs(&self);                                 // window, input, GPU submit
}
impl Counter { pub fn is_done(&self) -> bool; pub fn wait(&self) -> impl Future<Output = ()>; }
impl Ctx { pub fn rng(&self, item_seed: u64) -> Pcg32; }               // hash(world_seed, frame, item_seed)
```

**ECS decision: `bevy_ecs` 0.19 for storage, queries, relationships, hooks and observers; Forge's job graph as the executor.** Rationale: the shared client/server sim crates gain a mature, well-documented API with change detection and relationships (0.19 even unified resources as components), and Forge's editor/tooling will want reflection and observers that `hecs` and the custom-table option do not provide. The executor is *not* pluggable (`ExecutorKind` enum), but it does not need to be: Forge's `SystemGraph` stores `Box<dyn System>`s, reads their `component_access_set()` to compute the conflict bitset exactly as `multi_threaded.rs` does, and runs each system as a Forge job via `System::run_unsafe` on an `UnsafeWorldCell`, with exclusive systems as main-thread jobs and deferred commands applied at fixed points. This is ~500 lines mirroring Bevy's own executor, and it keeps ECS scheduling, `parallel_for` over queries, render-graph jobs and streaming under one scheduler with one thread budget. `ComputeTaskPool` is still initialised by Forge (same worker count, same pins, threads parked when unused) so `Query::par_iter` remains a legal fallback while the custom executor matures. `flecs_ecs` is revisited when the binding leaves alpha; the custom SoA-table option is revisited only if profiles show `bevy_ecs` archetype storage itself as the bottleneck. Bevy's ~4-month breaking cadence is mitigated by keeping all world access behind Forge's `sim` crate.

**Benchmarks and the demo that proves it** (all report p50/p90/p99/p99.9/max, exported to CSV and Tracy):
1. `bench_transform_1m`: 1M entities in a 4-level hierarchy, transform propagation as `parallel_for` over `bevy_ecs` tables, at `W ∈ {1, 2, 4, 6, 8, 14}`. Pass: ≥ 5× at `W = 6` versus 1, `p99/p50 < 1.3`.
2. `bench_dag_10k`: 10k jobs, random DAG with mean fan-in 3, trivial bodies; report T₁, T∞, T_P, per-job scheduling overhead (target < 300 ns), spawn-to-start latency (target p99 < 20 µs), against `rayon::scope`, `bevy_tasks` scope and Taskflow's published numbers.
3. `demo_frame_p99`: a synthetic frame of ~200 jobs including `parallel_for` over 100k items and a render-record stub, with a live 48 kHz / 256-frame audio callback (sine) and a tokio echo socket alive, run 5 minutes in three configurations: `W = 6` pinned, `W = 6` unpinned, and `W = 16` spin-searching (the old rayon shape). Pass: zero audio underruns, `p99 ≤ 1.25 × p50`, no frame above `2 × p50`; the `W = 16` run is expected to reproduce the 15–28 ms stalls and is kept as the regression fixture.
4. `test_determinism`: 1000 ticks of the sim at `W = 1` and `W = 6`, per-tick world hash identical.

## Checked and left out

- **Gor Nishanov, "Fibers under the magnifying glass" (WG21 P1364R0, 2018).** The PDF at open-std.org is reachable (376 KB downloaded) but binary-only here (no PDF text extraction available on this machine), and the 2018 WG21 index fetch did not surface the entry, so title/date could not be confirmed against a readable page. It is the best single critique of fibers (TLS, stack size, debugging, lock interaction) and should be added once verified.
- **Sean Middleditch / Insomniac job-system material.** `seanmiddleditch.github.io` returned 404 and the search budget was exhausted before an alternative was found; no Insomniac job-system talk was located beyond Acton's CppCon 2014 (included).
- **Ubisoft Anvil ("Rendering of Assassin's Creed") as a job-system source.** Not verified; the search budget ran out before it could be checked, and the title suggests a rendering talk rather than a scheduler talk.
- **`taskflow-rs`, `yoshi`.** Could not be verified to exist as Rust job-system crates; not cited.
- **Sebastian Aaltonen's HypeHype job system.** His SIGGRAPH 2023 Advances talk is "HypeHype Mobile Rendering Architecture" (listing verified); it is a rendering talk, and the April 2022 job-system thread does not mention HypeHype. The thread is cited on its own merits.
- **Sousa/Geffroy as a CPU job-system talk.** Verified as a rendering talk only; kept under Section A with that caveat.
- **Gyrling's slide numbers (fiber counts, stack sizes, queue counts) and Tatarchuk's exact "frame packet" wording.** Both PDFs are reachable but were not text-extracted here; the summaries above avoid restating numbers that were not re-read.
- **Bevy + rayon oversubscription as the cause of the prior stalls.** Own analysis from the documented defaults (rayon: logical CPUs; `TaskPoolOptions`: 4 + 4 + remainder); no external post asserts it.
- **`num_cpus::get_physical`, `hwloc`, Windows `GetLogicalProcessorInformationEx` for sibling mapping.** Not re-verified in this pass; the recommendation says "derive the sibling mapping" without naming the API.
- **Graham Wihlidal's Halcyon architecture posts; Unity DOTS `Entities` scheduling internals.** Not checked; out of scope for the first system.

## Verification notes

- Verification date: 23 September 2026. Web search was used for roughly 40 queries until the session's search budget was exhausted; the remaining ~50 confirmations were direct fetches of known URLs (GDC Vault, archive.org, docs.rs, GitHub, Khronos, Microsoft Learn, man7, EA/Frostbite, advances.realtimerendering.com, Wikipedia). Medium was not needed; the Our Machinery post was read from the `ruby0x1` archive.
- GDC Vault IDs confirmed by fetch or search result: Gyrling 1022186, Genova 1022164, Tatarchuk 1021926, O'Donnell 1024612, Ladavac 1025407 (page fetched; exact title confirmed).
- Crate versions seen on docs.rs at verification time: `rayon` 1.12.0, `bevy_ecs`/`bevy_tasks` 0.19.1 (13 Aug 2026), `tokio` 1.53.1 (20 Jul 2026), `async-executor` 1.14.0 (14 Sep 2026), `pollster` 1.0.1 (10 Jul 2026), `parking_lot` 0.12.5 (3 Aug 2026), `crossbeam-channel` 0.5.17, `switchyard` 0.3.1 (3 Aug 2026), `corosensei` 0.3.4, `flecs_ecs` 0.1.1 (Flecs 4.0.1). Bevy 0.19 release date 19 June 2026 from bevy.org.
- `bevy_ecs` executor behaviour (`ComputeTaskPool::get_or_init(...).scope_with_executor()`, dependency counts + conflict bitset, `spawn_on_scope` for exclusive systems, deferred application points) was read from `multi_threaded.rs` on `main`, not from a tagged release; re-check against the 0.19.1 tag before relying on it.
- `TaskPoolOptions` defaults (IO 25% 1..4, async compute 25% 1..4, compute remainder) were read from `task_pool_plugin.rs` on `main`.
- Fetched pages were summarised by an automated fetch tool; only facts that the tool quoted or that were cross-checked in a second source are stated as fact. Claims about talk contents beyond the published abstracts (e.g. Destiny's per-frame packet, Doom's async compute) are stated at the level of the abstracts and widely reported summaries, and flagged where the primary PDF was not re-read.
- The 9800X3D figures come from the Wikipedia Zen 5 table (AMD's product page timed out); they match the machine description in the brief (8C/16T, 96 MB L3).
- No browser pane was used for any check; every confirmation went through WebSearch or WebFetch. YouTube links were not opened in a browser: Acton (CppCon 2014) was confirmed from the page title returned by a WebFetch of the YouTube URL; Genova and O'Donnell YouTube links were confirmed indirectly through their GDC Vault pages and search-result snippets that list the same videos.
