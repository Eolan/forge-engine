//! `task-bench`: measures the job system and demonstrates the frame-pacing rule.
//!
//! Run with `cargo run --release -p task-bench`. Options: `--workers N`, `--frames N`,
//! `--pin`, `--quick`.

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use forge_core::hash::mix64;
use forge_task::{Counter, PoolConfig, Priority, TaskGraph, TaskPool};

#[derive(Parser, Debug)]
#[command(about = "forge-task benchmarks")]
struct Args {
    /// Compute workers (default: physical cores - 2, the client rule).
    #[arg(long)]
    workers: Option<usize>,
    /// Frames to simulate in the pacing demo.
    #[arg(long, default_value_t = 600)]
    frames: usize,
    /// Pin workers to cores.
    #[arg(long)]
    pin: bool,
    /// Smaller problem sizes.
    #[arg(long)]
    quick: bool,
}

/// ~2 ns per iteration of a dependent hash chain.
fn work(seed: u64, iterations: u32) -> u64 {
    let mut h = seed;
    for i in 0..iterations {
        h = mix64(h ^ u64::from(i));
    }
    h
}

fn make_pool(workers: usize, pin: bool) -> TaskPool {
    let mut config = PoolConfig::with_workers(workers);
    config.pin_workers = pin;
    TaskPool::new(config)
}

fn time<R>(f: impl FnOnce() -> R) -> (R, Duration) {
    let start = Instant::now();
    let r = f();
    (r, start.elapsed())
}

fn best_of<R>(runs: usize, mut f: impl FnMut() -> R) -> (R, Duration) {
    let mut best: Option<(R, Duration)> = None;
    for _ in 0..runs {
        let (r, d) = time(&mut f);
        if best.as_ref().is_none_or(|(_, bd)| d < *bd) {
            best = Some((r, d));
        }
    }
    best.expect("at least one run")
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx]
}

fn bench_par_for(workers: usize, pin: bool, quick: bool) {
    let n: usize = if quick { 2_000_000 } else { 16_000_000 };
    let grain = 4096;
    println!("\n== Data-parallel map: {n} elements, grain {grain} ==");
    let mut out = vec![0_u64; n];
    let (_, serial) = best_of(3, || {
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = mix64(i as u64 ^ 0xABCD);
        }
        black_box(&out);
    });
    let serial_sum: u64 = out.iter().fold(0, |a, &b| a.wrapping_add(b));

    let pool = make_pool(workers, pin);
    let (_, forge) = best_of(5, || {
        pool.par_chunks_mut(&mut out, grain, |chunk, slice| {
            let base = chunk * grain;
            for (k, slot) in slice.iter_mut().enumerate() {
                *slot = mix64((base + k) as u64 ^ 0xABCD);
            }
        });
        black_box(&out);
    });
    let forge_sum: u64 = out.iter().fold(0, |a, &b| a.wrapping_add(b));
    assert_eq!(forge_sum, serial_sum, "parallel result differs from serial");
    let stats = pool.stats();
    drop(pool);

    let rayon_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .expect("rayon pool");
    let (_, rayon_time) = best_of(5, || {
        rayon_pool.install(|| {
            use rayon::prelude::*;
            out.par_chunks_mut(grain)
                .enumerate()
                .for_each(|(chunk, slice)| {
                    let base = chunk * grain;
                    for (k, slot) in slice.iter_mut().enumerate() {
                        *slot = mix64((base + k) as u64 ^ 0xABCD);
                    }
                });
        });
        black_box(&out);
    });

    println!("  serial            {:>8.3} ms", ms(serial));
    println!(
        "  forge-task ({workers} w) {:>8.3} ms  speedup x{:.2}  (steals {}, parks {})",
        ms(forge),
        serial.as_secs_f64() / forge.as_secs_f64(),
        stats.steals(),
        stats.parks()
    );
    println!(
        "  rayon      ({workers} t) {:>8.3} ms  speedup x{:.2}",
        ms(rayon_time),
        serial.as_secs_f64() / rayon_time.as_secs_f64()
    );
}

