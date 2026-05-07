//! Chains are stored as a [`TaskKind::Chain`](crate::task_kind::TaskKind) and
//! walked sequentially by the executor's dispatch loop. The public API lives
//! on [`Executor::add_chain`](crate::Executor::add_chain).
