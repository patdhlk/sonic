//! `Service<Req, Resp>` — iceoryx2 request/response paired with two event
//! services (one each for request- and response-available wakeups).

use crate::error::ExecutorError;
use core::marker::PhantomData;
use iceoryx2::node::Node;
use iceoryx2::port::client::Client as IxClient;
use iceoryx2::port::listener::Listener as IxListener;
use iceoryx2::port::notifier::Notifier as IxNotifier;
use iceoryx2::port::server::Server as IxServer;
use iceoryx2::prelude::*;
use iceoryx2::response::Response as IxResponse;
use std::sync::Arc;

type IpcService = ipc::Service;

/// Suffix appended to a service name to form the request-available event service name.
pub const REQ_EVENT_SUFFIX: &str = ".__sonic_req_event";

/// Suffix appended to a service name to form the response-available event service name.
pub const RESP_EVENT_SUFFIX: &str = ".__sonic_resp_event";

/// Request/response service with two paired event services for wakeup.
pub struct Service<Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + 'static,
{
    rr: iceoryx2::service::port_factory::request_response::PortFactory<
        IpcService,
        Req,
        (),
        Resp,
        (),
    >,
    req_event: iceoryx2::service::port_factory::event::PortFactory<IpcService>,
    resp_event: iceoryx2::service::port_factory::event::PortFactory<IpcService>,
    _marker: PhantomData<(Req, Resp)>,
}

impl<Req, Resp> Service<Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + 'static,
{
    /// Open or create the service by name, creating the two paired event services.
    pub fn open_or_create(
        node: &Node<IpcService>,
        topic: &str,
    ) -> Result<Arc<Self>, ExecutorError> {
        let rr_name = topic
            .try_into()
            .map_err(|e| ExecutorError::Builder(format!("invalid service name: {e:?}")))?;
        let rr = node
            .service_builder(&rr_name)
            .request_response::<Req, Resp>()
            .open_or_create()
            .map_err(ExecutorError::iceoryx2)?;

        let make_event = |suffix: &str| -> Result<_, ExecutorError> {
            let n = format!("{topic}{suffix}");
            let n = n
                .as_str()
                .try_into()
                .map_err(|e| ExecutorError::Builder(format!("invalid event-topic name: {e:?}")))?;
            node.service_builder(&n)
                .event()
                .open_or_create()
                .map_err(ExecutorError::iceoryx2)
        };
        let req_event = make_event(REQ_EVENT_SUFFIX)?;
        let resp_event = make_event(RESP_EVENT_SUFFIX)?;

        Ok(Arc::new(Self {
            rr,
            req_event,
            resp_event,
            _marker: PhantomData,
        }))
    }

    /// Create a new `Server` that listens for requests on this service.
    pub fn server(self: &Arc<Self>) -> Result<Server<Req, Resp>, ExecutorError> {
        let inner = self
            .rr
            .server_builder()
            .create()
            .map_err(ExecutorError::iceoryx2)?;
        let listener = self
            .req_event
            .listener_builder()
            .create()
            .map_err(ExecutorError::iceoryx2)?;
        let resp_notifier = self
            .resp_event
            .notifier_builder()
            .create()
            .map_err(ExecutorError::iceoryx2)?;
        // SAFETY: see `impl Send for Server<Req, Resp>` below.
        #[allow(clippy::arc_with_non_send_sync)]
        let listener = Arc::new(listener);
        Ok(Server {
            inner,
            listener,
            resp_notifier,
            _service: Arc::clone(self),
        })
    }

    /// Create a new `Client` that sends requests on this service.
    pub fn client(self: &Arc<Self>) -> Result<Client<Req, Resp>, ExecutorError> {
        let inner = self
            .rr
            .client_builder()
            .create()
            .map_err(ExecutorError::iceoryx2)?;
        let listener = self
            .resp_event
            .listener_builder()
            .create()
            .map_err(ExecutorError::iceoryx2)?;
        let req_notifier = self
            .req_event
            .notifier_builder()
            .create()
            .map_err(ExecutorError::iceoryx2)?;
        // SAFETY: see `impl Send for Client<Req, Resp>` below.
        #[allow(clippy::arc_with_non_send_sync)]
        let listener = Arc::new(listener);
        Ok(Client {
            inner,
            listener,
            req_notifier,
            _service: Arc::clone(self),
        })
    }
}

/// Server side of a `Service<Req, Resp>`. Receives requests and sends responses.
pub struct Server<Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + 'static,
{
    inner: IxServer<IpcService, Req, (), Resp, ()>,
    listener: Arc<IxListener<IpcService>>,
    resp_notifier: IxNotifier<IpcService>,
    _service: Arc<Service<Req, Resp>>,
}

// SAFETY: `IxServer<ipc::Service, …>` is `!Send` because
// `ipc::Service::ArcThreadSafetyPolicy` is `SingleThreaded`, which wraps an
// `Rc`-like interior.  The Rc is only mutated during port creation (constructor)
// and during `update_connections` (called inside `receive()`).  After
// construction, the executor only calls:
//   * `server.receive()` — drives `update_connections()` + shared-memory read
//   * `server.listener_handle()` — cheap `Arc::clone` of our own Arc
// No two threads concurrently touch the Rc, so moving a `Server` is sound.
// We do not implement `Sync`; the struct is move-only across threads.
#[allow(unsafe_code, clippy::non_send_fields_in_send_ty)]
unsafe impl<Req, Resp> Send for Server<Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + 'static,
{
}

