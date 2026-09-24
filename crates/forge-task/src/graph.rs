use std::any::Any;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::Priority;
use crate::counter::Counter;
use crate::job::{Job, JobFn};
use crate::pool::TaskPool;

/// Identifies a node of a [`TaskGraph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

impl NodeId {
    /// The id the `index`-th added node will have (nodes are numbered in insertion order).
    /// Lets a node reference one that is added later, for tests and for graph builders.
    pub const fn from_index(index: u32) -> Self {
        Self(index)
    }

    /// Insertion index of the node.
    pub const fn index(self) -> u32 {
        self.0
    }
}

struct Node {
    name: String,
    priority: Priority,
    deps: Vec<NodeId>,
    func: JobFn,
}

/// Errors detected when a graph is started.
#[derive(Debug, PartialEq, Eq)]
pub enum GraphError {
    /// A dependency refers to a node that is not in the graph.
    UnknownDependency {
        /// The node holding the bad dependency.
        node: NodeId,
        /// The missing dependency.
        dependency: NodeId,
    },
    /// The graph has a cycle; the named node is part of it.
    Cycle(String),
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownDependency { node, dependency } => {
                write!(f, "node {node:?} depends on unknown node {dependency:?}")
            }
            Self::Cycle(name) => write!(f, "task graph has a cycle through {name:?}"),
        }
    }
}

impl std::error::Error for GraphError {}

/// A directed acyclic graph of jobs, typically one per frame stage.
///
/// Each node becomes a job gated by a counter of its unfinished dependencies; finishing a
/// node decrements its dependents' gates, and a gate at zero releases the node as a
/// continuation. Nothing polls and nothing blocks.
#[derive(Default)]
pub struct TaskGraph {
    nodes: Vec<Node>,
}

impl TaskGraph {
    /// An empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a node at [`Priority::Normal`].
    pub fn add<F>(&mut self, name: impl Into<String>, deps: &[NodeId], f: F) -> NodeId
    where
        F: FnOnce() + Send + 'static,
    {
        self.add_with_priority(name, Priority::Normal, deps, f)
    }

    /// Adds a node with an explicit priority.
    pub fn add_with_priority<F>(
        &mut self,
        name: impl Into<String>,
        priority: Priority,
        deps: &[NodeId],
        f: F,
    ) -> NodeId
    where
        F: FnOnce() + Send + 'static,
    {
        let id = NodeId(u32::try_from(self.nodes.len()).expect("graph too large"));
        self.nodes.push(Node {
            name: name.into(),
            priority,
            deps: deps.to_vec(),
            func: Box::new(f),
        });
        id
    }

    /// Number of nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the graph has no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn validate(&self) -> Result<(), GraphError> {
        let n = self.nodes.len();
        let mut indegree = vec![0_usize; n];
        for (i, node) in self.nodes.iter().enumerate() {
            for dep in &node.deps {
                if dep.0 as usize >= n {
                    return Err(GraphError::UnknownDependency {
                        node: NodeId(i as u32),
                        dependency: *dep,
                    });
                }
                indegree[i] += 1;
            }
        }
        // Kahn's algorithm: every node must be reachable from the roots.
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, node) in self.nodes.iter().enumerate() {
            for dep in &node.deps {
                dependents[dep.0 as usize].push(i);
            }
        }
        let mut ready: Vec<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
        let mut visited = 0;
        while let Some(i) = ready.pop() {
            visited += 1;
            for &d in &dependents[i] {
                indegree[d] -= 1;
                if indegree[d] == 0 {
                    ready.push(d);
                }
            }
        }
        if visited != n {
            let culprit = (0..n)
                .find(|&i| indegree[i] > 0)
                .map(|i| self.nodes[i].name.clone());
            return Err(GraphError::Cycle(culprit.unwrap_or_default()));
        }
        Ok(())
    }

    /// Schedules the graph and returns without waiting.
    pub fn start(self, pool: &TaskPool) -> Result<GraphRun, GraphError> {
        self.validate()?;
        let n = self.nodes.len();
        let done = Counter::with_value(u32::try_from(n).expect("graph too large"));
        let panic: Arc<Mutex<Option<Box<dyn Any + Send>>>> = Arc::new(Mutex::new(None));
        let gates: Vec<Counter> = self
            .nodes
            .iter()
            .map(|node| Counter::with_value(node.deps.len() as u32))
            .collect();
        let mut dependents: Vec<Vec<Counter>> = vec![Vec::new(); n];
        for (i, node) in self.nodes.iter().enumerate() {
            for dep in &node.deps {
                dependents[dep.0 as usize].push(gates[i].clone());
            }
        }
        let shared = pool.shared();
        let mut roots = Vec::new();
        for (i, (node, dependents)) in self.nodes.into_iter().zip(dependents).enumerate() {
            let panic = Arc::clone(&panic);
            let func = node.func;
            let wrapper: JobFn = Box::new(move || {
                if let Err(payload) = catch_unwind(AssertUnwindSafe(func)) {
                    let mut slot = panic.lock();
                    if slot.is_none() {
                        *slot = Some(payload);
                    }
                }
                for gate in dependents {
                    gate.decrement();
                }
            });
            let job = Job::with_signal(wrapper, done.clone());
            if node.deps.is_empty() {
                roots.push((node.priority, job));
            } else {
                gates[i].then(Arc::clone(shared), node.priority, job);
            }
        }
        // Continuations are registered before any root runs so no gate can be missed.
        for (priority, job) in roots {
            shared.push(priority, job);
        }
        Ok(GraphRun { done, panic })
    }

    /// Schedules the graph and waits for it, helping. Re-raises the first node panic.
    pub fn run(self, pool: &TaskPool) -> Result<(), GraphError> {
        let run = self.start(pool)?;
        run.wait(pool);
        Ok(())
    }
}

/// A graph in flight, returned by [`TaskGraph::start`].
pub struct GraphRun {
    done: Counter,
    panic: Arc<Mutex<Option<Box<dyn Any + Send>>>>,
}

impl GraphRun {
    /// Counter that reaches zero when every node has run.
    pub fn counter(&self) -> &Counter {
        &self.done
    }

    /// Whether every node has run.
    pub fn is_done(&self) -> bool {
        self.done.is_zero()
    }

    /// Waits for the graph, helping. Re-raises the first node panic.
    pub fn wait(self, pool: &TaskPool) {
        pool.wait(&self.done);
        if let Some(payload) = self.panic.lock().take() {
            resume_unwind(payload);
        }
    }
}