fn bench_fork_join(workers: usize, pin: bool, quick: bool) {
    let n: u64 = if quick { 27 } else { 30 };
    println!("\n== Fork-join recursion: fib({n}) via join, serial cutoff at 16 ==");
    // `black_box` keeps the optimiser from folding the recursion at compile time.
    fn fib_serial(n: u64) -> u64 {
        if n < 2 {
            black_box(n)
        } else {
            fib_serial(n - 1) + fib_serial(n - 2)
        }
    }
    fn fib(pool: &TaskPool, n: u64) -> u64 {
        if n < 16 {
            return fib_serial(black_box(n));
        }
        let (a, b) = pool.join(|| fib(pool, n - 1), || fib(pool, n - 2));
        a + b
    }
    fn fib_rayon(n: u64) -> u64 {
        if n < 16 {
            return fib_serial(black_box(n));
        }
        let (a, b) = rayon::join(|| fib_rayon(n - 1), || fib_rayon(n - 2));
        a + b
    }
    let (expected, serial) = best_of(3, || fib_serial(black_box(n)));
    let pool = make_pool(workers, pin);
    let (got, forge) = best_of(5, || fib(&pool, black_box(n)));
    assert_eq!(got, expected);
    let jobs = pool.stats().jobs();
    drop(pool);
    let rayon_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .expect("rayon pool");
    let (_, rayon_time) = best_of(5, || rayon_pool.install(|| fib_rayon(black_box(n))));
    println!("  serial            {:>8.3} ms", ms(serial));
    println!(
        "  forge-task        {:>8.3} ms  speedup x{:.2}  ({jobs} jobs, {:.0} ns/job overhead incl. work)",
        ms(forge),
        serial.as_secs_f64() / forge.as_secs_f64(),
        forge.as_nanos() as f64 * workers as f64 / jobs.max(1) as f64
    );
    println!(
        "  rayon             {:>8.3} ms  speedup x{:.2}",
        ms(rayon_time),
        serial.as_secs_f64() / rayon_time.as_secs_f64()
    );
}

fn bench_spawn_throughput(workers: usize, pin: bool, quick: bool) {
    let n: u64 = if quick { 50_000 } else { 300_000 };
    println!(
        "\n== Detached spawn throughput: {n} tiny jobs (~100 ns each) from the main thread =="
    );
    let pool = make_pool(workers, pin);
    let (_, elapsed) = best_of(3, || {
        let done = Counter::new();
        for i in 0..n {
            pool.spawn_signal(&done, Priority::Normal, move || {
                black_box(work(i, 50));
            });
        }
        pool.wait(&done);
    });
    println!(
        "  {:>8.3} ms total, {:.2} M jobs/s, {:.0} ns per spawn+run",
        ms(elapsed),
        n as f64 / elapsed.as_secs_f64() / 1e6,
        elapsed.as_nanos() as f64 / n as f64
    );
}

fn bench_latency(workers: usize, pin: bool) {
    println!("\n== Single-job round trip latency (spawn on main → run on worker → wait) ==");
    let pool = make_pool(workers, pin);
    let mut start_lat = Vec::new();
    let mut round_trip = Vec::new();
    for _ in 0..2000 {
        let t0 = Instant::now();
        let task = pool.spawn_task(Priority::High, move || t0.elapsed());
        let started = task.wait(&pool);
        round_trip.push(t0.elapsed().as_secs_f64() * 1e6);
        start_lat.push(started.as_secs_f64() * 1e6);
        // Let workers park between samples: this is the cold path, the one that matters
        // for input-driven work at the start of a frame.
        thread::sleep(Duration::from_micros(300));
    }
    start_lat.sort_by(f64::total_cmp);
    round_trip.sort_by(f64::total_cmp);
    println!(
        "  spawn→start  p50 {:>6.1} µs  p99 {:>6.1} µs  max {:>7.1} µs",
        percentile(&start_lat, 0.5),
        percentile(&start_lat, 0.99),
        percentile(&start_lat, 1.0)
    );
    println!(
        "  round trip   p50 {:>6.1} µs  p99 {:>6.1} µs  max {:>7.1} µs",
        percentile(&round_trip, 0.5),
        percentile(&round_trip, 0.99),
        percentile(&round_trip, 1.0)
    );
}

