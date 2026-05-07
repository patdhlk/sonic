//! `signal_slot::pair` — pre-built [`ExecutableItem`]s wrapping a [`Channel<T>`](crate::Channel).

use crate::context::Context;
use crate::control_flow::{ControlFlow, ExecuteResult};
use crate::error::ExecutorError;
use crate::executor::Executor;
use crate::item::ExecutableItem;
use crate::trigger::TriggerDeclarer;
use crate::{Publisher, Subscriber};
use iceoryx2::prelude::ZeroCopySend;

/// Type alias for the optional before-send callback stored inside [`SignalItem`].
type BeforeSendCb<T> = Option<Box<dyn FnMut(&mut T) -> bool + Send + 'static>>;

/// Type alias for the optional after-receive callback stored inside [`SlotItem`].
type AfterRecvCb<T> = Option<Box<dyn FnMut(&T) -> bool + Send + 'static>>;

/// How many messages a slot consumes per `execute`.
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub enum TakePolicy {
    /// Take exactly one message; if none is available, return `StopChain`.
    Single,
    /// Take all currently buffered messages, calling `after_recv` for each.
    All,
}

/// Open a fresh signal/slot pair backed by a `Channel<T>`.
pub fn pair<T: ZeroCopySend + Default + Copy + core::fmt::Debug + Send + 'static>(
    exec: &mut Executor,
    topic: &str,
) -> Result<(SignalItem<T>, SlotItem<T>), ExecutorError> {
    let ch = exec.channel::<T>(topic)?;
    let publisher = ch.publisher()?;
    let subscriber = ch.subscriber()?;
    Ok((
        SignalItem {
            publisher,
            before_send: None,
            _marker: core::marker::PhantomData,
        },
        SlotItem {
            subscriber,
            policy: TakePolicy::Single,
            after_recv: None,
            _marker: core::marker::PhantomData,
        },
    ))
}

/// Signal half of a signal/slot pair: an [`ExecutableItem`] that, when fired,
/// publishes a message on the underlying channel.
pub struct SignalItem<T: ZeroCopySend + Default + Copy + core::fmt::Debug + Send + 'static> {
    publisher: Publisher<T>,
    before_send: BeforeSendCb<T>,
    _marker: core::marker::PhantomData<T>,
}

impl<T: ZeroCopySend + Default + Copy + core::fmt::Debug + Send + 'static> SignalItem<T> {
    /// Install a callback invoked just before each send. Returning `false`
    /// skips the send and the `execute` call returns `StopChain`.
    #[must_use]
    pub fn before_send<F>(mut self, f: F) -> Self
    where
        F: FnMut(&mut T) -> bool + Send + 'static,
    {
        self.before_send = Some(Box::new(f));
        self
    }
}

impl<T: ZeroCopySend + Default + Copy + core::fmt::Debug + Send + 'static> ExecutableItem
    for SignalItem<T>
{
    fn declare_triggers(&mut self, _d: &mut TriggerDeclarer<'_>) -> Result<(), ExecutorError> {
        Ok(())
    }

    fn execute(&mut self, _ctx: &mut Context<'_>) -> ExecuteResult {
        let did_send = if let Some(cb) = self.before_send.as_mut() {
            self.publisher
                .loan_send(|t: &mut T| (cb)(t))
                .map_err(|e| -> crate::error::ItemError { Box::new(e) })?
        } else {
            self.publisher
                .loan_send(|_| true)
                .map_err(|e| -> crate::error::ItemError { Box::new(e) })?
        };
        if did_send {
            Ok(ControlFlow::Continue)
        } else {
            Ok(ControlFlow::StopChain)
        }
    }
}

/// Slot half of a signal/slot pair: an [`ExecutableItem`] that, when its
/// channel receives a message, runs the optional `after_recv` callback.
pub struct SlotItem<T: ZeroCopySend + Default + Copy + core::fmt::Debug + Send + 'static> {
    subscriber: Subscriber<T>,
    policy: TakePolicy,
    after_recv: AfterRecvCb<T>,
    _marker: core::marker::PhantomData<T>,
}

impl<T: ZeroCopySend + Default + Copy + core::fmt::Debug + Send + 'static> SlotItem<T> {
    /// Override the default [`TakePolicy::Single`].
    #[must_use]
    pub const fn take_policy(mut self, p: TakePolicy) -> Self {
        self.policy = p;
        self
    }

    /// Install a callback invoked for each received message. Returning `false`
    /// stops the chain (returns `StopChain`).
    #[must_use]
    pub fn after_recv<F>(mut self, f: F) -> Self
    where
        F: FnMut(&T) -> bool + Send + 'static,
    {
        self.after_recv = Some(Box::new(f));
        self
    }

    /// Construct a slot from an existing subscriber rather than a fresh channel.
    #[must_use]
    pub fn from_subscriber(subscriber: Subscriber<T>) -> Self {
        Self {
            subscriber,
            policy: TakePolicy::Single,
            after_recv: None,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<T: ZeroCopySend + Default + Copy + core::fmt::Debug + Send + 'static> ExecutableItem
    for SlotItem<T>
{
    fn declare_triggers(&mut self, d: &mut TriggerDeclarer<'_>) -> Result<(), ExecutorError> {
        d.subscriber(&self.subscriber);
        Ok(())
    }

    fn execute(&mut self, _ctx: &mut Context<'_>) -> ExecuteResult {
        let mut delivered_any = false;
        while let Some(sample) = self
            .subscriber
            .take()
            .map_err(|e| -> crate::error::ItemError { Box::new(e) })?
        {
            delivered_any = true;
            if let Some(cb) = self.after_recv.as_mut() {
                if !(cb)(sample.payload()) {
                    return Ok(ControlFlow::StopChain);
                }
            }
            if matches!(self.policy, TakePolicy::Single) {
                break;
            }
        }
        if delivered_any {
            Ok(ControlFlow::Continue)
        } else {
            Ok(ControlFlow::StopChain)
        }
    }
}
