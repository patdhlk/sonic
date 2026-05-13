//! [`ZenohQueryable`] — plugin-side query-handling handle
//! (`REQ_0420`, `REQ_0422`, `REQ_0423`, `REQ_0426`). Constructed by
//! [`crate::ZenohConnector::create_queryable`] (wired in Z3e).

use std::marker::PhantomData;

use sonic_connector_core::{ConnectorError, PayloadCodec};
use sonic_connector_transport_iox::{RawChannelReader, RawChannelWriter};

use crate::registry::QueryId;
use crate::session::FrameKind;

/// Plugin-side query-handling handle.
///
/// `Q` is the request type (decoded from the gateway-delivered envelope);
/// `R` is the reply type (encoded on [`Self::reply`]).
pub struct ZenohQueryable<Q, R, C, const N: usize>
where
    C: PayloadCodec,
{
    reader: RawChannelReader<N>,
    writer: RawChannelWriter<N>,
    codec: C,
    scratch_recv: Vec<u8>,
    scratch_send: Vec<u8>,
    _ty: PhantomData<fn() -> (Q, R)>,
}

impl<Q, R, C, const N: usize> ZenohQueryable<Q, R, C, N>
where
    Q: serde::de::DeserializeOwned,
    R: serde::Serialize,
    C: PayloadCodec,
{
    /// Construct a queryable from raw iox handles. Called only by the
    /// connector's `create_queryable` impl (wired in Z3e).
    #[allow(dead_code)] // Wired in Z3e via `ZenohConnector::create_queryable`.
    pub(crate) fn new(
        reader: RawChannelReader<N>,
        writer: RawChannelWriter<N>,
        codec: C,
    ) -> Self {
        Self {
            reader,
            writer,
            codec,
            scratch_recv: vec![0u8; N],
            scratch_send: vec![0u8; N],
            _ty: PhantomData,
        }
    }

    /// Try to receive one incoming query. Returns `Ok(None)` if no
    /// envelope is pending; `Ok(Some((id, q)))` with the [`QueryId`]
    /// minted by the gateway and the decoded request value.
    ///
    /// # Errors
    /// Returns [`ConnectorError::Codec`] on decode failure.
    pub fn try_recv(&mut self) -> Result<Option<(QueryId, Q)>, ConnectorError> {
        let Some(sample) = self.reader.try_recv_into(&mut self.scratch_recv)? else {
            return Ok(None);
        };
        let id = QueryId(sample.correlation_id);
        let value: Q = self.codec.decode(&self.scratch_recv[..sample.payload_len])?;
        Ok(Some((id, value)))
    }

    /// Send one reply chunk for the given [`QueryId`]. Multiple replies
    /// are supported (`REQ_0423`); a final [`Self::terminate`]
    /// finalises the stream (`REQ_0426`).
    ///
    /// # Errors
    /// Returns [`ConnectorError::Codec`] on encode failure;
    /// [`ConnectorError::PayloadOverflow`] if the encoded reply +
    /// 1-byte discriminator exceeds `N`.
    pub fn reply(&mut self, id: QueryId, r: &R) -> Result<(), ConnectorError> {
        if self.scratch_send.is_empty() {
            return Err(ConnectorError::PayloadOverflow { actual: 1, max: 0 });
        }
        self.scratch_send[0] = FrameKind::Data.discriminator();
        let written = self.codec.encode(r, &mut self.scratch_send[1..])?;
        self.writer
            .send_raw_bytes(&self.scratch_send[..=written], id.0)?;
        Ok(())
    }

    /// Finalise the reply stream for `id` (`REQ_0426`). Publishes a
    /// 1-byte `0x02` envelope; the gateway uses this to drop its
    /// upstream-Query handle, finalising the stream from the upstream
    /// peer's perspective.
    ///
    /// # Errors
    /// Returns [`ConnectorError::Stack`] on iox failure.
    pub fn terminate(&mut self, id: QueryId) -> Result<(), ConnectorError> {
        self.scratch_send[0] = FrameKind::EndOfStream.discriminator();
        self.writer.send_raw_bytes(&self.scratch_send[..1], id.0)?;
        Ok(())
    }
}
