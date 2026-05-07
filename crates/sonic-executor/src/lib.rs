//! # sonic-executor
//!
//! Execution framework on top of [iceoryx2](https://docs.rs/iceoryx2).
//! Provides:
//!
//! * [`ExecutableItem`] — the unit of work the executor schedules.
//! * [`Executor`] / [`Runner`] — the run loop and a dedicated-thread host.
//! * [`Channel<T>`](Channel) / [`Service`] — pub/sub and request-response primitives
//!   with paired event services so subscribers wake on send.
//! * Sequential [chains](Executor::add_chain) and parallel
//!   [graphs](Executor::add_graph).
//! * Signal/slot via [`signal_slot::pair`].
//! * Lifecycle hooks via [`Observer`] and timing hooks via
//!   [`ExecutionMonitor`].
//!
//! See the workspace `README.md` for a quick-start.
#![doc(html_root_url = "https://docs.rs/sonic-executor/0.1.0")]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod chain;
mod channel;
mod condition;
mod context;
mod control_flow;
mod error;
mod executor;
mod graph;
mod item;
mod monitor;
mod observer;
mod payload;
mod pool;
mod runner;
mod service;
mod shutdown;
pub mod signal_slot;
mod task_id;
mod task_kind;
mod thread_attrs;
mod trigger;

pub use channel::{Channel, EVENT_SUFFIX, Publisher, Subscriber};
pub use condition::{Conditional, wrap_with_condition};
pub use context::{Context, Stoppable};
pub use control_flow::{ControlFlow, ExecuteResult};
pub use error::{ExecutorError, ItemError};
pub use executor::{Executor, ExecutorBuilder, ExecutorGraphBuilder};
pub use graph::{GraphBuilder, Vertex};
pub use item::{ExecutableItem, FnItem, FnItemWithTriggers, item, item_with_triggers};
pub use monitor::ExecutionMonitor;
pub use observer::{Observer, UserEvent};
pub use payload::Payload;
pub use runner::{Runner, RunnerFlags};
pub use service::{
    ActiveRequest, Client, PendingRequest, REQ_EVENT_SUFFIX, RESP_EVENT_SUFFIX, Server, Service,
};
pub use task_id::TaskId;
pub use thread_attrs::ThreadAttributes;
pub use trigger::{RawListener, TriggerDeclarer};
