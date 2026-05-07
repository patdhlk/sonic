//! What an executor's task entry actually contains.

// pub(crate) inside a private module — intentional, used by executor.rs.
#![allow(clippy::redundant_pub_crate)]

use crate::item::ExecutableItem;

/// The kind of work a [`TaskEntry`](crate::executor::TaskEntry) holds.
pub(crate) enum TaskKind {
    /// A single item dispatched as one pool job.
    Single(Box<dyn ExecutableItem>),
    /// A sequential chain of items walked by one pool job.
    Chain(Vec<Box<dyn ExecutableItem>>),
    // Graph variant lands in Task 13.
}
