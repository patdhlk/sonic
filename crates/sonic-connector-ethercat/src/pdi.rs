//! PDI bit-slice translation. Pure-logic helpers for `REQ_0326`
//! (outbound payload → PDI bit slice) and `REQ_0327` (PDI bit slice →
//! inbound payload).
//!
//! Both functions operate on a per-SubDevice PDI buffer (the slice
//! returned by `BusDriver::subdevice_outputs_mut` / `subdevice_inputs`,
//! which corresponds to ethercrab's
//! `subdevice.outputs_raw_mut()` / `inputs_raw()`). The
//! [`EthercatRouting`]'s `bit_offset` is relative to byte 0 of that
//! per-SubDevice slice, NOT the flat group PDI.
//!
//! Bit ordering matches the EtherCAT convention: bit 0 of byte 0 is
//! the first bit, bit 7 of byte 0 is the eighth, bit 0 of byte 1 is
//! the ninth, and so on. The payload buffer (`value` for write,
//! `into` for read) follows the same convention.
//!
//! The implementation is bit-at-a-time. For typical PDOs (16 / 32 /
//! 64-bit signal types) that's 16–64 iterations per call — trivial
//! against the cycle period. A byte-aligned fast path could be
//! added if benchmarks justify it; the spec text demands correctness
//! across arbitrary bit alignments, which the loop handles uniformly.

use sonic_connector_core::ConnectorError;

use crate::routing::EthercatRouting;

/// Write `value`'s first `routing.bit_length` bits into `pdi`
/// starting at `routing.bit_offset`. Preserves all other bits in
/// `pdi` (read-modify-write on partial leading / trailing bytes).
/// `REQ_0326`.
///
/// # Errors
///
/// Returns [`ConnectorError::PayloadOverflow`] when:
///
/// * `value` is too short to cover `bit_length` bits, or
/// * `pdi` is too short to cover `bit_offset + bit_length` bits.
///
/// # Panics
///
/// Panics only via the `expect` calls on `u32::try_from(_ % 8)`,
/// which is structurally infeasible (`x % 8` always fits in `u32`).
/// Listed for clippy completeness.
pub fn write_routing(
    pdi: &mut [u8],
    routing: &EthercatRouting,
    value: &[u8],
) -> Result<(), ConnectorError> {
    let bit_length = routing.bit_length as usize;
    if bit_length == 0 {
        return Ok(());
    }

    let value_bits_available = value.len().saturating_mul(8);
    if value_bits_available < bit_length {
        return Err(ConnectorError::PayloadOverflow {
            actual: bit_length,
            max: value_bits_available,
        });
    }

    let bit_offset = routing.bit_offset as usize;
    let pdi_bits_available = pdi.len().saturating_mul(8);
    if bit_offset.saturating_add(bit_length) > pdi_bits_available {
        return Err(ConnectorError::PayloadOverflow {
            actual: bit_offset.saturating_add(bit_length),
            max: pdi_bits_available,
        });
    }

    for i in 0..bit_length {
        let src_byte = i / 8;
        let src_bit = u32::try_from(i % 8).expect("i % 8 < 8 fits in u32");
        let bit = (value[src_byte] >> src_bit) & 1;

        let dst_pos = bit_offset + i;
        let dst_byte = dst_pos / 8;
        let dst_bit = u32::try_from(dst_pos % 8).expect("dst_pos % 8 < 8 fits in u32");

        // Clear-then-set on the target bit. Leaves every other bit
        // in this byte (and every other byte in `pdi`) untouched —
        // this is what guarantees REQ_0326's "preserves adjacent
        // slices" clause.
        pdi[dst_byte] = (pdi[dst_byte] & !(1u8 << dst_bit)) | (bit << dst_bit);
    }

    Ok(())
}

/// Extract `routing.bit_length` bits from `pdi` starting at
/// `routing.bit_offset`. `REQ_0327`.
///
/// Writes them into `into`'s first `ceil(bit_length / 8)` bytes
/// (low-numbered bits in low-numbered bytes). Bits beyond
/// `bit_length` in the final byte of `into` are cleared to 0, so
/// the caller can rely on a clean payload. Does not modify `pdi`.
///
/// # Errors
///
/// Returns [`ConnectorError::PayloadOverflow`] when:
///
/// * `into` is too short to hold `bit_length` bits, or
/// * `pdi` is too short to cover `bit_offset + bit_length` bits.
///
/// # Panics
///
/// See [`write_routing`].
pub fn read_routing(
    pdi: &[u8],
    routing: &EthercatRouting,
    into: &mut [u8],
) -> Result<(), ConnectorError> {
    let bit_length = routing.bit_length as usize;
    if bit_length == 0 {
        // Caller may have passed a non-empty `into` buffer — leave
        // it as-is. A zero-length routing reads zero bits.
        return Ok(());
    }

    let into_bits_available = into.len().saturating_mul(8);
    if into_bits_available < bit_length {
        return Err(ConnectorError::PayloadOverflow {
            actual: bit_length,
            max: into_bits_available,
        });
    }

    let bit_offset = routing.bit_offset as usize;
    let pdi_bits_available = pdi.len().saturating_mul(8);
    if bit_offset.saturating_add(bit_length) > pdi_bits_available {
        return Err(ConnectorError::PayloadOverflow {
            actual: bit_offset.saturating_add(bit_length),
            max: pdi_bits_available,
        });
    }

    // Clear the destination bytes we'll touch so caller-supplied
    // buffer contents past `bit_length` end up zero (per the
    // function's contract).
    let touched_bytes = bit_length.div_ceil(8);
    for byte in &mut into[..touched_bytes] {
        *byte = 0;
    }

    for i in 0..bit_length {
        let src_pos = bit_offset + i;
        let src_byte = src_pos / 8;
        let src_bit = u32::try_from(src_pos % 8).expect("src_pos % 8 < 8 fits in u32");
        let bit = (pdi[src_byte] >> src_bit) & 1;

        let dst_byte = i / 8;
        let dst_bit = u32::try_from(i % 8).expect("i % 8 < 8 fits in u32");
        into[dst_byte] |= bit << dst_bit;
    }

    Ok(())
}