fn bench_graph(workers: usize, pin: bool, quick: bool) {
    let layers = if quick { 40 } else { 100 };
    let width = 100;
    let iters = 2500; // ≈ 5 µs per node
    println!(
        "\n== Task graph: {} nodes in {layers} layers × {width}, ~5 µs each, 2 deps per node ==",
        layers * width
    );
    let pool = make_pool(workers, pin);
    let build = || {
        let mut graph = TaskGraph::new();
        let mut previous: Vec<forge_task::NodeId> = Vec::new();
        let sink = Arc::new(AtomicU64::new(0));
        for layer in 0..layers {
            let mut current = Vec::with_capacity(width);
            for i in 0..width {
                let deps: Vec<_> = if layer == 0 {
                    Vec::new()
                } else {
                    vec![previous[i], previous[(i * 7 + 3) % width]]
                };
                let sink = Arc::clone(&sink);
                let seed = (layer * width + i) as u64;
                current.push(graph.add("n", &deps, move || {
                    sink.fetch_add(work(seed, iters), Ordering::Relaxed);
                }));
            }
            previous = current;
        }
        (graph, sink)
    };
    let (_, serial) = best_of(2, || {
        let mut acc = 0_u64;
        for i in 0..(layers * width) as u64 {
            acc = acc.wrapping_add(work(i, iters));
        }
        black_box(acc);
    });
    let (_, elapsed) = best_of(5, || {
        let (graph, sink) = build();
        graph.run(&pool).expect("valid graph");
        black_box(sink.load(Ordering::Relaxed));
    });
    let critical_path = serial.as_secs_f64() / (layers * width) as f64 * layers as f64;
    println!("  serial work        {:>8.3} ms", ms(serial));
    println!(
        "  graph on {workers} w     {:>8.3} ms  speedup x{:.2}  (critical path {:.3} ms, ideal {:.3} ms)",
        ms(elapsed),
        serial.as_secs_f64() / elapsed.as_secs_f64(),
        critical_path * 1e3,
        (serial.as_secs_f64() / workers as f64).max(critical_path) * 1e3
    );
}

/// A stand-in for the audio thread: wakes every `period`, does `busy` of work, and records
/// how late each wake-up was. On a machine where the job system hogs every core, it misses.
struct RealtimeThread {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<(Vec<f64>, u64)>>,
}

impl RealtimeThread {
    fn start(period: Duration, busy: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("fake-audio".into())
            .spawn(move || {
                let mut lateness = Vec::new();
                let mut misses = 0_u64;
                let mut next = Instant::now() + period;
                while !stop2.load(Ordering::Relaxed) {
                    // Sleep most of the way, then spin: sleep granularity is the enemy here.
                    loop {
                        let now = Instant::now();
                        if now >= next {
                            break;
                        }
                        let remaining = next - now;
                        if remaining > Duration::from_micros(1500) {
                            thread::sleep(remaining - Duration::from_micros(1000));
                        } else {
                            std::hint::spin_loop();
                        }
                    }
                    let late = Instant::now().saturating_duration_since(next);
                    lateness.push(late.as_secs_f64() * 1e3);
                    if late > Duration::from_millis(1) {
                        misses += 1;
                    }
                    let t = Instant::now();
                    while t.elapsed() < busy {
                        black_box(work(1, 100));
                    }
                    next += period;
                }
                (lateness, misses)
            })
            .expect("spawn realtime thread");
        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn stop(mut self) -> (Vec<f64>, u64) {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .expect("handle")
            .join()
            .expect("realtime thread")
    }
}

fn simulate_frames(pool: &TaskPool, frames: usize) -> Vec<f64> {
    let mut buffer_a = vec![0_u64; 600_000];
    let mut buffer_c = vec![0_u64; 300_000];
    let mut frame_times = Vec::with_capacity(frames);
    for frame in 0..frames {
        let start = Instant::now();
        // Phase A: wide data-parallel pass (e.g. transform update).
        pool.par_chunks_mut(&mut buffer_a, 2048, |chunk, slice| {
            let base = chunk * 2048;
            for (k, slot) in slice.iter_mut().enumerate() {
                *slot = work((base + k) as u64 ^ frame as u64, 4);
            }
        });
        // Phase B: a small DAG of heterogeneous stages (e.g. culling → sorting → recording).
        let mut graph = TaskGraph::new();
        let mut previous = Vec::new();
        for layer in 0..4 {
            let mut current = Vec::new();
            for i in 0..16 {
                let deps: Vec<_> = if layer == 0 {
                    vec![]
                } else {
                    vec![previous[i], previous[(i + 5) % 16]]
                };
                let seed = (frame * 64 + layer * 16 + i) as u64;
                current.push(
                    graph.add_with_priority("stage", Priority::High, &deps, move || {
                        black_box(work(seed, 8000));
                    }),
                );
            }
            previous = current;
        }
        graph.run(pool).expect("valid graph");
        // Phase C: another data-parallel pass (e.g. animation / particles).
        pool.par_chunks_mut(&mut buffer_c, 1024, |chunk, slice| {
            let base = chunk * 1024;
            for (k, slot) in slice.iter_mut().enumerate() {
                *slot = work((base + k) as u64 ^ 0x77, 4);
            }
        });
        black_box((&buffer_a, &buffer_c));
        frame_times.push(start.elapsed().as_secs_f64() * 1e3);
    }
    frame_times
}

/// Endless low-priority background work (chunk generation, asset decoding): `streams`
/// self-re-spawning jobs of ~`job_len` each, so the queues are never empty.
fn start_background_load(
    pool: &Arc<TaskPool>,
    streams: usize,
    job_len: Duration,
) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    fn step(pool: Arc<TaskPool>, stop: Arc<AtomicBool>, job_len: Duration, seed: u64) {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let t = Instant::now();
        let mut h = seed;
        while t.elapsed() < job_len {
            h = work(h, 100);
        }
        black_box(h);
        let next = Arc::clone(&pool);
        pool.spawn_with_priority(Priority::Low, move || step(next, stop, job_len, seed + 1));
    }
    for stream in 0..streams {
        let pool_ref = Arc::clone(pool);
        let stop = Arc::clone(&stop);
        pool.spawn_with_priority(Priority::Low, move || {
            step(pool_ref, stop, job_len, stream as u64)
        });
    }
    stop
}

