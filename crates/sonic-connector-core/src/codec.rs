//! [`PayloadCodec`] — compile-time codec dispatch trait. `REQ_0210`,
//! `REQ_0211`.

use crate::ConnectorError;

/// How typed values become payload bytes and back. Each `Connector`
/// implementation parameterises on `C: PayloadCodec`; monomorphisation
/// makes the codec choice a compile-time decision (`REQ_0211`).
///
/// Implementations live in `sonic-connector-codec` (e.g. `JsonCodec` per
/// `REQ_0212`). User-provided codecs may also implement this trait — the
/// framework does not require any particular format.
pub trait PayloadCodec {
    /// Static name for the format this codec implements (e.g. `"json"`).
    /// Carried in [`ConnectorError::Codec`] when encode / decode fail.
    fn format_name(&self) -> &'static str;

    /// Encode `value` into `buf`. Returns the number of bytes written.
    ///
    /// # Errors
    ///
    /// * [`ConnectorError::Codec`] when serialisation fails for any reason
    ///   (buffer too small, serialiser-internal error).
    /// * [`ConnectorError::PayloadOverflow`] if the caller-provided buffer
    ///   is smaller than the encoded form.
    fn encode<T>(&self, value: &T, buf: &mut [u8]) -> Result<usize, ConnectorError>
    where
        T: serde::Serialize;

    /// Decode `T` from `buf`.
    ///
    /// # Errors
    ///
    /// [`ConnectorError::Codec`] when deserialisation fails (truncated
    /// input, wrong shape, deserialiser-internal error). The framework
    /// shall not silently drop the envelope — `REQ_0214`.
    fn decode<T>(&self, buf: &[u8]) -> Result<T, ConnectorError>
    where
        T: serde::de::DeserializeOwned;
}
