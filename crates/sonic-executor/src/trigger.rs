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
#[derive(Clone, Debug)]
pub(crate) enum TriggerDecl {
    /// Wake when the listener (paired with a subscriber's channel) fires.
    Subscriber {
        /// Listener cloned from the subscriber's paired event service.
        listener: Arc<RawListener>,
    },
    /// Wake periodically.
    Interval(Duration),
    /// Wake on the listener firing OR after `deadline` elapses without one.
    ///
    /// `listener` and `deadline` live in the same variant because iceoryx2's
    /// `WaitSet::attach_deadline` takes both atomically; splitting them here
    /// would create a footgun where one could be attached without the other.
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
    pub fn interval(&mut self, period: impl Into<Duration>) -> &mut Self {
        self.decls.push(TriggerDecl::Interval(period.into()));
        self
    }

    /// Declare a subscriber trigger that *also* fires the deadline if no
    /// event arrives within `deadline`.
    pub fn deadline<T: ZeroCopySend + Default + core::fmt::Debug + 'static>(
        &mut self,
        sub: &Subscriber<T>,
        deadline: impl Into<Duration>,
    ) -> &mut Self {
        self.decls.push(TriggerDecl::Deadline {
            listener: sub.listener_handle(),
            deadline: deadline.into(),
        });
        self
    }

    /// Escape hatch — attach a raw iceoryx2 listener directly.
    pub fn raw_listener(&mut self, listener: Arc<RawListener>) -> &mut Self {
        self.decls.push(TriggerDecl::RawListener(listener));
        self
    }

    /// Declare that the item should fire when the server receives a request.
    pub fn server<Req, Resp>(&mut self, srv: &crate::service::Server<Req, Resp>) -> &mut Self
    where
        Req: iceoryx2::prelude::ZeroCopySend + Default + core::fmt::Debug + Copy + 'static,
        Resp: iceoryx2::prelude::ZeroCopySend + Default + core::fmt::Debug + Copy + 'static,
    {
        self.decls.push(TriggerDecl::Subscriber {
            listener: srv.listener_handle(),
        });
        self
    }

    /// Declare that the item should fire when the client receives a response.
    pub fn client<Req, Resp>(&mut self, cl: &crate::service::Client<Req, Resp>) -> &mut Self
    where
        Req: iceoryx2::prelude::ZeroCopySend + Default + core::fmt::Debug + Copy + 'static,
        Resp: iceoryx2::prelude::ZeroCopySend + Default + core::fmt::Debug + Copy + 'static,
    {
        self.decls.push(TriggerDecl::Subscriber {
            listener: cl.listener_handle(),
        });
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

    fn make_subscriber(topic: &str) -> crate::Subscriber<Msg> {
        use iceoryx2::prelude::*;
        let node = NodeBuilder::new().create::<ipc::Service>().unwrap();
        let ch = crate::Channel::<Msg>::open_or_create(&node, topic).unwrap();
        ch.subscriber().unwrap()
    }

    #[test]
    fn collects_subscriber_decl() {
        let sub = make_subscriber("sonic.test.trig.sub");
        let expected = sub.listener_handle();
        let mut d = TriggerDeclarer::new_test();
        d.subscriber(&sub);
        assert_eq!(d.decls.len(), 1);
        let TriggerDecl::Subscriber { listener } = &d.decls[0] else {
            panic!("expected Subscriber variant");
        };
        assert!(std::sync::Arc::ptr_eq(listener, &expected));
    }

    #[test]
    fn collects_interval_decl() {
        let mut d = TriggerDeclarer::new_test();
        d.interval(Duration::from_millis(100));
        assert!(matches!(d.decls[0], TriggerDecl::Interval(dur) if dur == Duration::from_millis(100)));
    }

    #[test]
    fn collects_deadline_decl() {
        let sub = make_subscriber("sonic.test.trig.deadline");
        let expected_listener = sub.listener_handle();
        let mut d = TriggerDeclarer::new_test();
        d.deadline(&sub, Duration::from_millis(50));
        let TriggerDecl::Deadline { listener, deadline } = &d.decls[0] else {
            panic!("expected Deadline variant");
        };
        assert!(std::sync::Arc::ptr_eq(listener, &expected_listener));
        assert_eq!(*deadline, Duration::from_millis(50));
    }

    #[test]
    fn collects_raw_listener_decl() {
        let sub = make_subscriber("sonic.test.trig.raw");
        let handle = sub.listener_handle();
        let expected = std::sync::Arc::clone(&handle);
        let mut d = TriggerDeclarer::new_test();
        d.raw_listener(handle);
        let TriggerDecl::RawListener(stored) = &d.decls[0] else {
            panic!("expected RawListener variant");
        };
        assert!(std::sync::Arc::ptr_eq(stored, &expected));
    }

    #[test]
    #[allow(clippy::unnecessary_wraps)]
    fn declarer_chains() -> Result<(), ExecutorError> {
        let sub = make_subscriber("sonic.test.trig.chain");
        let mut d = TriggerDeclarer::new_test();
        d.subscriber(&sub).interval(Duration::from_millis(10));
        assert_eq!(d.decls.len(), 2);
        Ok(())
    }
}
