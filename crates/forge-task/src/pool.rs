use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use crossbeam_utils::CachePadded;
use parking_lot::{Condvar, Mutex};

use crate::blocking::BlockingPool;
use crate::counter::Counter;
use crate::job::Job;
use crate::scope::Scope;
use crate::task::Task;
use crate::{PRIORITY_COUNT, Priority};

/// Configuration of a [`TaskPool`].
#[derive(Clone, Debug)]
pub struct PoolConfig {
    /// Number of compute worker threads. The thread that calls [`TaskPool::wait`] or
    /// [`TaskPool::scope`] also runs jobs while it waits, so `workers` may be zero.
    pub workers: usize,
    /// Threads of the attached [`BlockingPool`] for I/O.
    pub blocking_threads: usize,
    /// Pin each worker to a core (skipping core 0 for the main thread). Opt-in: pinning helps
    /// cache locality on a dedicated machine and hurts on a shared one.
    pub pin_workers: bool,
    /// Thread name prefix (shown in debuggers and Tracy).
    pub thread_name: String,
    /// Spin iterations before a worker parks when it finds nothing to do.
    pub spin_tries: u32,
    /// Safety-net wake-up period for parked workers.
    pub park_timeout: Duration,
}

impl PoolConfig {
    fn base(workers: usize) -> Self {
        Self {
            workers,
            blocking_threads: 2,
            pin_workers: false,
            thread_name: "forge-worker".to_owned(),
            spin_tries: 256,
            park_timeout: Duration::from_millis(2),
        }
    }

    /// A pool with exactly `workers` compute threads.
    pub fn with_workers(workers: usize) -> Self {
        Self::base(workers)
    }

    /// Client default: one worker per physical core minus two, leaving room for the main
    /// thread (window, input, orchestration) and the render / audio threads.
    pub fn client() -> Self {
        let physical = num_cpus::get_physical().max(1);
        Self::base(physical.saturating_sub(2).max(1))
    }

    /// Headless server default: one worker per hardware thread.
    pub fn server() -> Self {
        Self::base(num_cpus::get().max(1))
    }
}

/// Per-worker counters, one cache line each.
#[derive(Default)]
pub(crate) struct WorkerStats {
    pub jobs: AtomicU64,
    pub steals: AtomicU64,
    pub parks: AtomicU64,
    pub panics: AtomicU64,
}

/// A copy of one worker's counters.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorkerStatsSnapshot {
    /// Jobs executed.
    pub jobs: u64,
    /// Jobs taken from another worker's queue.
    pub steals: u64,
    /// Times the worker went to sleep.
    pub parks: u64,
    /// Detached jobs that panicked.
    pub panics: u64,
}

/// Pool-wide statistics.
#[derive(Clone, Debug, Default)]
pub struct PoolStats {
    /// Per-worker counters, in worker order.
    pub workers: Vec<WorkerStatsSnapshot>,
    /// Jobs run by non-worker threads while helping in [`TaskPool::wait`] / [`TaskPool::scope`].
    pub helped_jobs: u64,
}

impl PoolStats {
    /// Total jobs executed by workers and helpers.
    pub fn jobs(&self) -> u64 {
        self.workers.iter().map(|w| w.jobs).sum::<u64>() + self.helped_jobs
    }
    /// Total steals.
    pub fn steals(&self) -> u64 {
        self.workers.iter().map(|w| w.steals).sum()
    }
    /// Total parks.
    pub fn parks(&self) -> u64 {
        self.workers.iter().map(|w| w.parks).sum()
    }
}

/// State shared by the pool handle, its workers and every counter continuation.
pub(crate) struct Shared {
    id: u64,
    injectors: [Injector<Job>; PRIORITY_COUNT],
    stealers: Vec<[Stealer<Job>; PRIORITY_COUNT]>,
    /// Bumped on every push; workers compare it before parking to avoid lost wake-ups.
    epoch: AtomicU64,
    /// Number of parked workers; producers only take the wake lock when this is non-zero.
    idle: AtomicUsize,
    sleep: Mutex<()>,
    wake: Condvar,
    shutdown: AtomicBool,
    stats: Vec<CachePadded<WorkerStats>>,
    helped_jobs: CachePadded<AtomicU64>,
    spin_tries: u32,
    park_timeout: Duration,
}

struct WorkerLocal {
    pool_id: u64,
    index: usize,
    queues: [Worker<Job>; PRIORITY_COUNT],
}

thread_local! {
    static LOCAL: RefCell<Option<WorkerLocal>> = const { RefCell::new(None) };
    static STEAL_RNG: Cell<u64> = const { Cell::new(0) };
}

