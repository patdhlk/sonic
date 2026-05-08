//! `Channel<T>` — iceoryx2 publish/subscribe paired with an event service so
//! that subscribers can be attached as triggers on the executor's `WaitSet`.

use crate::error::ExecutorError;
use crate::payload::Payload;
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
}

impl<T: Payload> Channel<T> {
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

        Ok(Arc::new(Self { pubsub, event }))
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
        Ok(Publisher { inner, notifier })
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
        Ok(Subscriber { inner, listener })
    }
}

/// Pub/sub publisher that auto-notifies the paired event service on every send.
pub struct Publisher<T: core::fmt::Debug + ZeroCopySend + 'static> {
    inner: IxPublisher<IpcService, T, ()>,
    notifier: IxNotifier<IpcService>,
}

// SAFETY: same rationale as `Subscriber<T>` above. `IxPublisher` is
// `!Send` only because of the same `SingleThreaded` Rc; after port
// creation, `publisher.send_copy(...)` and `publisher.loan_send(...)`
// don't touch the Rc concurrently. Move-only, no Sync.
#[allow(unsafe_code, clippy::non_send_fields_in_send_ty)]
unsafe impl<T: core::fmt::Debug + ZeroCopySend + 'static> Send for Publisher<T> {}

impl<T: Payload + Copy> Publisher<T> {
    /// Send by value (copies). Notifies the paired event service on success.
    pub fn send_copy(&self, value: T) -> Result<(), ExecutorError> {
        self.inner
            .send_copy(value)
            .map_err(ExecutorError::iceoryx2)?;
        self.notifier.notify().map_err(ExecutorError::iceoryx2)?;
        Ok(())
    }
}

impl<T: Payload> Publisher<T> {
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
    ///
    /// Loan a sample initialised to `T::default()`, run `f` to fill it, then
    /// send + notify. Returns `Ok(false)` if `f` returns `false` — caller
    /// signalled "skip send".
    ///
    /// `T: Default` is required here because the shared-memory slot is
    /// pre-initialised via `T::default()` before the closure runs. For types
    /// that do not implement `Default`, use [`loan`](Self::loan) instead.
    pub fn loan_send<F>(&self, f: F) -> Result<bool, ExecutorError>
    where
        T: Default,
        F: FnOnce(&mut T) -> bool,
    {
        let sample = self.inner.loan_uninit().map_err(ExecutorError::iceoryx2)?;
        let mut sample = sample.write_payload(T::default());
        let cont = f(sample.payload_mut());
        if !cont {
            return Ok(false);
        }
        sample.send().map_err(ExecutorError::iceoryx2)?;
        self.notifier.notify().map_err(ExecutorError::iceoryx2)?;
        Ok(true)
    }

    /// True zero-copy send. The closure receives `&mut MaybeUninit<T>`; it
    /// must fully initialize the payload (e.g., via `MaybeUninit::write(v)`
    /// or in-place construction such as iceoryx2's `placement_default!`)
    /// before returning `true`. Returning `false` skips the send.
    ///
    /// On success, sends and notifies. Returns `Ok(true)` if the payload was
    /// sent, `Ok(false)` if the closure returned false.
    ///
    /// # Contract
    ///
    /// **Returning `true` from the closure asserts that the payload is
    /// fully initialized.** Returning `true` without writing a valid `T`
    /// causes undefined behaviour at the subsequent `assume_init` step.
    ///
    /// `T: Default` is **not** required — that's the point of this method
    /// versus [`loan_send`](Self::loan_send). For types that have a sensible
    /// `Default` and are cheap to default-construct, prefer `loan_send`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use core::mem::MaybeUninit;
    /// use iceoryx2::prelude::*;
    /// use sonic_executor::Channel;
    /// use std::sync::Arc;
    ///
    /// #[derive(Debug, ZeroCopySend)]
    /// #[repr(C)]
    /// struct LargeMsg { payload: [u8; 64] }
    ///
    /// // Manual Default impl — e.g. initialised to a sentinel value rather
    /// // than zero, so `loan_send` would use it but it is expensive.
    /// impl Default for LargeMsg {
    ///     fn default() -> Self { LargeMsg { payload: [0xFF; 64] } }
    /// }
    ///
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let node = NodeBuilder::new().create::<ipc::Service>()?;
    /// let ch: Arc<Channel<LargeMsg>> = Channel::open_or_create(&node, "demo")?;
    /// let publisher = ch.publisher()?;
    ///
    /// publisher.loan(|slot: &mut MaybeUninit<LargeMsg>| {
    ///     // Initialise directly in shared memory — no Default construction,
    ///     // no stack temporary for the payload array.
    ///     slot.write(LargeMsg { payload: [0u8; 64] });
    ///     true
    /// })?;
    /// # Ok(()) }
    /// ```
    #[allow(unsafe_code)]
    pub fn loan<F>(&self, f: F) -> Result<bool, ExecutorError>
    where
        F: FnOnce(&mut core::mem::MaybeUninit<T>) -> bool,
    {
        let mut sample = self.inner.loan_uninit().map_err(ExecutorError::iceoryx2)?;
        let cont = f(sample.payload_mut());
        if !cont {
            return Ok(false);
        }
        // SAFETY: the closure returned `true`, asserting that the payload was
        // fully initialised before this point. Per the documented contract,
        // a closure that returns `true` without writing a valid `T` is a
        // contract violation and the resulting behaviour is undefined.
        let sample = unsafe { sample.assume_init() };
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
}

// SAFETY:
// `IxSubscriber<ipc::Service, T, ()>` is `!Send` because the `ipc::Service`
// `ArcThreadSafetyPolicy` is `SingleThreaded`, which holds an `Rc<...>`.
// The Rc is mutated only when methods that call `lock()` on the policy
// run — primarily during port creation. After construction, the executor
// only invokes:
//   * `subscriber.take()` → `IxSubscriber::receive()` (does not touch the
//     listener's Rc; pure shared-memory read path)
//   * `subscriber.listener_handle()` → cheap `Arc::clone` (own Arc, not iceoryx2's Rc)
// No two threads concurrently mutate the same Rc refcount, so moving a
// `Subscriber` to a pool worker is sound. We do not implement `Sync`;
// `Subscriber` is move-only across threads, never shared.
#[allow(unsafe_code, clippy::non_send_fields_in_send_ty)]
unsafe impl<T: core::fmt::Debug + ZeroCopySend + 'static> Send for Subscriber<T> {}

impl<T: Payload> Subscriber<T> {
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
