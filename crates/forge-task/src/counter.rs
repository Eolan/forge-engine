use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

use crate::Priority;
use crate::job::Job;
use crate::pool::Shared;

/// A continuation registered with [`Counter::then`], released when the counter reaches zero.
struct Waiter {
    pool: Arc<Shared>,
    priority: Priority,
    job: Job,
}

struct Inner {
    value: AtomicU32,
    waiters: Mutex<Vec<Waiter>>,
    sleep: Mutex<()>,
    wake: Condvar,
}

/// A dependency counter (the Naughty Dog / Destiny primitive).
///
/// Jobs increment it when they are scheduled and decrement it when they finish. Anything can
/// wait for it to reach zero — either by *helping* ([`crate::TaskPool::wait`]) or by
/// registering a continuation ([`crate::TaskPool::spawn_after`]) that is scheduled the moment
/// the count hits zero. Counters are cheap, clonable handles and may be reused across frames.
#[derive(Clone)]
pub struct Counter(Arc<Inner>);

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

impl Counter {
    /// A counter at zero.
    pub fn new() -> Self {
        Self::with_value(0)
    }

    /// A counter starting at `value`.
    pub fn with_value(value: u32) -> Self {
        Self(Arc::new(Inner {
            value: AtomicU32::new(value),
            waiters: Mutex::new(Vec::new()),
            sleep: Mutex::new(()),
            wake: Condvar::new(),
        }))
    }

    /// Current count.
    pub fn value(&self) -> u32 {
        self.0.value.load(Ordering::Acquire)
    }

    /// Whether the count is zero.
    pub fn is_zero(&self) -> bool {
        self.value() == 0
    }

    /// Adds `n` pending completions.
    pub fn add(&self, n: u32) {
        self.0.value.fetch_add(n, Ordering::AcqRel);
    }

    /// Records one completion. Releases continuations and wakes blocked waiters on zero.
    ///
    /// # Panics
    /// If the counter is already zero (a completion without a matching [`Self::add`]).
    pub fn decrement(&self) {
        let previous = self.0.value.fetch_sub(1, Ordering::AcqRel);
        assert!(previous > 0, "Counter decremented below zero");
        if previous == 1 {
            self.release();
        }
    }

    fn release(&self) {
        let waiters = std::mem::take(&mut *self.0.waiters.lock());
        for waiter in waiters {
            waiter.pool.push(waiter.priority, waiter.job);
        }
        // Taking the lock orders this wake after any waiter's check-then-sleep.
        let _guard = self.0.sleep.lock();
        self.0.wake.notify_all();
    }

    /// Schedules `job` on `pool` once the count reaches zero (immediately if it already is).
    pub(crate) fn then(&self, pool: Arc<Shared>, priority: Priority, job: Job) {
        {
            // The value is read under the waiters lock so `release` (which takes the same
            // lock after its decrement) cannot slip between the check and the push.
            let mut waiters = self.0.waiters.lock();
            if self.value() != 0 {
                waiters.push(Waiter {
                    pool,
                    priority,
                    job,
                });
                return;
            }
        }
        pool.push(priority, job);
    }

    /// Blocks the calling thread (without helping) until the count is zero.
    ///
    /// Prefer [`crate::TaskPool::wait`] from any thread that could run jobs itself.
    pub fn wait_blocking(&self) {
        let mut guard = self.0.sleep.lock();
        while self.value() != 0 {
            self.0.wake.wait(&mut guard);
        }
    }

    /// Blocks for at most `timeout`. Returns `true` if the count is zero.
    pub(crate) fn wait_timeout(&self, timeout: Duration) -> bool {
        let mut guard = self.0.sleep.lock();
        if self.value() == 0 {
            return true;
        }
        self.0.wake.wait_for(&mut guard, timeout);
        self.value() == 0
    }
}

impl fmt::Debug for Counter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Counter({})", self.value())
    }
}
