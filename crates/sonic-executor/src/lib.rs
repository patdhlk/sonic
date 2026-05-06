//! Execution framework for iceoryx2-based Rust applications.
//!
//! See `docs/superpowers/specs/2026-05-06-sonic-executor-design.md` for the
//! design rationale.
#![doc(html_root_url = "https://docs.rs/sonic-executor/0.1.0")]

mod error;
mod task_id;

pub use error::{ExecutorError, ItemError};
pub use task_id::TaskId;
