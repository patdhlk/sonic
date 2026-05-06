//! Return type for [`ExecutableItem::execute`].

use crate::error::ItemError;

/// What the executor should do after an item runs successfully.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ControlFlow {
    /// Continue scheduling normally.
    Continue,
    /// Abort the enclosing chain or graph (no further items dispatched).
    StopChain,
}

/// Return type of [`ExecutableItem::execute`].
pub type ExecuteResult = Result<ControlFlow, ItemError>;