static NEXT_POOL_ID: AtomicU64 = AtomicU64::new(1);

fn xorshift(state: &mut u64) -> u64 {
    if *state == 0 {
        *state = 0x9E37_79B9_7F4A_7C15;
    }
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

impl Shared {
    /// Queues a job: on the local deque when called from one of this pool's workers, on the
    /// global injector otherwise. Wakes a parked worker if any.
    pub(crate) fn push(&self, priority: Priority, job: Job) {
        let mut job = Some(job);
        LOCAL.with(|cell| {
            if let Some(local) = cell.borrow().as_ref()
                && local.pool_id == self.id
            {
                local.queues[priority as usize].push(job.take().expect("job present"));
            }
        });
        if let Some(job) = job {
            self.injectors[priority as usize].push(job);
        }
        self.epoch.fetch_add(1, Ordering::SeqCst);
        if self.idle.load(Ordering::SeqCst) > 0 {
            let _guard = self.sleep.lock();
            self.wake.notify_one();
        }
    }

    fn wake_all(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let _guard = self.sleep.lock();
        self.wake.notify_all();
    }

    /// Finds a runnable job, highest priority first: own queue, then the injector, then steal.
    fn find_work(&self, local: Option<&WorkerLocal>, rng: &mut u64) -> Option<Job> {
        for priority in 0..PRIORITY_COUNT {
            if let Some(local) = local
                && let Some(job) = local.queues[priority].pop()
            {
                return Some(job);
            }
            loop {
                let stolen = match local {
                    Some(local) => self.injectors[priority]
                        .steal_batch_with_limit_and_pop(&local.queues[priority], 16),
                    None => self.injectors[priority].steal(),
                };
                match stolen {
                    Steal::Success(job) => return Some(job),
                    Steal::Empty => break,
                    Steal::Retry => continue,
                }
            }
            let count = self.stealers.len();
            if count == 0 {
                continue;
            }
            let start = (xorshift(rng) % count as u64) as usize;
            for k in 0..count {
                let victim = (start + k) % count;
                if local.is_some_and(|l| l.index == victim) {
                    continue;
                }
                loop {
                    match self.stealers[victim][priority].steal() {
                        Steal::Success(job) => {
                            if let Some(local) = local {
                                self.stats[local.index]
                                    .steals
                                    .fetch_add(1, Ordering::Relaxed);
                            }
                            return Some(job);
                        }
                        Steal::Empty => break,
                        Steal::Retry => continue,
                    }
                }
            }
        }
        None
    }

    fn find_work_from_current_thread(&self) -> Option<Job> {
        let mut rng = STEAL_RNG.with(Cell::get);
        if rng == 0 {
            rng = NEXT_POOL_ID.fetch_add(0x9E37_79B9, Ordering::Relaxed) ^ 0xD1B5_4A32_D192_ED03;
        }
        let job = LOCAL.with(|cell| {
            let borrowed = cell.borrow();
            let local = borrowed.as_ref().filter(|l| l.pool_id == self.id);
            self.find_work(local, &mut rng)
        });
        STEAL_RNG.with(|c| c.set(rng));
        job
    }

    /// Runs jobs on the calling thread until `done()` holds. Sleeps briefly on `park_on`
    /// when there is nothing to run.
    pub(crate) fn help_until(&self, done: impl Fn() -> bool, park_on: Option<&Counter>) {
        let mut idle_rounds = 0_u32;
        while !done() {
            match self.find_work_from_current_thread() {
                Some(job) => {
                    idle_rounds = 0;
                    run_job(job, None);
                    self.helped_jobs.fetch_add(1, Ordering::Relaxed);
                }
                None => {
                    idle_rounds += 1;
                    if idle_rounds < 32 {
                        std::hint::spin_loop();
                    } else if idle_rounds < 64 || park_on.is_none() {
                        thread::yield_now();
                    } else if let Some(counter) = park_on {
                        counter.wait_timeout(Duration::from_micros(200));
                    }
                }
            }
        }
    }

    fn is_worker_thread(&self) -> bool {
        LOCAL.with(|cell| cell.borrow().as_ref().is_some_and(|l| l.pool_id == self.id))
    }
}

fn run_job(job: Job, stats: Option<&WorkerStats>) {
    #[cfg(feature = "profiling")]
    let _zone = tracy_client::span!("job");
    if catch_unwind(AssertUnwindSafe(|| job.run())).is_err() {
        // Scope and graph jobs catch their own panics and re-raise them on the waiting thread;
        // only fire-and-forget jobs land here.
        if let Some(stats) = stats {
            stats.panics.fetch_add(1, Ordering::Relaxed);
        }
        tracing::error!("detached job panicked; the worker continues");
    }
}

fn worker_main(shared: Arc<Shared>, local: WorkerLocal) {
    let index = local.index;
    LOCAL.with(|cell| *cell.borrow_mut() = Some(local));
    let stats = &shared.stats[index];
    let mut rng = 0x2545_F491_4F6C_DD1D ^ (index as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut idle_rounds = 0_u32;
    loop {
        let epoch = shared.epoch.load(Ordering::SeqCst);
        let job = LOCAL.with(|cell| {
            let borrowed = cell.borrow();
            shared.find_work(borrowed.as_ref(), &mut rng)
        });
        if let Some(job) = job {
            idle_rounds = 0;
            run_job(job, Some(stats));
            stats.jobs.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if shared.shutdown.load(Ordering::SeqCst) {
            break;
        }
        idle_rounds += 1;
        if idle_rounds < shared.spin_tries {
            if idle_rounds.is_multiple_of(16) {
                thread::yield_now();
            } else {
                std::hint::spin_loop();
            }
            continue;
        }
        idle_rounds = 0;
        shared.idle.fetch_add(1, Ordering::SeqCst);
        {
            let mut guard = shared.sleep.lock();
            if shared.epoch.load(Ordering::SeqCst) == epoch
                && !shared.shutdown.load(Ordering::SeqCst)
            {
                shared.wake.wait_for(&mut guard, shared.park_timeout);
            }
        }
        shared.idle.fetch_sub(1, Ordering::SeqCst);
        stats.parks.fetch_add(1, Ordering::Relaxed);
    }
    LOCAL.with(|cell| *cell.borrow_mut() = None);
}

/// The job system: compute workers plus an attached blocking pool.
pub struct TaskPool {
    shared: Arc<Shared>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    blocking: BlockingPool,
}

impl TaskPool {
    /// Starts the worker threads.
    pub fn new(config: PoolConfig) -> Self {
        let id = NEXT_POOL_ID.fetch_add(1, Ordering::Relaxed);
        let workers: Vec<[Worker<Job>; PRIORITY_COUNT]> = (0..config.workers)
            .map(|_| [Worker::new_lifo(), Worker::new_lifo(), Worker::new_lifo()])
            .collect();
        let stealers = workers
            .iter()
            .map(|queues| {
                [
                    queues[0].stealer(),
                    queues[1].stealer(),
                    queues[2].stealer(),
                ]
            })
            .collect();
        let shared = Arc::new(Shared {
            id,
            injectors: [Injector::new(), Injector::new(), Injector::new()],
            stealers,
            epoch: AtomicU64::new(0),
            idle: AtomicUsize::new(0),
            sleep: Mutex::new(()),
            wake: Condvar::new(),
            shutdown: AtomicBool::new(false),
            stats: (0..config.workers)
                .map(|_| CachePadded::new(WorkerStats::default()))
                .collect(),
            helped_jobs: CachePadded::new(AtomicU64::new(0)),
            spin_tries: config.spin_tries,
            park_timeout: config.park_timeout,
        });
        let core_ids = if config.pin_workers {
            core_affinity::get_core_ids()
        } else {
            None
        };
        let physical = num_cpus::get_physical().max(1);
        let threads = workers
            .into_iter()
            .enumerate()
            .map(|(index, queues)| {
                let shared = Arc::clone(&shared);
                let name = format!("{}-{index}", config.thread_name);
                let core = core_ids.as_ref().map(|ids| {
                    // Skip core 0 (and its SMT sibling) for the main thread; spread across
                    // physical cores when the OS lists siblings next to each other.
                    let stride = if ids.len() >= physical * 2 { 2 } else { 1 };
                    ids[(stride * (index + 1)) % ids.len()]
                });
                thread::Builder::new()
                    .name(name.clone())
                    .spawn(move || {
                        if let Some(core) = core {
                            core_affinity::set_for_current(core);
                        }
                        #[cfg(feature = "profiling")]
                        if let Some(client) = tracy_client::Client::running() {
                            client.set_thread_name(&name);
                        }
                        worker_main(
                            shared,
                            WorkerLocal {
                                pool_id: id,
                                index,
                                queues,
                            },
                        );
                    })
                    .expect("spawn worker thread")
            })
            .collect();
        let blocking = BlockingPool::new(
            config.blocking_threads,
            &format!("{}-io", config.thread_name),
        );
        Self {
            shared,
            threads: Mutex::new(threads),
            blocking,
        }
    }

    /// A pool with the [`PoolConfig::client`] defaults.
    pub fn client() -> Self {
        Self::new(PoolConfig::client())
    }

    /// A pool with the [`PoolConfig::server`] defaults.
    pub fn server() -> Self {
        Self::new(PoolConfig::server())
    }

    /// Number of compute worker threads.
    pub fn worker_count(&self) -> usize {
        self.shared.stealers.len()
    }

    /// Whether the calling thread is one of this pool's workers.
    pub fn is_worker_thread(&self) -> bool {
        self.shared.is_worker_thread()
    }

    /// The attached blocking pool.
    pub fn blocking(&self) -> &BlockingPool {
        &self.blocking
    }

    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// Fire-and-forget at [`Priority::Normal`].
    pub fn spawn<F: FnOnce() + Send + 'static>(&self, f: F) {
        self.spawn_with_priority(Priority::Normal, f);
    }

    /// Fire-and-forget at the given priority.
    pub fn spawn_with_priority<F: FnOnce() + Send + 'static>(&self, priority: Priority, f: F) {
        self.shared.push(priority, Job::new(Box::new(f)));
    }

    /// Schedules `f` and counts it on `signal` (incremented now, decremented when `f` is done).
    pub fn spawn_signal<F: FnOnce() + Send + 'static>(
        &self,
        signal: &Counter,
        priority: Priority,
        f: F,
    ) {
        signal.add(1);
        self.shared
            .push(priority, Job::with_signal(Box::new(f), signal.clone()));
    }

    /// Schedules `f` once `after` reaches zero (a continuation: nothing blocks).
    pub fn spawn_after<F: FnOnce() + Send + 'static>(
        &self,
        after: &Counter,
        priority: Priority,
        f: F,
    ) {
        after.then(Arc::clone(&self.shared), priority, Job::new(Box::new(f)));
    }

    /// Continuation that is also counted on `signal`, for chaining stages.
    pub fn spawn_after_signal<F: FnOnce() + Send + 'static>(
        &self,
        after: &Counter,
        signal: &Counter,
        priority: Priority,
        f: F,
    ) {
        signal.add(1);
        after.then(
            Arc::clone(&self.shared),
            priority,
            Job::with_signal(Box::new(f), signal.clone()),
        );
    }

    /// Schedules `f` and returns a handle to its result.
    pub fn spawn_task<T, F>(&self, priority: Priority, f: F) -> Task<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (task, slot, done) = Task::pending();
        done.add(1);
        let job = Job::with_signal(
            Box::new(move || {
                let result = catch_unwind(AssertUnwindSafe(f));
                *slot.lock() = Some(result);
            }),
            done,
        );
        self.shared.push(priority, job);
        task
    }

    /// Runs `f` on the blocking pool (file and network I/O, anything that may sleep).
    pub fn spawn_blocking<T, F>(&self, f: F) -> Task<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        self.blocking.spawn(f)
    }

    /// Fork-join: jobs spawned on the scope may borrow from the enclosing stack frame. Returns
    /// once every job has finished; the calling thread runs jobs while it waits. A panic in
    /// any job is re-raised here after all jobs have completed.
    pub fn scope<'scope, F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Scope<'scope>) -> R,
    {
        let scope = Scope::new(Arc::clone(&self.shared));
        let result = catch_unwind(AssertUnwindSafe(|| f(&scope)));
        self.wait(scope.counter());
        scope.propagate_panic();
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Waits for `counter` to reach zero, running other jobs meanwhile.
    pub fn wait(&self, counter: &Counter) {
        if counter.is_zero() {
            return;
        }
        self.shared.help_until(|| counter.is_zero(), Some(counter));
    }

    /// Runs queued jobs on the calling thread until every queue is empty. Useful in tests
    /// and for zero-worker pools.
    pub fn drain(&self) {
        while let Some(job) = self.shared.find_work_from_current_thread() {
            run_job(job, None);
            self.shared.helped_jobs.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Snapshot of the counters.
    pub fn stats(&self) -> PoolStats {
        PoolStats {
            workers: self
                .shared
                .stats
                .iter()
                .map(|s| WorkerStatsSnapshot {
                    jobs: s.jobs.load(Ordering::Relaxed),
                    steals: s.steals.load(Ordering::Relaxed),
                    parks: s.parks.load(Ordering::Relaxed),
                    panics: s.panics.load(Ordering::Relaxed),
                })
                .collect(),
            helped_jobs: self.shared.helped_jobs.load(Ordering::Relaxed),
        }
    }
}

impl Drop for TaskPool {
    /// Finishes queued jobs, then stops the workers. Drop the pool from a thread that is not
    /// itself waiting on a counter served by this pool.
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        self.drain();
        self.shared.wake_all();
        for handle in self.threads.lock().drain(..) {
            let _ = handle.join();
        }
    }
}
