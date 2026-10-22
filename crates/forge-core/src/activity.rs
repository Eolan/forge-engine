//! What the engine is busy with: tasks any system begins and ends on any thread, and the steps
//! it finished (the owner, 2026-10-21: long loads should say what they load). A system begins a
//! [`Task`] with its name and a label ("shaders", "meshlet.slang resolve_main"), may give its
//! count done, and the task ends when the guard drops; a step finished without a task of its own
//! is a [`note`]. Whoever wants to know reads a [`snapshot`]: the loading screen shows the
//! longest tasks running and the last steps done.
//!
//! Nothing here touches the simulation or what the world generates (the crate's rule): it is
//! the program's own account of its time, read by its screens and its logs.
//!
//! A load can be profiled (#345, the owner: "we will add a bit of profiling on the loadings"):
//! between [`record_start`] and [`record_finish`] every task ended and every note is kept with
//! its thread, its times and the bytes its thread uploaded to the GPU ([`uploaded`]), for a
//! [`Profile`]'s report. Development builds only: the `shipping` feature compiles the recording
//! out.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread::ThreadId;
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
    /// The thread it began on, and that thread's bytes uploaded then.
    thread: ThreadId,
    uploaded: u64,
}

struct State {
    next: u64,
    running: Vec<Entry>,
    done: VecDeque<(&'static str, String, Option<Duration>, Instant)>,
    /// The load being profiled ([`record_start`]).
    recording: Option<Recording>,
}

/// A load being profiled: when it started, the steps kept, the threads seen.
struct Recording {
    started: Instant,
    steps: Vec<Recorded>,
    threads: Vec<(ThreadId, String)>,
}

impl Recording {
    /// Thread `id`'s number, given in the order the threads first recorded.
    fn thread(&mut self, id: ThreadId) -> u32 {
        if let Some(n) = self.threads.iter().position(|(t, _)| *t == id) {
            return n as u32;
        }
        let current = std::thread::current();
        let name = if current.id() == id {
            current.name().unwrap_or("unnamed").to_owned()
        } else {
            format!("{id:?}")
        };
        self.threads.push((id, name));
        self.threads.len() as u32 - 1
    }

    /// Keeps a step that ended `took` ago at `ended`.
    fn keep(
        &mut self,
        system: &'static str,
        label: &str,
        thread: ThreadId,
        ended: Instant,
        took: Duration,
        uploaded: u64,
    ) {
        let end = ended.saturating_duration_since(self.started);
        let thread = self.thread(thread);
        self.steps.push(Recorded {
            system,
            label: label.to_owned(),
            thread,
            start: end.saturating_sub(took),
            took,
            uploaded,
        });
    }
}

static STATE: Mutex<State> = Mutex::new(State {
    next: 0,
    running: Vec::new(),
    done: VecDeque::new(),
    recording: None,
});

thread_local! {
    /// Bytes this thread uploaded to the GPU ([`uploaded`]).
    static UPLOADED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// `bytes` this thread uploaded to the GPU (a staged copy, an image's levels): counted into the
/// tasks it runs while a load is profiled.
pub fn uploaded(bytes: u64) {
    UPLOADED.with(|u| u.set(u.get() + bytes));
}

/// This thread's bytes uploaded so far.
fn uploaded_here() -> u64 {
    UPLOADED.with(std::cell::Cell::get)
}

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
            if let Some(r) = s.recording.as_mut() {
                // The bytes its thread uploaded meanwhile, when it ends where it began.
                let here = std::thread::current().id() == e.thread;
                let bytes = if here {
                    uploaded_here().saturating_sub(e.uploaded)
                } else {
                    0
                };
                r.keep(e.system, &e.label, e.thread, Instant::now(), took, bytes);
            }
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
        thread: std::thread::current().id(),
        uploaded: uploaded_here(),
    });
    Task { id }
}

/// A step of `system` done, `label` saying what, `took` how long when known: one without a task
/// of its own (a phase a timer measured).
pub fn note(system: &'static str, label: impl Into<String>, took: Option<Duration>) {
    let label = label.into();
    let mut s = state();
    if let (Some(r), Some(took)) = (s.recording.as_mut(), took) {
        r.keep(
            system,
            &label,
            std::thread::current().id(),
            Instant::now(),
            took,
            0,
        );
    }
    push_done(&mut s, system, label, took);
}

/// Starts profiling a load (#345): every task ended and every note with a time from now on is
/// kept until [`record_finish`]. Nothing in a `shipping` build.
pub fn record_start() {
    if cfg!(feature = "shipping") {
        return;
    }
    state().recording = Some(Recording {
        started: Instant::now(),
        steps: Vec::new(),
        threads: Vec::new(),
    });
}

