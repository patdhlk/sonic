//! What an executor's task entry actually contains.

// pub(crate) inside a private module — intentional, used by executor.rs.
#![allow(clippy::redundant_pub_crate)]

use crate::graph::Graph;
use crate::item::ExecutableItem;

/// The kind of work a [`TaskEntry`](crate::executor::TaskEntry) holds.
#[allow(clippy::large_enum_variant)] // Graph variant is naturally larger; chains and graphs are rare relative to singles
pub(crate) enum TaskKind {
    /// A single item dispatched as one pool job.
    Single(Box<dyn ExecutableItem>),
    /// A sequential chain of items walked by one pool job.
    Chain(Vec<Box<dyn ExecutableItem>>),
    /// A DAG of items dispatched in dependency order. Task 14 wires the scheduler.
    ///
    /// Boxed so the inner `Graph` lives at a stable heap address — REQ_0060
    /// requires per-vertex dispatch closures to capture a raw pointer back
    /// into the `Graph` (counters, ready ring, etc.). If the `Graph` were
    /// stored inline, moving the `TaskEntry` (e.g. when `self.tasks` grows)
    /// would invalidate every captured pointer.
    #[allow(dead_code)] // Task 14 will read this field when wiring the DAG scheduler.
    Graph(Box<Graph>),
}
