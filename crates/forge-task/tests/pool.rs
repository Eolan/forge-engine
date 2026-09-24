//! Integration tests for the job system: scopes, counters, graphs, panics, priorities.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use forge_task::{Counter, GraphError, PoolConfig, Priority, TaskGraph, TaskPool};

fn pool(workers: usize) -> TaskPool {
    TaskPool::new(PoolConfig::with_workers(workers))
}

#[test]
fn scope_runs_every_job_and_returns_value() {
    let pool = pool(4);
    let hits = AtomicUsize::new(0);
    let value = pool.scope(|s| {
        for _ in 0..1000 {
            s.spawn(|_| {
                hits.fetch_add(1, Ordering::Relaxed);
            });
        }
        42
    });
    assert_eq!(value, 42);
    assert_eq!(hits.load(Ordering::Relaxed), 1000);
}

#[test]
fn scope_jobs_may_borrow_stack_data() {
    let pool = pool(3);
    let mut data = vec![0_u32; 64];
    pool.scope(|s| {
        for (i, slot) in data.iter_mut().enumerate() {
            s.spawn(move |_| *slot = i as u32 * 2);
        }
    });
    assert!(data.iter().enumerate().all(|(i, &v)| v == i as u32 * 2));
}

#[test]
fn nested_scopes_inside_jobs_do_not_deadlock() {
    let pool = pool(2);
    let total = AtomicU64::new(0);
    pool.scope(|s| {
        for outer in 0..16_u64 {
            let total = &total;
            s.spawn(move |_| {
                // A worker waiting on a nested scope keeps running other jobs.
                let pool_local = TaskPool::new(PoolConfig::with_workers(0));
                let mut part = 0_u64;
                pool_local.scope(|inner| {
                    inner.spawn(|_| part = outer * 10);
                });
                total.fetch_add(part, Ordering::Relaxed);
            });
        }
    });
    assert_eq!(
        total.load(Ordering::Relaxed),
        (0..16_u64).map(|o| o * 10).sum()
    );
}

#[test]
fn nested_scope_on_same_pool() {
    let pool = pool(2);
    let sum = AtomicU64::new(0);
    pool.scope(|s| {
        for i in 0..8_u64 {
            let sum = &sum;
            let pool = &pool;
            s.spawn(move |_| {
                pool.scope(|inner| {
                    for j in 0..8_u64 {
                        inner.spawn(move |_| {
                            sum.fetch_add(i * j, Ordering::Relaxed);
                        });
                    }
                });
            });
        }
    });
    let expected: u64 = (0..8_u64)
        .flat_map(|i| (0..8_u64).map(move |j| i * j))
        .sum();
    assert_eq!(sum.load(Ordering::Relaxed), expected);
}

#[test]
fn zero_worker_pool_runs_everything_on_the_caller() {
    let pool = pool(0);
    let hits = AtomicUsize::new(0);
    pool.scope(|s| {
        for _ in 0..100 {
            s.spawn(|_| {
                hits.fetch_add(1, Ordering::Relaxed);
            });
        }
    });
    assert_eq!(hits.load(Ordering::Relaxed), 100);
    let mut out = vec![0_u64; 1000];
    pool.par_map_into(&mut out, 7, |i| i as u64);
    assert!(out.iter().enumerate().all(|(i, &v)| v == i as u64));
}

#[test]
fn par_for_and_par_chunks_match_serial() {
    let pool = pool(4);
    let n = 100_003;
    let serial: Vec<u64> = (0..n).map(|i| forge_hash(i as u64)).collect();

    let mut chunked = vec![0_u64; n];
    pool.par_chunks_mut(&mut chunked, 1000, |chunk_index, chunk| {
        for (k, slot) in chunk.iter_mut().enumerate() {
            *slot = forge_hash((chunk_index * 1000 + k) as u64);
        }
    });
    assert_eq!(chunked, serial);

    let hits = AtomicUsize::new(0);
    pool.par_for(0..n, 64, |i| {
        assert!(i < n);
        hits.fetch_add(1, Ordering::Relaxed);
    });
    assert_eq!(hits.load(Ordering::Relaxed), n);

    let mut mapped = vec![0_u64; n];
    pool.par_map_into(&mut mapped, 333, |i| forge_hash(i as u64));
    assert_eq!(mapped, serial);
}

