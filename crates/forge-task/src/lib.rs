//! Forge job system.
//!
//! The engine runs on a fixed set of worker threads that pull jobs from work-stealing deques
//! (Chase–Lev, via `crossbeam-deque`). There are three priorities, dependency **counters**
//! that fire continuations instead of blocking, fork-join **scopes** that may borrow stack data,
//! a **task graph** for per-frame DAGs, and a separate **blocking pool** for file and network
//! I/O so a stalled read never occupies a compute worker.
//!
//! Design rules (see `docs/research/task-system.md`):
//! - Workers never block on a dependency: a job that needs another job's result is expressed
//!   as a continuation (`spawn_after`) or as a scope that *helps* — runs other jobs — while it
//!   waits. This is the continuation model of Frostbite / Destiny rather than fibers.
//! - The pool leaves cores free for the main, render and audio threads
//!   ([`PoolConfig::client`]): a worker on every core is a worker too many.
//! - Determinism never depends on scheduling: parallel work merges results in a fixed order
//!   ([`TaskPool::par_chunks_mut`] and friends index by position, never by completion order).
//!
//! # Example
//! ```
//! use forge_task::{PoolConfig, TaskPool};
//!
//! let pool = TaskPool::new(PoolConfig::with_workers(2));
//! let mut squares = vec![0_u64; 1000];
//! pool.par_chunks_mut(&mut squares, 64, |chunk_index, chunk| {
//!     for (k, value) in chunk.iter_mut().enumerate() {
//!         let i = (chunk_index * 64 + k) as u64;
//!         *value = i * i;
//!     }
//! });
//! assert_eq!(squares[10], 100);
//! ```

// `unsafe` is confined to `scope.rs`, where borrowed closures are lifetime-extended under the
// guarantee that the scope waits for every job before returning (the `rayon` / `std::thread::scope`
// pattern). Every block carries a `SAFETY:` comment.
#![allow(unsafe_code)]

mod blocking;
mod counter;
mod graph;
mod job;
mod par;
mod pool;
mod scope;
mod task;

pub use blocking::BlockingPool;
pub use counter::Counter;
pub use graph::{GraphError, GraphRun, NodeId, TaskGraph};
pub use pool::{PoolConfig, PoolStats, TaskPool, WorkerStatsSnapshot};
pub use scope::Scope;
pub use task::Task;

/// Scheduling priority. Workers drain `High` before `Normal` before `Low`, both in their own
/// queue and when stealing.
#[repr(usize)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Priority {
    /// Latency-critical frame work (culling, command recording, input-driven simulation).
    High = 0,
    /// Ordinary frame work.
    #[default]
    Normal = 1,
    /// Background work that may take many frames (chunk generation, asset decoding).
    Low = 2,
}

/// Number of priority levels.
pub const PRIORITY_COUNT: usize = 3;
