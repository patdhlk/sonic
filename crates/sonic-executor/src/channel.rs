//! `Channel<T>` — iceoryx2 publish/subscribe paired with an event service so
//! that subscribers can be attached as triggers on the executor's `WaitSet`.

use crate::error::ExecutorError;
use core::marker::PhantomData;
use iceoryx2::port::listener::Listener as IxListener;
use iceoryx2::port::notifier::Notifier as IxNotifier;
use iceoryx2::port::publisher::Publisher as IxPublisher;
use iceoryx2::port::subscriber::Subscriber as IxSubscriber;
use iceoryx2::prelude::*;
use iceoryx2::sample::Sample as IxSample;
use std::sync::Arc;

/// Suffix appended to a topic name to form the paired event-service name.
///
/// `Channel<T>` reserves this suffix; users must not open an event service
/// at `<topic><EVENT_SUFFIX>` themselves through iceoryx2 directly.
pub const EVENT_SUFFIX: &str = ".__sonic_event";

type IpcService = ipc::Service;

/// Pub/sub channel with a paired event service.
pub struct Channel<T: core::fmt::Debug + ZeroCopySend + 'static> {
    pubsub: iceoryx2::service::port_factory::publish_subscribe::PortFactory<IpcService, T, ()>,
    event: iceoryx2::service::port_factory::event::PortFactory<IpcService>,
    _marker: PhantomData<T>,
}

impl<T: core::fmt::Debug + ZeroCopySend + 'static> Channel<T> {
    /// Open or create the channel by topic name.
    pub fn open_or_create(
        node: &iceoryx2::node::Node<IpcService>,
        topic: &str,
    ) -> Result<Arc<Self>, ExecutorError> {
        let pubsub_name = topic
            .try_into()
            .map_err(|e| ExecutorError::Builder(format!("invalid topic name: {e:?}")))?;
        let pubsub = node
            .service_builder(&pubsub_name)
            .publish_subscribe::<T>()
            .open_or_create()
            .map_err(ExecutorError::iceoryx2)?;

        let event_topic = format!("{topic}{EVENT_SUFFIX}");
        let event_name = event_topic
            .as_str()
            .try_into()
            .map_err(|e| ExecutorError::Builder(format!("invalid event-topic name: {e:?}")))?;
        let event = node
            .service_builder(&event_name)
            .event()
            .open_or_create()
            .map_err(ExecutorError::iceoryx2)?;

        Ok(Arc::new(Self {
            pubsub,
            event,
            _marker: PhantomData,
        }))
    }

    /// Create a new publisher attached to this channel.
    pub fn publisher(self: &Arc<Self>) -> Result<Publisher<T>, ExecutorError> {
        let inner = self
            .pubsub
            .publisher_builder()
            .create()
            .map_err(ExecutorError::iceoryx2)?;
        let notifier = self
            .event
            .notifier_builder()
            .create()
            .map_err(ExecutorError::iceoryx2)?;
        Ok(Publisher {
            inner,
            notifier,
            _channel: Arc::clone(self),
        })
    }

    /// Create a new subscriber attached to this channel.
    pub fn subscriber(self: &Arc<Self>) -> Result<Subscriber<T>, ExecutorError> {
        let inner = self
            .pubsub
            .subscriber_builder()
            .create()
            .map_err(ExecutorError::iceoryx2)?;
        let listener = self
            .event
            .listener_builder()
            .create()
            .map_err(ExecutorError::iceoryx2)?;
        // SAFETY: iceoryx2's `Listener<ipc::Service>` is conditionally
        // `Send + Sync` (the impl exists but clippy cannot verify the concrete
        // service type satisfies the bounds at this generic call site).
        #[allow(clippy::arc_with_non_send_sync)]
        let listener = Arc::new(listener);
        Ok(Subscriber {
            inner,
            listener,
            _channel: Arc::clone(self),
        })
    }
}

