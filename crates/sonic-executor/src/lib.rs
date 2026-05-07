//! Execution framework for iceoryx2-based Rust applications.
//!
//! See `docs/superpowers/specs/2026-05-06-sonic-executor-design.md` for the
//! design rationale.
#![doc(html_root_url = "https://docs.rs/sonic-executor/0.1.0")]

mod chain;
mod channel;
mod condition;
pub mod signal_slot;
mod context;
mod control_flow;
mod error;
mod executor;
mod graph;
mod item;
mod monitor;
mod observer;
mod pool;
mod runner;
mod service;
mod shutdown;
mod task_id;
mod task_kind;
mod thread_attrs;
mod trigger;

pub use monitor::ExecutionMonitor;
pub use observer::{Observer, UserEvent};
pub use channel::{Channel, Publisher, Subscriber, EVENT_SUFFIX};
pub use service::{
    ActiveRequest, Client, PendingRequest, Server, Service, REQ_EVENT_SUFFIX, RESP_EVENT_SUFFIX,
};
pub use condition::{wrap_with_condition, Conditional};
pub use context::{Context, Stoppable};
pub use control_flow::{ControlFlow, ExecuteResult};
pub use error::{ExecutorError, ItemError};
pub use executor::{Executor, ExecutorBuilder, ExecutorGraphBuilder};
pub use graph::{GraphBuilder, Vertex};
pub use item::{item, item_with_triggers, ExecutableItem, FnItem, FnItemWithTriggers};
pub use runner::{Runner, RunnerFlags};
pub use task_id::TaskId;
pub use thread_attrs::ThreadAttributes;
pub use trigger::{RawListener, TriggerDeclarer};
