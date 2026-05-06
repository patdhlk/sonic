//! Trigger declaration. Items pass an iceoryx2 listener / interval / etc. into
//! the [`TriggerDeclarer`]; the executor consumes the declarations at add-time
//! and turns them into `WaitSet` attachments inside its run loop.

/// Collects trigger intentions; consumed by the executor at add-time.
///
/// The full surface lands in Task 5. This stub exists so [`ExecutableItem`]
/// can be tested without a real executor.
pub struct TriggerDeclarer<'a> {
    _marker: core::marker::PhantomData<&'a mut ()>,
    #[allow(dead_code)]
    pub(crate) decls: Vec<TriggerDecl>,
}

#[allow(dead_code, clippy::redundant_pub_crate)]
#[derive(Debug)]
pub(crate) enum TriggerDecl {
    /// Placeholder so the enum compiles before Task 5 populates it.
    _Empty,
}

impl TriggerDeclarer<'_> {
    /// Internal constructor; used by the executor when adding a task.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub(crate) fn new_internal() -> Self {
        Self { _marker: core::marker::PhantomData, decls: Vec::new() }
    }

    #[cfg(test)]
    pub(crate) fn new_test() -> Self {
        Self::new_internal()
    }
}
