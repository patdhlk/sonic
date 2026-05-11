//! [`JsonCodec`] — `serde_json`-backed `PayloadCodec`. ``REQ_0212``.
//!
//! Encode writes directly into the caller-provided buffer (via a small
//! `CountingWriter` adapter), so a successful encode does not allocate
//! on the heap. Buffer-too-small surfaces as
//! [`ConnectorError::PayloadOverflow`] for consistency with
//! `ChannelWriter::send`'s overflow behaviour (`TEST_0125`); other
//! serializer failures surface as [`ConnectorError::Codec`] (`REQ_0213`).
//!
//! Decode delegates to `serde_json::from_slice`; failures (truncated
//! input, schema mismatch) surface as [`ConnectorError::Codec`]
//! (`REQ_0214`) rather than being silently dropped.

use std::io;

use sonic_connector_core::{ConnectorError, PayloadCodec};

/// JSON codec built on `serde_json`. Zero-sized; clone-cheap;
/// thread-safe.
#[derive(Copy, Clone, Debug, Default)]
pub struct JsonCodec;

impl JsonCodec {
    /// Static format name carried in [`ConnectorError::Codec`]. Constant
    /// `"json"`.
    pub const FORMAT_NAME: &'static str = "json";

    /// Construct a fresh codec. `Default` is identical; provided as a
    /// convenience for explicit construction in `static` contexts.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl PayloadCodec for JsonCodec {
    fn format_name(&self) -> &'static str {
        Self::FORMAT_NAME
    }

    fn encode<T>(&self, value: &T, buf: &mut [u8]) -> Result<usize, ConnectorError>
    where
        T: serde::Serialize,
    {
        let max = buf.len();
        let mut writer = CountingWriter::new(buf);
        match serde_json::to_writer(&mut writer, value) {
            Ok(()) => Ok(writer.bytes_written()),
            Err(e) if e.is_io() => {
                // Serde_json's writer adapter signals buffer exhaustion via
                // `io::ErrorKind::WriteZero` from our `CountingWriter`. The
                // overflow path is the only IO category serde_json emits when
                // wrapping a writer that never blocks.
                //
                // Compute the actual encoded size on the error path so the
                // caller can see how big the payload would have been. Re-
                // encoding via `to_vec` allocates a Vec, but only on the
                // failure path; the success path stays allocation-free.
                let actual = serde_json::to_vec(value).map_or(max + 1, |v| v.len());
                Err(ConnectorError::PayloadOverflow { actual, max })
            }
            Err(e) => Err(ConnectorError::codec(Self::FORMAT_NAME, e)),
        }
    }

    fn decode<T>(&self, buf: &[u8]) -> Result<T, ConnectorError>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_slice(buf).map_err(|e| ConnectorError::codec(Self::FORMAT_NAME, e))
    }
}

/// Counts bytes written and returns [`io::ErrorKind::WriteZero`] on
/// overflow so [`serde_json::to_writer`] surfaces a recognisable
/// failure. Holds a `&mut [u8]` borrow — caller owns the buffer.
struct CountingWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> CountingWriter<'a> {
    const fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    const fn bytes_written(&self) -> usize {
        self.pos
    }
}

impl io::Write for CountingWriter<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let remaining = self.buf.len() - self.pos;
        if data.len() > remaining {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "buffer full"));
        }
        self.buf[self.pos..self.pos + data.len()].copy_from_slice(data);
        self.pos += data.len();
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
