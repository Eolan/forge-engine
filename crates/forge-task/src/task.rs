use std::panic::resume_unwind;
use std::sync::Arc;
use std::thread;

use parking_lot::Mutex;

use crate::counter::Counter;
use crate::pool::TaskPool;

/// Result slot shared between a task and its handle.
pub(crate) type Slot<T> = Arc<Mutex<Option<thread::Result<T>>>>;

/// Handle to the result of [`TaskPool::spawn_task`] or [`TaskPool::spawn_blocking`].
pub struct Task<T> {
    slot: Slot<T>,
    done: Counter,
}

impl<T: Send + 'static> Task<T> {
    pub(crate) fn pending() -> (Self, Slot<T>, Counter) {
        let slot: Slot<T> = Arc::new(Mutex::new(None));
        let done = Counter::new();
        (
            Self {
                slot: Arc::clone(&slot),
                done: done.clone(),
            },
            slot,
            done,
        )
    }

    /// Whether the result is available.
    pub fn is_done(&self) -> bool {
        self.done.is_zero()
    }

    /// The completion counter, for continuations.
    pub fn counter(&self) -> &Counter {
        &self.done
    }

    /// Waits for the result, running other jobs of `pool` meanwhile. Re-raises the task's
    /// panic if it had one.
    pub fn wait(self, pool: &TaskPool) -> T {
        pool.wait(&self.done);
        self.take()
    }

    /// Waits without helping (from a thread that must not run jobs, e.g. the audio thread).
    pub fn wait_blocking(self) -> T {
        self.done.wait_blocking();
        self.take()
    }

    /// Takes the result if it is ready.
    pub fn try_take(self) -> Result<T, Self> {
        if self.is_done() {
            Ok(self.take())
        } else {
            Err(self)
        }
    }

    fn take(self) -> T {
        match self.slot.lock().take() {
            Some(Ok(value)) => value,
            Some(Err(payload)) => resume_unwind(payload),
            None => unreachable!("task counter reached zero before its result was stored"),
        }
    }
}