fn forge_hash(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[test]
fn join_returns_both_results() {
    let pool = pool(2);
    fn fib(pool: &TaskPool, n: u64) -> u64 {
        if n < 12 {
            return if n < 2 {
                n
            } else {
                fib(pool, n - 1) + fib(pool, n - 2)
            };
        }
        let (a, b) = pool.join(|| fib(pool, n - 1), || fib(pool, n - 2));
        a + b
    }
    assert_eq!(fib(&pool, 25), 75_025);
}

#[test]
fn counter_continuation_runs_after_all_signals() {
    let pool = pool(3);
    let stage_a = Counter::new();
    let done = Counter::new();
    let a_hits = Arc::new(AtomicUsize::new(0));
    let ordered = Arc::new(AtomicBool::new(true));
    for _ in 0..50 {
        let a_hits = Arc::clone(&a_hits);
        pool.spawn_signal(&stage_a, Priority::Normal, move || {
            std::thread::sleep(Duration::from_micros(50));
            a_hits.fetch_add(1, Ordering::SeqCst);
        });
    }
    {
        let a_hits = Arc::clone(&a_hits);
        let ordered = Arc::clone(&ordered);
        pool.spawn_after_signal(&stage_a, &done, Priority::High, move || {
            if a_hits.load(Ordering::SeqCst) != 50 {
                ordered.store(false, Ordering::SeqCst);
            }
        });
    }
    pool.wait(&done);
    assert!(
        ordered.load(Ordering::SeqCst),
        "continuation ran before stage A finished"
    );
    assert!(stage_a.is_zero());
}

#[test]
fn continuation_on_zero_counter_runs_immediately() {
    let pool = pool(1);
    let ready = Counter::new();
    let done = Counter::new();
    let ran = Arc::new(AtomicBool::new(false));
    let ran2 = Arc::clone(&ran);
    pool.spawn_after_signal(&ready, &done, Priority::Normal, move || {
        ran2.store(true, Ordering::SeqCst)
    });
    pool.wait(&done);
    assert!(ran.load(Ordering::SeqCst));
}

#[test]
fn spawn_task_returns_value_and_blocking_pool_works() {
    let pool = pool(2);
    let task = pool.spawn_task(Priority::Normal, || (1..=10_u64).product::<u64>());
    assert_eq!(task.wait(&pool), 3_628_800);
    let io = pool.spawn_blocking(|| {
        std::thread::sleep(Duration::from_millis(5));
        "loaded"
    });
    assert_eq!(io.wait(&pool), "loaded");
    let blocking_direct = pool.blocking().spawn(|| 7);
    assert_eq!(blocking_direct.wait_blocking(), 7);
}

#[test]
fn panics_in_scope_jobs_propagate_and_pool_survives() {
    let pool = pool(2);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pool.scope(|s| {
            s.spawn(|_| panic!("boom"));
            for _ in 0..10 {
                s.spawn(|_| std::thread::sleep(Duration::from_micros(100)));
            }
        });
    }));
    assert!(result.is_err());
    let mut x = 0;
    let value = pool.scope(|s| {
        s.spawn(|_| x = 5);
        7
    });
    assert_eq!((value, x), (7, 5));
    let task = pool.spawn_task(Priority::Normal, || -> u32 { panic!("task boom") });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task.wait(&pool))).is_err());
    assert_eq!(pool.spawn_task(Priority::Normal, || 3).wait(&pool), 3);
}

#[test]
fn detached_panic_does_not_kill_workers() {
    let pool = pool(1);
    let done = Counter::new();
    pool.spawn(|| panic!("detached boom"));
    pool.spawn_signal(&done, Priority::Normal, || {});
    pool.wait(&done);
    let stats = pool.stats();
    assert!(stats.workers.iter().map(|w| w.panics).sum::<u64>() <= 1);
    assert_eq!(pool.spawn_task(Priority::Normal, || 1).wait(&pool), 1);
}

