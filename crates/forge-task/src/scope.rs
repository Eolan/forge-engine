use std::any::Any;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::Priority;
use crate::counter::Counter;
use crate::job::Job;
use crate::pool::Shared;

/// A fork-join scope created by [`crate::TaskPool::scope`].
///
/// Jobs spawned here may borrow data that outlives `'scope`. The scope does not return until
/// every job has finished, which is what makes those borrows sound.
pub struct Scope<'scope> {
    shared: Arc<Shared>,
    counter: Counter,
    panic: Mutex<Option<Box<dyn Any + Send>>>,
    _marker: PhantomData<fn(&'scope ()) -> &'scope ()>,
}

/// Raw pointer to the scope, moved into each job.
struct ScopePtr<'scope>(*const Scope<'scope>);

// SAFETY: `Scope` is `Sync` (an `Arc`, a `Counter` and a `Mutex` of a `Send` payload) and
// `TaskPool::scope` keeps it alive until every job that holds this pointer has finished.
unsafe impl Send for ScopePtr<'_> {}

impl<'scope> ScopePtr<'scope> {
    /// Taking `self` by value makes the closure capture the whole (Send) wrapper rather than
    /// its raw-pointer field, which edition-2021 disjoint capture would otherwise pick.
    fn get(self) -> *const Scope<'scope> {
        self.0
    }
}

impl<'scope> Scope<'scope> {
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            counter: Counter::new(),
            panic: Mutex::new(None),
            _marker: PhantomData,
        }
    }

    /// The counter of outstanding jobs in this scope.
    pub fn counter(&self) -> &Counter {
        &self.counter
    }

    /// Spawns a job at [`Priority::Normal`].
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce(&Scope<'scope>) + Send + 'scope,
    {
        self.spawn_with_priority(Priority::Normal, f);
    }

    /// Spawns a job at the given priority.
    pub fn spawn_with_priority<F>(&self, priority: Priority, f: F)
    where
        F: FnOnce(&Scope<'scope>) + Send + 'scope,
    {
        self.counter.add(1);
        let ptr = ScopePtr(self as *const Self);
        let job: Box<dyn FnOnce() + Send + 'scope> = Box::new(move || {
            let raw = ptr.get();
            // SAFETY: `TaskPool::scope` waits for `counter` to reach zero — which this job's
            // signal guard decrements only after this closure returns — before the `Scope` is
            // dropped, so `raw` points to a live `Scope` for the whole run of the job.
            let scope: &Scope<'scope> = unsafe { &*raw };
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| f(scope))) {
                scope.record_panic(payload);
            }
        });
        // SAFETY: lifetime extension from `'scope` to `'static`. The job can only run while
        // `TaskPool::scope` is blocked in its helping wait, and that wait ends only when every
        // job in the scope has completed; every borrow captured by `f` therefore outlives its
        // last use. This is the same argument as `std::thread::scope` and `rayon::scope`.
        let job: Box<dyn FnOnce() + Send + 'static> = unsafe {
            std::mem::transmute::<
                Box<dyn FnOnce() + Send + 'scope>,
                Box<dyn FnOnce() + Send + 'static>,
            >(job)
        };
        self.shared
            .push(priority, Job::with_signal(job, self.counter.clone()));
    }

    fn record_panic(&self, payload: Box<dyn Any + Send>) {
        let mut slot = self.panic.lock();
        if slot.is_none() {
            *slot = Some(payload);
        }
    }

    pub(crate) fn propagate_panic(&self) {
        if let Some(payload) = self.panic.lock().take() {
            resume_unwind(payload);
        }
    }
}