impl<Req, Resp> Server<Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + Copy + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + Copy + 'static,
{
    /// Take the next pending request, if any.
    ///
    /// Returns `(payload_copy, ActiveRequest)`. Use the `ActiveRequest` to
    /// respond via `respond_copy`.
    #[allow(clippy::type_complexity, clippy::option_if_let_else)]
    pub fn take_request(
        &self,
    ) -> Result<Option<(Req, ActiveRequest<'_, Req, Resp>)>, ExecutorError> {
        match self.inner.receive().map_err(ExecutorError::iceoryx2)? {
            None => Ok(None),
            Some(active) => {
                let req = *active;
                Ok(Some((
                    req,
                    ActiveRequest {
                        active,
                        server: self,
                    },
                )))
            }
        }
    }

    /// Borrow the request-event listener (executor uses this for trigger attachment).
    pub fn listener_handle(&self) -> Arc<IxListener<IpcService>> {
        Arc::clone(&self.listener)
    }
}

/// A received request, used to send the response.
pub struct ActiveRequest<'s, Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + 'static,
{
    active: iceoryx2::active_request::ActiveRequest<IpcService, Req, (), Resp, ()>,
    server: &'s Server<Req, Resp>,
}

impl<Req, Resp> ActiveRequest<'_, Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + Copy + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + Copy + 'static,
{
    /// Send a response by value and notify the client's listener.
    pub fn respond_copy(self, resp: Resp) -> Result<(), ExecutorError> {
        let sample = self.active.loan_uninit().map_err(ExecutorError::iceoryx2)?;
        let sample = sample.write_payload(resp);
        sample.send().map_err(ExecutorError::iceoryx2)?;
        self.server
            .resp_notifier
            .notify()
            .map_err(ExecutorError::iceoryx2)?;
        Ok(())
    }
}

/// Client side of a `Service<Req, Resp>`. Sends requests and receives responses.
pub struct Client<Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + 'static,
{
    inner: IxClient<IpcService, Req, (), Resp, ()>,
    listener: Arc<IxListener<IpcService>>,
    req_notifier: IxNotifier<IpcService>,
    _service: Arc<Service<Req, Resp>>,
}

// SAFETY: same rationale as `Server<Req, Resp>` above.
// `IxClient<ipc::Service, …>` is `!Send` because `SingleThreaded` holds an Rc.
// After construction, only `send_copy` and `listener_handle` are called.
// `send_copy` does not touch the Rc concurrently; `listener_handle` is an
// `Arc::clone`. No concurrent Rc mutation, so moving a `Client` is sound.
// We do not implement `Sync`.
#[allow(unsafe_code, clippy::non_send_fields_in_send_ty)]
unsafe impl<Req, Resp> Send for Client<Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + 'static,
{
}

impl<Req, Resp> Client<Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + Copy + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + Copy + 'static,
{
    /// Send a request by value. Returns a `PendingRequest` handle for receiving
    /// the response(s), and notifies the server's listener.
    pub fn send_copy(&self, req: Req) -> Result<PendingRequest<Req, Resp>, ExecutorError> {
        let pending = self.inner.send_copy(req).map_err(ExecutorError::iceoryx2)?;
        self.req_notifier
            .notify()
            .map_err(ExecutorError::iceoryx2)?;
        Ok(PendingRequest { inner: pending })
    }

    /// Borrow the response-event listener (executor uses this for trigger attachment).
    pub fn listener_handle(&self) -> Arc<IxListener<IpcService>> {
        Arc::clone(&self.listener)
    }
}

/// Handle to an in-flight request — receives the matching response(s).
pub struct PendingRequest<Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + 'static,
{
    inner: iceoryx2::pending_response::PendingResponse<IpcService, Req, (), Resp, ()>,
}

// SAFETY: `PendingResponse<ipc::Service, …>` is `!Send` for the same
// `SingleThreaded` Rc reason.  After construction, only `receive()` is
// called (shared-memory read path, no concurrent Rc mutation).
// Move-only across threads; no `Sync`.
#[allow(unsafe_code, clippy::non_send_fields_in_send_ty)]
unsafe impl<Req, Resp> Send for PendingRequest<Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + 'static,
{
}

impl<Req, Resp> PendingRequest<Req, Resp>
where
    Req: core::fmt::Debug + ZeroCopySend + Default + Copy + 'static,
    Resp: core::fmt::Debug + ZeroCopySend + Default + Copy + 'static,
{
    /// Try to receive the next response, if one has arrived.
    ///
    /// The iceoryx2 0.8.1 `PendingResponse::receive()` returns a
    /// `Response<IpcService, Resp, ()>`, not a `Sample` — this wraps it.
    pub fn take(&self) -> Result<Option<IxResponse<IpcService, Resp, ()>>, ExecutorError> {
        self.inner.receive().map_err(ExecutorError::iceoryx2)
    }
}
