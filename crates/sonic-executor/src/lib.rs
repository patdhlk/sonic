//! Execution framework for iceoryx2-based Rust applications.
//!
//! See `docs/superpowers/specs/2026-05-06-sonic-executor-design.md` for the
//! design rationale.
#![doc(html_root_url = "https://docs.rs/sonic-executor/0.1.0")]

mod chain;
mod channel;
mod condition;
mod context;
mod control_flow;
mod error;
mod executor;
mod item;
mod pool;
mod runner;
mod task_id;
mod task_kind;
mod trigger;

pub use channel::{Channel, Publisher, Subscriber, EVENT_SUFFIX};
pub use condition::{wrap_with_condition, Conditional};
pub use context::{Context, Stoppable};
pub use control_flow::{ControlFlow, ExecuteResult};
pub use error::{ExecutorError, ItemError};
pub use executor::{Executor, ExecutorBuilder};
pub use item::{item, item_with_triggers, ExecutableItem, FnItem, FnItemWithTriggers};
pub use runner::{Runner, RunnerFlags};
pub use task_id::TaskId;
pub use trigger::{RawListener, TriggerDeclarer};
