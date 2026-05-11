//! [`ConnectorError`] — framework error type. Variants cover codec
//! failures (`REQ_0213`, `REQ_0214`), payload overflow (`TEST_0125`),
//! back-pressure (`REQ_0323`), and lifecycle errors.

/// Errors surfaced by the connector framework and its protocol crates.
///
/// Concrete connectors may wrap their protocol stack's errors via
/// [`ConnectorError::Stack`]; payload / codec / descriptor errors come
/// directly from the framework.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectorError {
    /// Codec encode or decode failure. Carries the codec's
    /// [`crate::PayloadCodec::format_name`] and the underlying source.
    #[error("codec error ({format}): {source}")]
    Codec {
        /// Codec format name — e.g. `"json"`.
        format: &'static str,
        /// Underlying serialiser / deserialiser error.
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    /// Outbound channel buffer is full — caller should retry or surface
    /// back-pressure to the application. `REQ_0323`.
    #[error("connector back pressure (outbound bridge saturated)")]
    BackPressure,

    /// Encoded payload exceeds the channel's compile-time maximum
    /// (`N` in [`crate::ChannelDescriptor<R, N>`]). `TEST_0125`.
    #[error("payload overflow: encoded size {actual} exceeds channel max {max}")]
    PayloadOverflow {
        /// Bytes the codec produced.
        actual: usize,
        /// Channel's compile-time maximum (the `N` const generic).
        max: usize,
    },

    /// Channel descriptor failed validation (e.g. empty name).
    #[error("invalid descriptor: {0}")]
    InvalidDescriptor(String),

    /// The connector is in [`crate::ConnectorHealth::Down`]; the caller
    /// must wait for recovery before retrying. The framework never
    /// persists envelopes to durable storage on `Down` (`REQ_0292`).
    #[error("connector down: {reason}")]
    Down {
        /// Why the connector entered `Down` — surfaced from the underlying
        /// stack or framework lifecycle.
        reason: String,
    },

    /// Underlying protocol stack reported an error (e.g. iceoryx2 service
    /// open failure, ethercrab bus error). Connector-specific wrappers
    /// may downcast `source` to their stack's native error type.
    #[error("stack error: {source}")]
    Stack {
        /// Underlying stack error.
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}

impl ConnectorError {
    /// Construct a [`ConnectorError::Codec`] from a codec name and any
    /// boxed source error. Convenience for codec implementations.
    pub fn codec<E>(format: &'static str, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Codec {
            format,
            source: Box::new(source),
        }
    }

    /// Construct a [`ConnectorError::Stack`] from any source error.
    pub fn stack<E>(source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Stack {
            source: Box::new(source),
        }
    }
}