/// Whether a load is being profiled.
pub fn recording() -> bool {
    state().recording.is_some()
}

/// Ends the profile [`record_start`] began: its steps, in the order they ended. `None` when none
/// was begun.
pub fn record_finish() -> Option<Profile> {
    let r = state().recording.take()?;
    Some(Profile {
        wall: r.started.elapsed(),
        steps: r.steps,
        threads: r.threads.into_iter().map(|(_, name)| name).collect(),
    })
}

/// A step kept while a load was profiled ([`record_start`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Recorded {
    /// The system that did it.
    pub system: &'static str,
    /// What it was.
    pub label: String,
    /// Its thread, numbered from 0 in the order the threads first recorded ([`Profile::threads`]).
    pub thread: u32,
    /// When it began, from the recording's start.
    pub start: Duration,
    /// How long it took.
    pub took: Duration,
    /// Bytes its thread uploaded to the GPU while it ran ([`uploaded`]), those of the tasks
    /// inside it too.
    pub uploaded: u64,
}

/// A load profiled ([`record_finish`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    /// From [`record_start`] to [`record_finish`].
    pub wall: Duration,
    /// Every task ended and every note with a time, in the order they ended.
    pub steps: Vec<Recorded>,
    /// The threads' names, by their number.
    pub threads: Vec<String>,
}

/// The profiles [`Profile::save`] keeps, the latest.
const PROFILES_KEPT: usize = 10;
/// The oldest profile [`Profile::save`] keeps.
const PROFILES_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
/// The slowest steps a report names.
const REPORT_SLOWEST: usize = 15;

impl Profile {
    /// Whether step `k` lies inside another step of its thread (a task's inner task): its time
    /// and bytes are that step's already.
    fn nested(&self, k: usize) -> bool {
        let s = &self.steps[k];
        let end = s.start + s.took;
        self.steps.iter().enumerate().any(|(j, o)| {
            j != k
                && o.thread == s.thread
                && o.start <= s.start
                && o.start + o.took >= end
                && (o.took > s.took || j > k)
        })
    }

    /// The time the steps chosen by `keep` cover on each thread, their intervals merged, summed
    /// over the threads.
    fn busy(&self, keep: impl Fn(&Recorded) -> bool) -> Duration {
        let mut total = Duration::ZERO;
        for thread in 0..self.threads.len() as u32 {
            let mut spans: Vec<(Duration, Duration)> = self
                .steps
                .iter()
                .filter(|s| s.thread == thread && keep(s))
                .map(|s| (s.start, s.start + s.took))
                .collect();
            spans.sort();
            let mut at = Duration::ZERO;
            for (a, b) in spans {
                let a = a.max(at);
                if b > a {
                    total += b - a;
                    at = b;
                }
            }
        }
        total
    }

    /// Bytes uploaded over the load: the outermost steps' on each thread.
    pub fn uploaded(&self) -> u64 {
        (0..self.steps.len())
            .filter(|&k| !self.nested(k))
            .map(|k| self.steps[k].uploaded)
            .sum()
    }

