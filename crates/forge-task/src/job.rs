use crate::counter::Counter;

/// Type-erased unit of work.
pub(crate) type JobFn = Box<dyn FnOnce() + Send + 'static>;

/// A job plus the counter that must be decremented when it has run.
pub(crate) struct Job {
    func: JobFn,
    signal: Option<Counter>,
}

impl Job {
    pub(crate) fn new(func: JobFn) -> Self {
        Self { func, signal: None }
    }

    pub(crate) fn with_signal(func: JobFn, signal: Counter) -> Self {
        Self {
            func,
            signal: Some(signal),
        }
    }

    /// Runs the job. The signal counter is decremented even if the job panics, so a scope or
    /// graph waiting on it can never hang.
    pub(crate) fn run(self) {
        struct SignalGuard(Option<Counter>);
        impl Drop for SignalGuard {
            fn drop(&mut self) {
                if let Some(counter) = self.0.take() {
                    counter.decrement();
                }
            }
        }
        let _guard = SignalGuard(self.signal);
        (self.func)();
    }
}
