//! TEST_0216 / TEST_0217 / TEST_0218 — PDI bit-slice translation
//! round-trip + non-interference. Pure-logic; no hardware, no
//! `ethercrab`.

#![allow(
    clippy::doc_markdown,
    clippy::cast_possible_truncation,
    clippy::unreadable_literal,
    clippy::missing_const_for_fn,
    clippy::similar_names,
    clippy::explicit_iter_loop
)]

use proptest::prelude::*;
use sonic_connector_core::ConnectorError;
use sonic_connector_ethercat::{EthercatRouting, PdoDirection, pdi};

/// Helper: construct a `EthercatRouting` for the tests. Address /
/// direction don't matter for the pure-logic module (the routing's
/// other fields are inspected by the gateway dispatcher, not by
/// `pdi::*`).
fn routing(bit_offset: u32, bit_length: u16) -> EthercatRouting {
    EthercatRouting::new(0, PdoDirection::Tx, bit_offset, bit_length)
}

proptest! {
    /// TEST_0216 — byte-aligned round-trip. `bit_offset % 8 == 0`
    /// AND `bit_length % 8 == 0`. The written bytes round-trip
    /// exactly; PDI bytes outside the slice are unchanged.
    #[test]
    fn byte_aligned_round_trip_recovers_value_and_preserves_neighbours(
        byte_offset in 0usize..32,
        byte_length in 1usize..16,
        value_seed in any::<u64>(),
        prior_seed in any::<u64>(),
    ) {
        let mut pdi = pseudo_random_bytes(prior_seed, 64);
        let original_pdi = pdi.clone();

        let bit_offset = (byte_offset * 8) as u32;
        let bit_length = (byte_length * 8) as u16;
        prop_assume!(byte_offset + byte_length <= 64);
        let r = routing(bit_offset, bit_length);

        let value = pseudo_random_bytes(value_seed, byte_length);
        pdi::write_routing(&mut pdi, &r, &value).unwrap();

        // Bytes outside [byte_offset, byte_offset + byte_length) match
        // the original PDI exactly (REQ_0326 neighbour-preservation).
        for (i, (b, o)) in pdi.iter().zip(original_pdi.iter()).enumerate() {
            if i < byte_offset || i >= byte_offset + byte_length {
                prop_assert_eq!(b, o, "neighbour byte at {} changed", i);
            }
        }

        let mut readback = vec![0u8; byte_length];
        pdi::read_routing(&pdi, &r, &mut readback).unwrap();
        prop_assert_eq!(readback, value);
    }

    /// TEST_0217 — unaligned round-trip. Same property over arbitrary
    /// `bit_offset` and `bit_length` (including non-multiples of 8).
    #[test]
    fn unaligned_round_trip_recovers_value_and_preserves_neighbours(
        bit_offset in 0u32..128,
        bit_length in 1u16..96,
        value_seed in any::<u64>(),
        prior_seed in any::<u64>(),
    ) {
        prop_assume!(u32::from(bit_length) + bit_offset <= 64 * 8);
        let r = routing(bit_offset, bit_length);

        let mut pdi = pseudo_random_bytes(prior_seed, 64);
        let original_pdi = pdi.clone();

        let value_bytes = (bit_length as usize).div_ceil(8);
        let mut value = pseudo_random_bytes(value_seed, value_bytes);
        // Mask off bits past `bit_length` in the final byte of
        // value so the round-trip comparison is meaningful (read
        // returns zeros for bits beyond bit_length).
        mask_tail_bits(&mut value, bit_length as usize);

        pdi::write_routing(&mut pdi, &r, &value).unwrap();

        // Bits outside the slice are unchanged in pdi.
        let slice_start = bit_offset as usize;
        let slice_end = slice_start + bit_length as usize;
        for bit_pos in 0..(pdi.len() * 8) {
            if bit_pos < slice_start || bit_pos >= slice_end {
                let byte = bit_pos / 8;
                let bit = (bit_pos % 8) as u32;
                let new_bit = (pdi[byte] >> bit) & 1;
                let old_bit = (original_pdi[byte] >> bit) & 1;
                prop_assert_eq!(
                    new_bit, old_bit,
                    "bit {} (byte {} bit {}) outside slice changed",
                    bit_pos, byte, bit
                );
            }
        }

        let mut readback = vec![0u8; value_bytes];
        pdi::read_routing(&pdi, &r, &mut readback).unwrap();
        prop_assert_eq!(readback, value);
    }
}

