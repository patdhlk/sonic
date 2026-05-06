//! Execution framework for iceoryx2-based Rust applications.
//!
//! See `docs/superpowers/specs/2026-05-06-sonic-executor-design.md` for the
//! design rationale.
#![doc(html_root_url = "https://docs.rs/sonic-executor/0.1.0")]

mod channel;
mod context;
mod control_flow;
mod error;
mod item;
mod task_id;
mod trigger;

pub use channel::{Channel, Publisher, Subscriber, EVENT_SUFFIX};
pub use context::{Context, Stoppable};
pub use control_flow::{ControlFlow, ExecuteResult};
pub use error::{ExecutorError, ItemError};
pub use item::{item, item_with_triggers, ExecutableItem, FnItem, FnItemWithTriggers};
pub use task_id::TaskId;
pub use trigger::{RawListener, TriggerDeclarer};
