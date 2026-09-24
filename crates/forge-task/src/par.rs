//! Data-parallel helpers built on scopes. Work is split recursively (the left half runs on
//! the calling thread, the right half is spawned), which keeps stealing coarse: an idle
//! worker takes the oldest, largest piece.

use std::ops::Range;

use crate::pool::TaskPool;
use crate::scope::Scope;

impl TaskPool {
    /// Runs `a` and `b` in parallel and returns both results.
    pub fn join<A, B, RA, RB>(&self, a: A, b: B) -> (RA, RB)
    where
        A: FnOnce() -> RA + Send,
        B: FnOnce() -> RB + Send,
        RA: Send,
        RB: Send,
    {
        let mut result_b = None;
        let result_a = self.scope(|scope| {
            scope.spawn(|_| result_b = Some(b()));
            a()
        });
        (
            result_a,
            result_b.expect("join: second closure did not run"),
        )
    }

    /// Calls `f(i)` for every `i` in `range`, in pieces of at most `grain` iterations.
    pub fn par_for<F>(&self, range: Range<usize>, grain: usize, f: F)
    where
        F: Fn(usize) + Sync,
    {
        let grain = grain.max(1);
        self.scope(|scope| split_range(scope, range, grain, &f));
    }

    /// Calls `f(chunk_index, chunk)` for every chunk of `chunk_len` elements of `data`.
    ///
    /// Results are indexed by position, so the outcome is independent of scheduling.
    pub fn par_chunks_mut<T, F>(&self, data: &mut [T], chunk_len: usize, f: F)
    where
        T: Send,
        F: Fn(usize, &mut [T]) + Sync,
    {
        let chunk_len = chunk_len.max(1);
        self.scope(|scope| split_chunks(scope, data, 0, chunk_len, &f));
    }

    /// Fills `out[i] = f(i)` in parallel.
    pub fn par_map_into<T, F>(&self, out: &mut [T], grain: usize, f: F)
    where
        T: Send,
        F: Fn(usize) -> T + Sync,
    {
        let grain = grain.max(1);
        self.par_chunks_mut(out, grain, |chunk_index, chunk| {
            let base = chunk_index * grain;
            for (k, slot) in chunk.iter_mut().enumerate() {
                *slot = f(base + k);
            }
        });
    }
}

fn split_range<'scope, F>(scope: &Scope<'scope>, range: Range<usize>, grain: usize, f: &'scope F)
where
    F: Fn(usize) + Sync,
{
    let len = range.len();
    if len <= grain {
        for i in range {
            f(i);
        }
        return;
    }
    let mid = range.start + len / 2;
    let right = mid..range.end;
    scope.spawn(move |scope| split_range(scope, right, grain, f));
    split_range(scope, range.start..mid, grain, f);
}

fn split_chunks<'scope, T, F>(
    scope: &Scope<'scope>,
    data: &'scope mut [T],
    first_chunk: usize,
    chunk_len: usize,
    f: &'scope F,
) where
    T: Send,
    F: Fn(usize, &mut [T]) + Sync,
{
    let chunks = data.len().div_ceil(chunk_len);
    if chunks <= 1 {
        if !data.is_empty() {
            f(first_chunk, data);
        }
        return;
    }
    let left_chunks = chunks / 2;
    let (left, right) = data.split_at_mut(left_chunks * chunk_len);
    scope.spawn(move |scope| split_chunks(scope, right, first_chunk + left_chunks, chunk_len, f));
    split_chunks(scope, left, first_chunk, chunk_len, f);
}