/// TEST_0218 — two adjacent slices don't interfere on writes.
/// Slice A occupies bits `[0, 12)`; slice B occupies bits `[12, 24)`.
/// Distinct writes to A and B (in either order) round-trip both
/// values cleanly.
#[test]
fn adjacent_bit_slices_do_not_interfere_on_writes() {
    let r_a = routing(0, 12);
    let r_b = routing(12, 12);

    let mut pdi = vec![0u8; 8];

    let value_a = [0xAB, 0x0C]; // bits 0..12 = 0xCAB (little-endian within slice)
    let value_b = [0xCD, 0x07]; // bits 0..12 = 0x7CD

    // Write A first, then B.
    pdi::write_routing(&mut pdi, &r_a, &value_a).unwrap();
    pdi::write_routing(&mut pdi, &r_b, &value_b).unwrap();

    let mut readback_a = [0u8; 2];
    let mut readback_b = [0u8; 2];
    pdi::read_routing(&pdi, &r_a, &mut readback_a).unwrap();
    pdi::read_routing(&pdi, &r_b, &mut readback_b).unwrap();

    assert_eq!(readback_a, value_a, "slice A corrupted after B written");
    assert_eq!(readback_b, value_b, "slice B did not match value");

    // Reverse order: fresh buffer, write B first, then A.
    let mut pdi2 = vec![0u8; 8];
    pdi::write_routing(&mut pdi2, &r_b, &value_b).unwrap();
    pdi::write_routing(&mut pdi2, &r_a, &value_a).unwrap();

    let mut readback_a2 = [0u8; 2];
    let mut readback_b2 = [0u8; 2];
    pdi::read_routing(&pdi2, &r_a, &mut readback_a2).unwrap();
    pdi::read_routing(&pdi2, &r_b, &mut readback_b2).unwrap();

    assert_eq!(readback_a2, value_a, "slice A corrupted (reverse order)");
    assert_eq!(readback_b2, value_b, "slice B corrupted (reverse order)");
}

#[test]
fn payload_overflow_when_value_too_short_for_bit_length() {
    let r = routing(0, 16);
    let mut pdi = [0u8; 4];
    let value = [0xAA]; // only 8 bits, need 16.
    let err = pdi::write_routing(&mut pdi, &r, &value).expect_err("overflow");
    assert!(matches!(err, ConnectorError::PayloadOverflow { .. }));
}

#[test]
fn payload_overflow_when_pdi_too_short_for_slice() {
    let r = routing(28, 8); // bits 28..36; needs at least 5 bytes.
    let mut pdi = [0u8; 4]; // only 32 bits.
    let value = [0xFF];
    let err = pdi::write_routing(&mut pdi, &r, &value).expect_err("overflow");
    assert!(matches!(err, ConnectorError::PayloadOverflow { .. }));
}

#[test]
fn zero_length_routing_is_noop() {
    let r = routing(7, 0); // bit_offset doesn't matter when length is 0
    let mut pdi = vec![0xAA, 0xBB, 0xCC];
    let value: [u8; 0] = [];
    pdi::write_routing(&mut pdi, &r, &value).unwrap();
    assert_eq!(
        pdi,
        vec![0xAA, 0xBB, 0xCC],
        "zero-length write must not touch pdi"
    );

    let mut into: [u8; 0] = [];
    pdi::read_routing(&pdi, &r, &mut into).unwrap();
    // (no data to compare; just exercise the path)
}

#[test]
fn read_zeros_trailing_bits_of_final_byte() {
    // bit_length=5 → final byte has bits 5..8 cleared.
    let r = routing(0, 5);
    let pdi = [0b1101_1011u8]; // bits 0..5 = 0b1_1011 = 0x1B
    let mut into = [0xFFu8]; // pre-filled non-zero
    pdi::read_routing(&pdi, &r, &mut into).unwrap();
    assert_eq!(
        into[0], 0b0001_1011,
        "read must clear bits beyond bit_length"
    );
}

// ── helpers ────────────────────────────────────────────────────

fn pseudo_random_bytes(seed: u64, len: usize) -> Vec<u8> {
    // Tiny LCG; good enough for filling proptest buffers
    // deterministically.
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        out.push((state >> 33) as u8);
    }
    out
}

fn mask_tail_bits(buf: &mut [u8], total_bits: usize) {
    if buf.is_empty() {
        return;
    }
    let used_bits_in_last = total_bits % 8;
    if used_bits_in_last == 0 {
        return; // exact byte fit, no masking needed
    }
    let last_byte_idx = total_bits / 8;
    if last_byte_idx < buf.len() {
        let mask = (1u8 << used_bits_in_last) - 1;
        buf[last_byte_idx] &= mask;
    }
    // Bytes past the used range — zero them too.
    let used_bytes = total_bits.div_ceil(8);
    for byte in &mut buf[used_bytes..] {
        *byte = 0;
    }
}
