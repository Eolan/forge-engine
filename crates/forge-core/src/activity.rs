//! What the engine is busy with: tasks any system begins and ends on any thread, and the steps
//! it finished (the owner, 2026-10-21: long loads should say what they load). A system begins a
//! [`Task`] with its name and a label ("shaders", "meshlet.slang resolve_main"), may give its
//! count done, and the task ends when the guard drops; a step finished without a task of its own
//! is a [`note`]. Whoever wants to know reads a [`snapshot`]: the loading screen shows the
//! longest tasks running and the last steps done.
//!
//! Nothing here touches the simulation or what the world generates (the crate's rule): it is
//! the program's own account of its time, read by its screens and its logs.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Steps done the snapshot keeps of each system, the latest last.
const RECENT: usize = 16;

/// A task running ([`begin`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Running {
    /// The system that runs it ("shaders", "scene", "cooking").
    pub system: &'static str,
    /// What it is.
    pub label: String,
    /// How long it has run.
    pub elapsed: Duration,
    /// Its count done and its total, when it gives them ([`Task::progress`]).
    pub progress: Option<(u64, u64)>,
}

/// A step done: a task ended, or a [`note`].
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    /// The system that did it.
    pub system: &'static str,
    /// What it was.
    pub label: String,
    /// How long it took (none for a note given without).
    pub took: Option<Duration>,
    /// How long ago it ended.
    pub ago: Duration,
}

/// What the engine is busy with ([`snapshot`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    /// The tasks running, the longest first.
    pub running: Vec<Running>,
    /// The last steps done, the latest last.
    pub done: Vec<Done>,
}

impl Activity {
    /// The tasks of `system` running, the longest first.
    pub fn running_in<'a>(&'a self, system: &'a str) -> impl Iterator<Item = &'a Running> {
        self.running.iter().filter(move |r| r.system == system)
    }
}

struct Entry {
    id: u64,
    system: &'static str,
    label: String,
    started: Instant,
    progress: Option<(u64, u64)>,
}

struct State {
    next: u64,
    running: Vec<Entry>,
    done: VecDeque<(&'static str, String, Option<Duration>, Instant)>,
}

static STATE: Mutex<State> = Mutex::new(State {
    next: 0,
    running: Vec::new(),
    done: VecDeque::new(),
});

/// The state, whatever a thread that panicked holding it left.
fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A task running until it drops ([`begin`]).
#[must_use = "the task ends when dropped"]
pub struct Task {
    id: u64,
}

impl Task {
    /// Its count done of `total`.
    pub fn progress(&self, done: u64, total: u64) {
        if let Some(e) = state().running.iter_mut().find(|e| e.id == self.id) {
            e.progress = Some((done, total));
        }
    }

    /// Renames it (a step of a longer task).
    pub fn relabel(&self, label: impl Into<String>) {
        if let Some(e) = state().running.iter_mut().find(|e| e.id == self.id) {
            e.label = label.into();
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        let mut s = state();
        if let Some(i) = s.running.iter().position(|e| e.id == self.id) {
            let e = s.running.swap_remove(i);
            let took = e.started.elapsed();
            push_done(&mut s, e.system, e.label, Some(took));
        }
    }
}

/// Keeps the step done, and of its system's the last [`RECENT`]: a system that does many (the
/// shaders, a hundred compiles) does not push the others' out.
fn push_done(s: &mut State, system: &'static str, label: String, took: Option<Duration>) {
    if s.done.iter().filter(|d| d.0 == system).count() >= RECENT
        && let Some(oldest) = s.done.iter().position(|d| d.0 == system)
    {
        s.done.remove(oldest);
    }
    s.done.push_back((system, label, took, Instant::now()));
}

/// Begins a task of `system`, `label` saying what: it runs until the returned guard drops.
pub fn begin(system: &'static str, label: impl Into<String>) -> Task {
    let mut s = state();
    let id = s.next;
    s.next += 1;
    s.running.push(Entry {
        id,
        system,
        label: label.into(),
        started: Instant::now(),
        progress: None,
    });
    Task { id }
}

/// A step of `system` done, `label` saying what, `took` how long when known: one without a task
/// of its own (a phase a timer measured).
pub fn note(system: &'static str, label: impl Into<String>, took: Option<Duration>) {
    push_done(&mut state(), system, label.into(), took);
}

/// What the engine is busy with now.
pub fn snapshot() -> Activity {
    let s = state();
    let now = Instant::now();
    let mut running: Vec<Running> = s
        .running
        .iter()
        .map(|e| Running {
            system: e.system,
            label: e.label.clone(),
            elapsed: now - e.started,
            progress: e.progress,
        })
        .collect();
    running.sort_by_key(|r| std::cmp::Reverse(r.elapsed));
    let done = s
        .done
        .iter()
        .map(|(system, label, took, at)| Done {
            system,
            label: label.clone(),
            took: *took,
            ago: now - *at,
        })
        .collect();
    Activity { running, done }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_run_until_dropped_and_are_kept_as_done() {
        let outer = begin("test-activity", "outer");
        {
            let inner = begin("test-activity", "inner");
            inner.progress(3, 10);
            let now = snapshot();
            let mine: Vec<_> = now.running_in("test-activity").collect();
            assert_eq!(mine.len(), 2);
            // The longest first: the outer began before.
            assert_eq!(mine[0].label, "outer");
            assert_eq!(mine[1].progress, Some((3, 10)));
        }
        note("test-activity", "a phase", Some(Duration::from_millis(5)));
        drop(outer);
        let now = snapshot();
        assert_eq!(now.running_in("test-activity").count(), 0);
        let done: Vec<_> = now
            .done
            .iter()
            .filter(|d| d.system == "test-activity")
            .map(|d| d.label.as_str())
            .collect();
        assert_eq!(done, ["inner", "a phase", "outer"]);
    }
}
