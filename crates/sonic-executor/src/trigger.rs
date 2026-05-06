//! Trigger declaration. Items hand iceoryx2 listeners / intervals / etc. to
//! the [`TriggerDeclarer`]; the executor turns the recorded declarations into
//! `WaitSet` attachments at add-time.

use crate::Subscriber;
use core::time::Duration;
use iceoryx2::port::listener::Listener as IxListener;
use iceoryx2::prelude::ipc;
use iceoryx2::prelude::ZeroCopySend;
use std::sync::Arc;

/// Listener type the rest of the crate manipulates. Aliased so client code
/// using `RawListener` keeps working if iceoryx2 renames its types.
pub type RawListener = IxListener<ipc::Service>;

/// Internal representation of a trigger declaration. Consumed by the executor.
#[allow(dead_code, clippy::redundant_pub_crate)]
#[derive(Debug)]
pub(crate) enum TriggerDecl {
    /// Wake when the listener (paired with a subscriber's channel) fires.
    Subscriber {
        /// Listener cloned from the subscriber's paired event service.
        listener: Arc<RawListener>,
    },
    /// Wake periodically.
    Interval(Duration),
    /// Wake on the listener firing OR after `deadline` elapses without one.
    Deadline {
        /// Listener cloned from the subscriber's paired event service.
        listener: Arc<RawListener>,
        /// Deadline duration after which a missed-deadline event fires.
        deadline: Duration,
    },
    /// Raw user-supplied listener, used as the escape hatch.
    RawListener(Arc<RawListener>),
}

/// Records trigger intentions. Consumed by the executor at add-time.
pub struct TriggerDeclarer<'a> {
    _marker: core::marker::PhantomData<&'a mut ()>,
    pub(crate) decls: Vec<TriggerDecl>,
}

impl TriggerDeclarer<'_> {
    /// Internal constructor used by the executor when adding a task.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub(crate) fn new_internal() -> Self {
        Self {
            _marker: core::marker::PhantomData,
            decls: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_test() -> Self {
        Self::new_internal()
    }

    /// Declare that the item should fire when the given subscriber receives.
    pub fn subscriber<T: ZeroCopySend + Default + core::fmt::Debug + 'static>(
        &mut self,
        sub: &Subscriber<T>,
    ) -> &mut Self {
        self.decls.push(TriggerDecl::Subscriber {
            listener: sub.listener_handle(),
        });
        self
    }

    /// Declare a periodic interval trigger.
    pub fn interval(&mut self, period: Duration) -> &mut Self {
        self.decls.push(TriggerDecl::Interval(period));
        self
    }

    /// Declare a subscriber trigger that *also* fires the deadline if no
    /// event arrives within `deadline`.
    pub fn deadline<T: ZeroCopySend + Default + core::fmt::Debug + 'static>(
        &mut self,
        sub: &Subscriber<T>,
        deadline: Duration,
    ) -> &mut Self {
        self.decls.push(TriggerDecl::Deadline {
            listener: sub.listener_handle(),
            deadline,
        });
        self
    }

    /// Escape hatch — attach a raw iceoryx2 listener directly.
    pub fn raw_listener(&mut self, listener: Arc<RawListener>) -> &mut Self {
        self.decls.push(TriggerDecl::RawListener(listener));
        self
    }

    /// Drain the recorded declarations.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub(crate) fn into_decls(self) -> Vec<TriggerDecl> {
        self.decls
    }

    /// True if any triggers were declared. Used by the executor to warn when
    /// non-head items in a chain declare triggers (Task 12).
    #[doc(hidden)]
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.decls.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ExecutorError;

    #[derive(Debug, Default, Clone, Copy, iceoryx2::prelude::ZeroCopySend)]
    #[repr(C)]
    struct Msg(u32);

    fn make_subscriber() -> crate::Subscriber<Msg> {
        use iceoryx2::prelude::*;
        let node = NodeBuilder::new().create::<ipc::Service>().unwrap();
        let ch = crate::Channel::<Msg>::open_or_create(&node, "sonic.test.trig").unwrap();
        ch.subscriber().unwrap()
    }

    #[test]
    fn collects_subscriber_decl() {
        let sub = make_subscriber();
        let mut d = TriggerDeclarer::new_test();
        d.subscriber(&sub);
        assert_eq!(d.decls.len(), 1);
        assert!(matches!(d.decls[0], TriggerDecl::Subscriber { .. }));
    }

    #[test]
    fn collects_interval_decl() {
        let mut d = TriggerDeclarer::new_test();
        d.interval(Duration::from_millis(100));
        assert!(matches!(d.decls[0], TriggerDecl::Interval(dur) if dur == Duration::from_millis(100)));
    }

    #[test]
    fn collects_deadline_decl() {
        let sub = make_subscriber();
        let mut d = TriggerDeclarer::new_test();
        d.deadline(&sub, Duration::from_millis(50));
        assert!(matches!(d.decls[0], TriggerDecl::Deadline { .. }));
    }

    #[test]
    #[allow(clippy::unnecessary_wraps)]
    fn declarer_chains() -> Result<(), ExecutorError> {
        let sub = make_subscriber();
        let mut d = TriggerDeclarer::new_test();
        d.subscriber(&sub).interval(Duration::from_millis(10));
        assert_eq!(d.decls.len(), 2);
        Ok(())
    }
}
