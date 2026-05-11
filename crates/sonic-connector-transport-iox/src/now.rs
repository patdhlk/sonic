//! UNIX-epoch nanosecond timestamp helper. `REQ_0203`.

// `pub(crate)` inside a private module — intentional, used by `channel`.
#![allow(clippy::redundant_pub_crate)]

use std::time::{SystemTime, UNIX_EPOCH};

/// Current wall-clock time as nanoseconds since the UNIX epoch.
///
/// Returns `0` if the system clock is set before 1970 (which only
/// happens on machines with a corrupted RTC); the alternative —
/// panicking — would crash the gateway on a recoverable condition.
pub(crate) fn now_unix_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}
