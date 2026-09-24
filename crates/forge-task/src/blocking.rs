use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::job::Job;
use crate::task::Task;

/// A plain FIFO thread pool for work that blocks: file reads, database calls, DNS…
///
/// Kept apart from the compute workers so that a stalled read never steals a core from the
/// frame. Threads sleep on the channel when idle.
pub struct BlockingPool {
    sender: Option<Sender<Job>>,
    threads: Vec<JoinHandle<()>>,
}

impl BlockingPool {
    /// Starts `threads` threads named `"{name}-{index}"`.
    pub fn new(threads: usize, name: &str) -> Self {
        let (sender, receiver) = unbounded::<Job>();
        let threads = (0..threads.max(1))
            .map(|index| {
                let receiver: Receiver<Job> = receiver.clone();
                let thread_name = format!("{name}-{index}");
                thread::Builder::new()
                    .name(thread_name.clone())
                    .spawn(move || {
                        #[cfg(feature = "profiling")]
                        if let Some(client) = tracy_client::Client::running() {
                            client.set_thread_name(&thread_name);
                        }
                        for job in receiver.iter() {
                            if catch_unwind(AssertUnwindSafe(|| job.run())).is_err() {
                                tracing::error!("blocking job panicked; the thread continues");
                            }
                        }
                    })
                    .expect("spawn blocking thread")
            })
            .collect();
        Self {
            sender: Some(sender),
            threads,
        }
    }

    /// Queues `f` and returns a handle to its result.
    pub fn spawn<T, F>(&self, f: F) -> Task<T>
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
        self.sender
            .as_ref()
            .expect("blocking pool alive")
            .send(job)
            .expect("blocking thread alive");
        task
    }

    /// Number of threads.
    pub fn thread_count(&self) -> usize {
        self.threads.len()
    }
}

impl Drop for BlockingPool {
    /// Finishes queued jobs, then joins the threads.
    fn drop(&mut self) {
        drop(self.sender.take());
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
    }
}