fn bench_frames(client_workers: usize, all_threads: usize, pin: bool, frames: usize) {
    println!(
        "\n== Frame pacing demo: {frames} frames of A(par) → B(graph) → C(par) at High priority =="
    );
    println!(
        "   plus endless Low-priority background jobs (200 µs each) and a 2.9 ms real-time thread alongside."
    );
    println!(
        "   (This shows why the client pool leaves cores free. Numbers depend on what else the machine is doing.)"
    );
    for (label, workers) in [
        ("client rule", client_workers),
        ("every hw thread", all_threads),
    ] {
        let pool = Arc::new(make_pool(workers, pin));
        let realtime =
            RealtimeThread::start(Duration::from_micros(2900), Duration::from_micros(200));
        let stop_background = start_background_load(&pool, workers * 2, Duration::from_micros(200));
        thread::sleep(Duration::from_millis(50));
        let mut times = simulate_frames(&pool, frames);
        let (mut lateness, misses) = realtime.stop();
        stop_background.store(true, Ordering::Relaxed);
        let stats = pool.stats();
        drop(pool);
        times.sort_by(f64::total_cmp);
        lateness.sort_by(f64::total_cmp);
        println!(
            "  {label:<16} {workers:>2} workers | frame p50 {:>6.3} ms  p99 {:>6.3} ms  max {:>7.3} ms | audio late p99 {:>6.3} ms  max {:>6.3} ms  misses>1ms {misses:>3} / {} | steals {} parks {}",
            percentile(&times, 0.5),
            percentile(&times, 0.99),
            percentile(&times, 1.0),
            percentile(&lateness, 0.99),
            percentile(&lateness, 1.0),
            lateness.len(),
            stats.steals(),
            stats.parks()
        );
    }
}

fn main() {
    let args = Args::parse();
    let physical = num_cpus::get_physical();
    let logical = num_cpus::get();
    let client = PoolConfig::client().workers;
    let workers = args.workers.unwrap_or(client);
    println!(
        "forge-task bench — {physical} physical cores, {logical} hardware threads; client rule → {client} workers; using {workers}"
    );
    bench_par_for(workers, args.pin, args.quick);
    bench_fork_join(workers, args.pin, args.quick);
    bench_spawn_throughput(workers, args.pin, args.quick);
    bench_latency(workers, args.pin);
    bench_graph(workers, args.pin, args.quick);
    bench_frames(
        client,
        logical,
        args.pin,
        if args.quick {
            args.frames.min(200)
        } else {
            args.frames
        },
    );
}
