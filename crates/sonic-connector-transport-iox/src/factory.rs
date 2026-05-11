//! [`ServiceFactory`] — opens / creates the iceoryx2 pub/sub service for
//! a [`ChannelDescriptor`] and returns typed [`ChannelWriter`] /
//! [`ChannelReader`] handles. `REQ_0206`.
//!
//! Service-name convention: `<descriptor.name()>`. The two-direction
//! split mandated by `REQ_0206` (outbound app→gateway vs inbound
//! gateway→app) is realised at the host crate layer (`BB_0005`), which
//! constructs two `ChannelDescriptor` instances per logical channel —
//! one for each direction — and the gateway and plugin sides agree on
//! the naming convention there. This crate intentionally exposes a
//! single-direction API to keep the layering clean.

use iceoryx2::node::Node;
use iceoryx2::prelude::ipc;
use sonic_connector_core::{ChannelDescriptor, ConnectorError, PayloadCodec, Routing};

use crate::channel::{ChannelReader, ChannelWriter};
use crate::envelope::ConnectorEnvelope;
use crate::raw::{RawChannelReader, RawChannelWriter};

/// Wraps an iceoryx2 [`Node`] and opens pub/sub services on demand.
///
/// `ServiceFactory` borrows the node — the caller owns the node and is
/// responsible for keeping it alive for the lifetime of every writer /
/// reader the factory hands out.
pub struct ServiceFactory<'n> {
    node: &'n Node<ipc::Service>,
}

impl<'n> ServiceFactory<'n> {
    /// Construct a factory bound to `node`.
    #[must_use]
    pub const fn new(node: &'n Node<ipc::Service>) -> Self {
        Self { node }
    }

    /// Open or create the pub/sub service named after `descriptor` and
    /// return a [`ChannelWriter`] (the publisher side).
    pub fn create_writer<T, R, C, const N: usize>(
        &self,
        descriptor: &ChannelDescriptor<R, N>,
        codec: C,
    ) -> Result<ChannelWriter<T, C, N>, ConnectorError>
    where
        T: serde::Serialize,
        R: Routing,
        C: PayloadCodec,
    {
        let service = self.open_pubsub::<N>(descriptor.name())?;
        let publisher = service
            .publisher_builder()
            .create()
            .map_err(|e| ConnectorError::stack(svc_error(format!("publisher: {e:?}"))))?;
        Ok(ChannelWriter::new(publisher, codec))
    }

    /// Open or create the pub/sub service named after `descriptor` and
    /// return a [`ChannelReader`] (the subscriber side).
    pub fn create_reader<T, R, C, const N: usize>(
        &self,
        descriptor: &ChannelDescriptor<R, N>,
        codec: C,
    ) -> Result<ChannelReader<T, C, N>, ConnectorError>
    where
        T: serde::de::DeserializeOwned,
        R: Routing,
        C: PayloadCodec,
    {
        let service = self.open_pubsub::<N>(descriptor.name())?;
        let subscriber = service
            .subscriber_builder()
            .create()
            .map_err(|e| ConnectorError::stack(svc_error(format!("subscriber: {e:?}"))))?;
        Ok(ChannelReader::new(subscriber, codec))
    }

    /// Open or create the pub/sub service `name` and return a
    /// [`RawChannelWriter`]. Used by the gateway dispatcher
    /// (`REQ_0327`) to publish PDI bit-slice bytes back to the plugin
    /// without invoking the channel's codec.
    pub fn create_raw_writer_named<const N: usize>(
        &self,
        name: &str,
    ) -> Result<RawChannelWriter<N>, ConnectorError> {
        let service = self.open_pubsub::<N>(name)?;
        let publisher = service
            .publisher_builder()
            .create()
            .map_err(|e| ConnectorError::stack(svc_error(format!("publisher: {e:?}"))))?;
        Ok(RawChannelWriter::new(publisher))
    }

    /// Open or create the pub/sub service `name` and return a
    /// [`RawChannelReader`]. Used by the gateway dispatcher
    /// (`REQ_0326`) to drain plugin-side publisher envelopes into a
    /// caller-provided buffer without invoking the channel's codec.
    pub fn create_raw_reader_named<const N: usize>(
        &self,
        name: &str,
    ) -> Result<RawChannelReader<N>, ConnectorError> {
        let service = self.open_pubsub::<N>(name)?;
        let subscriber = service
            .subscriber_builder()
            .create()
            .map_err(|e| ConnectorError::stack(svc_error(format!("subscriber: {e:?}"))))?;
        Ok(RawChannelReader::new(subscriber))
    }

    fn open_pubsub<const N: usize>(
        &self,
        name: &str,
    ) -> Result<
        iceoryx2::service::port_factory::publish_subscribe::PortFactory<
            ipc::Service,
            ConnectorEnvelope<N>,
            (),
        >,
        ConnectorError,
    > {
        let service_name = name
            .try_into()
            .map_err(|e| ConnectorError::InvalidDescriptor(format!("iceoryx2 name: {e:?}")))?;
        self.node
            .service_builder(&service_name)
            .publish_subscribe::<ConnectorEnvelope<N>>()
            .open_or_create()
            .map_err(|e| ConnectorError::stack(svc_error(format!("open_or_create: {e:?}"))))
    }
}

/// Format helper for wrapping iceoryx2 service errors into a boxed
/// `std::error::Error`. Kept private so the public API exposes only
/// [`ConnectorError`].
const fn svc_error(msg: String) -> SvcError {
    SvcError(msg)
}

#[derive(Debug)]
struct SvcError(String);

impl core::fmt::Display for SvcError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "iceoryx2 service: {}", self.0)
    }
}

impl std::error::Error for SvcError {}