    /// The report: the load's time and its threads' busy time, then by system, the slowest
    /// steps, and each thread's.
    pub fn report(&self) -> String {
        use std::fmt::Write;
        let secs = |d: Duration| d.as_secs_f64();
        let busy = self.busy(|_| true);
        let mut out = String::new();
        let _ = writeln!(
            out,
            "load profile: {:.2} s, {} threads busy {:.2} s ({:.1} at once on average), {} uploaded",
            secs(self.wall),
            self.threads.len(),
            secs(busy),
            secs(busy) / secs(self.wall).max(1e-6),
            bytes(self.uploaded())
        );
        let mut systems: Vec<&'static str> = self.steps.iter().map(|s| s.system).collect();
        systems.sort_unstable();
        systems.dedup();
        let mut rows: Vec<(Duration, &'static str, usize, u64)> = systems
            .into_iter()
            .map(|system| {
                let steps = self.steps.iter().filter(|s| s.system == system).count();
                let uploaded = (0..self.steps.len())
                    .filter(|&k| self.steps[k].system == system && !self.nested(k))
                    .map(|k| self.steps[k].uploaded)
                    .sum();
                (self.busy(|s| s.system == system), system, steps, uploaded)
            })
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        let _ = writeln!(out, "by system (busy, steps, uploaded):");
        for (time, system, steps, uploaded) in rows {
            let _ = writeln!(
                out,
                "  {system:<12} {:>8.2} s {steps:>6} {:>10}",
                secs(time),
                bytes(uploaded)
            );
        }
        let mut slowest: Vec<&Recorded> = self.steps.iter().collect();
        slowest.sort_by_key(|s| std::cmp::Reverse(s.took));
        let _ = writeln!(out, "slowest steps (took, from, system, thread, uploaded):");
        for s in slowest.into_iter().take(REPORT_SLOWEST) {
            let _ = writeln!(
                out,
                "  {:>7.2} s at {:>6.2} s  {:<10} {:<14} {:>10}  {}",
                secs(s.took),
                secs(s.start),
                s.system,
                self.threads
                    .get(s.thread as usize)
                    .map_or("?", String::as_str),
                bytes(s.uploaded),
                s.label
            );
        }
        let _ = writeln!(out, "threads (busy, first step to last):");
        for (n, name) in self.threads.iter().enumerate() {
            let mine = || self.steps.iter().filter(|s| s.thread == n as u32);
            let first = mine().map(|s| s.start).min().unwrap_or_default();
            let last = mine().map(|s| s.start + s.took).max().unwrap_or_default();
            let _ = writeln!(
                out,
                "  {name:<16} {:>8.2} s  {:.2}..{:.2} s",
                secs(self.busy(|s| s.thread == n as u32)),
                secs(first),
                secs(last)
            );
        }
        out
    }

    /// Writes the report into `dir` as `load-PROGRAM-SECONDS.txt` and returns its path, keeping
    /// there the [`PROFILES_KEPT`] latest no older than a week (the owner: no gigabytes of
    /// obsolete data); `dir` is the cache's `profiles/` ([`crate::derived::CacheKind::Profiles`]).
    pub fn save(&self, dir: &std::path::Path, program: &str) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let now = std::time::SystemTime::now();
        let stamp = now
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let path = dir.join(format!("load-{program}-{stamp}.txt"));
        std::fs::write(&path, self.report())?;
        let mut kept: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                *p != path
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("load-") && n.ends_with(".txt"))
            })
            .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
            .collect();
        // The others, the latest first: this one and those that fit beside it stay.
        kept.sort_by(|a, b| b.cmp(a));
        for (k, (modified, old)) in kept.iter().enumerate() {
            let stale = now.duration_since(*modified).unwrap_or_default() > PROFILES_AGE;
            if k + 1 >= PROFILES_KEPT || stale {
                let _ = std::fs::remove_file(old);
            }
        }
        Ok(path)
    }
}

/// `n` bytes as B, KiB, MiB or GiB.
fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
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

    #[test]
    fn a_profiled_load_keeps_its_steps_threads_and_uploads_once() {
        record_start();
        {
            let outer = begin("test-profile", "outer");
            uploaded(100);
            {
                let _inner = begin("test-profile", "inner");
                uploaded(1000);
            }
            std::thread::Builder::new()
                .name("test-profile worker".to_owned())
                .spawn(|| {
                    let _task = begin("test-profile", "on a worker");
                    uploaded(10);
                })
                .expect("a thread")
                .join()
                .expect("the worker");
            drop(outer);
        }
        let profile = record_finish().expect("a profile");
        assert!(record_finish().is_none());
        let mine: Vec<&Recorded> = profile
            .steps
            .iter()
            .filter(|s| s.system == "test-profile")
            .collect();
        let labels: Vec<&str> = mine.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["inner", "on a worker", "outer"]);
        assert_eq!(mine[0].uploaded, 1000);
        assert_eq!(mine[1].uploaded, 10);
        // The outer task's bytes hold its inner task's.
        assert_eq!(mine[2].uploaded, 1100);
        assert_ne!(mine[1].thread, mine[2].thread);
        assert_eq!(
            profile.threads[mine[1].thread as usize],
            "test-profile worker"
        );
        // Another test's tasks may run beside: the total holds at least these, each once.
        assert!(profile.uploaded() >= 1110);
        let report = profile.report();
        assert!(report.contains("test-profile"));
        assert!(report.starts_with("load profile:"));
    }

    #[test]
    fn saved_profiles_are_bounded() {
        let dir = std::env::temp_dir().join(format!("forge-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let profile = Profile::default();
        for k in 0..PROFILES_KEPT + 3 {
            // The stamp is in seconds: a name of its own each.
            profile
                .save(&dir, &format!("test{k}"))
                .expect("a profile saved");
        }
        let left = std::fs::read_dir(&dir).expect("the directory").count();
        assert_eq!(left, PROFILES_KEPT);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