/// Pub/sub publisher that auto-notifies the paired event service on every send.
pub struct Publisher<T: core::fmt::Debug + ZeroCopySend + 'static> {
    inner: IxPublisher<IpcService, T, ()>,
    notifier: IxNotifier<IpcService>,
    _channel: Arc<Channel<T>>,
}

impl<T: core::fmt::Debug + ZeroCopySend + 'static + Copy> Publisher<T> {
    /// Send by value (copies). Notifies the paired event service on success.
    pub fn send_copy(&self, value: T) -> Result<(), ExecutorError> {
        self.inner
            .send_copy(value)
            .map_err(ExecutorError::iceoryx2)?;
        self.notifier.notify().map_err(ExecutorError::iceoryx2)?;
        Ok(())
    }
}

impl<T: core::fmt::Debug + ZeroCopySend + 'static> Publisher<T> {
    /// Loan an uninitialised sample, run `f` to fill it, then send + notify.
    /// Returns `Ok(false)` if `f` returns `false` — caller signalled "skip send".
    ///
    /// # Example
    ///
    /// ```no_run
    /// use iceoryx2::prelude::*;
    /// use sonic_executor::Channel;
    /// use std::sync::Arc;
    ///
    /// #[derive(Debug, Default, Clone, Copy, ZeroCopySend)]
    /// #[repr(C)]
    /// struct Tick(u64);
    ///
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let node = NodeBuilder::new().create::<ipc::Service>()?;
    /// let ch: Arc<Channel<Tick>> = Channel::open_or_create(&node, "demo")?;
    /// let publisher = ch.publisher()?;
    ///
    /// publisher.loan_send(|t: &mut Tick| { t.0 = 1; true })?;
    /// # Ok(()) }
    /// ```
    #[allow(unsafe_code)]
    pub fn loan_send<F>(&self, f: F) -> Result<bool, ExecutorError>
    where
        F: FnOnce(&mut T) -> bool,
    {
        let mut sample = self.inner.loan_uninit().map_err(ExecutorError::iceoryx2)?;
        // SAFETY: zero-init is sound for ZeroCopySend types iceoryx2 supports.
        // We let `f` overwrite. We do not require `Default` on `T`.
        let payload_ptr = sample.payload_mut().as_mut_ptr();
        unsafe { core::ptr::write_bytes(payload_ptr, 0, 1) };
        // SAFETY: write_bytes above zero-initialised the payload; for any
        // ZeroCopySend type accepted by iceoryx2, an all-zero bit pattern is
        // a valid value (the trait requires types compatible with shared
        // memory transport, which precludes types with niche bit-pattern
        // requirements that would invalidate this).
        let mut sample = unsafe { sample.assume_init() };
        let cont = f(sample.payload_mut());
        if !cont {
            return Ok(false);
        }
        sample.send().map_err(ExecutorError::iceoryx2)?;
        self.notifier.notify().map_err(ExecutorError::iceoryx2)?;
        Ok(true)
    }
}

/// Pub/sub subscriber. Carries the paired event listener as `Arc<Listener>`
/// so the executor can attach it to its `WaitSet`.
pub struct Subscriber<T: core::fmt::Debug + ZeroCopySend + 'static> {
    inner: IxSubscriber<IpcService, T, ()>,
    listener: Arc<IxListener<IpcService>>,
    _channel: Arc<Channel<T>>,
}

impl<T: core::fmt::Debug + ZeroCopySend + 'static> Subscriber<T> {
    /// Take the next sample, if any.
    pub fn take(&self) -> Result<Option<IxSample<IpcService, T, ()>>, ExecutorError> {
        self.inner.receive().map_err(ExecutorError::iceoryx2)
    }

    /// Borrow the listener handle (executor uses this for trigger attachment).
    #[doc(hidden)]
    pub fn listener_handle(&self) -> Arc<IxListener<IpcService>> {
        Arc::clone(&self.listener)
    }
}