#[test]
fn graph_respects_dependencies() {
    let pool = pool(4);
    let n = 200;
    let finished: Arc<Vec<AtomicBool>> = Arc::new((0..n).map(|_| AtomicBool::new(false)).collect());
    let violations = Arc::new(AtomicUsize::new(0));
    let mut graph = TaskGraph::new();
    let mut ids = Vec::new();
    for i in 0..n {
        let deps: Vec<_> = (1..=3)
            .filter(|&d| i >= d * 7)
            .map(|d| ids[i - d * 7])
            .collect();
        let finished = Arc::clone(&finished);
        let violations = Arc::clone(&violations);
        let dep_indices: Vec<usize> = (1..=3).filter(|&d| i >= d * 7).map(|d| i - d * 7).collect();
        let id = graph.add(format!("node{i}"), &deps, move || {
            for &d in &dep_indices {
                if !finished[d].load(Ordering::SeqCst) {
                    violations.fetch_add(1, Ordering::SeqCst);
                }
            }
            std::thread::sleep(Duration::from_micros(20));
            finished[i].store(true, Ordering::SeqCst);
        });
        ids.push(id);
    }
    graph.run(&pool).unwrap();
    assert_eq!(violations.load(Ordering::SeqCst), 0);
    assert!(finished.iter().all(|f| f.load(Ordering::SeqCst)));
}

#[test]
fn graph_detects_cycles_and_bad_dependencies() {
    let pool = pool(1);
    let mut graph = TaskGraph::new();
    let a = graph.add("a", &[], || {});
    // b depends on c which depends on b: c is added after b so we build the cycle via a
    // forward reference to an id we compute ahead of time.
    let b_id = forge_task::NodeId::from_index(1);
    let c_id = forge_task::NodeId::from_index(2);
    let _b = graph.add("b", &[a, c_id], || {});
    let _c = graph.add("c", &[b_id], || {});
    match graph.run(&pool) {
        Err(GraphError::Cycle(_)) => {}
        other => panic!("expected a cycle, got {other:?}"),
    }

    let mut graph = TaskGraph::new();
    graph.add("lonely", &[forge_task::NodeId::from_index(9)], || {});
    assert!(matches!(
        graph.run(&pool),
        Err(GraphError::UnknownDependency { .. })
    ));
}

#[test]
fn graph_panic_is_reported_after_completion() {
    let pool = pool(2);
    let mut graph = TaskGraph::new();
    let a = graph.add("a", &[], || panic!("node a"));
    let ran_b = Arc::new(AtomicBool::new(false));
    let ran_b2 = Arc::clone(&ran_b);
    graph.add("b", &[a], move || ran_b2.store(true, Ordering::SeqCst));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| graph.run(&pool)));
    assert!(result.is_err());
    assert!(
        ran_b.load(Ordering::SeqCst),
        "dependents still run so the graph completes"
    );
}

#[test]
fn priorities_are_drained_high_first_by_a_single_worker() {
    let pool = pool(1);
    // Hold the worker busy so everything queues up, then observe the order it drains in.
    let gate = Counter::with_value(1);
    let order = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let done = Counter::new();
    {
        let gate = gate.clone();
        pool.spawn_signal(&done, Priority::High, move || gate.wait_blocking());
    }
    std::thread::sleep(Duration::from_millis(20));
    for (priority, label) in [
        (Priority::Low, "low"),
        (Priority::Normal, "normal"),
        (Priority::High, "high"),
    ] {
        for _ in 0..3 {
            let order = Arc::clone(&order);
            pool.spawn_signal(&done, priority, move || order.lock().push(label));
        }
    }
    gate.decrement();
    done.wait_blocking();
    let order = order.lock().clone();
    assert_eq!(&order[..3], &["high"; 3]);
    assert_eq!(&order[3..6], &["normal"; 3]);
    assert_eq!(&order[6..], &["low"; 3]);
}

#[test]
fn stress_random_nested_work() {
    let pool = pool(4);
    let start = Instant::now();
    let total = AtomicU64::new(0);
    for round in 0..200_u64 {
        pool.scope(|s| {
            for i in 0..(round % 17 + 1) {
                let total = &total;
                let pool = &pool;
                s.spawn(move |s| {
                    if i % 3 == 0 {
                        pool.par_for(0..(i as usize * 50), 8, |_| {
                            total.fetch_add(1, Ordering::Relaxed);
                        });
                    } else {
                        s.spawn(move |_| {
                            total.fetch_add(i, Ordering::Relaxed);
                        });
                    }
                });
            }
        });
    }
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "stress test took too long"
    );
    assert!(total.load(Ordering::Relaxed) > 0);
}

#[test]
fn drop_finishes_queued_jobs() {
    let ran = Arc::new(AtomicUsize::new(0));
    {
        let pool = pool(2);
        for _ in 0..500 {
            let ran = Arc::clone(&ran);
            pool.spawn(move || {
                ran.fetch_add(1, Ordering::Relaxed);
            });
        }
    }
    assert_eq!(ran.load(Ordering::Relaxed), 500);
}
