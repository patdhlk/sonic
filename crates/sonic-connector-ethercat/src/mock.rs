//! [`MockBusDriver`] — programmable [`BusDriver`] implementation.
//!
//! Always compiled (cheap; no external deps) so downstream crates can
//! also use it for their own connector tests.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use sonic_connector_core::ConnectorError;

use crate::driver::{BringUp, BusDriver};

/// Programmable [`BusDriver`] for tests. Records every method call
/// and lets tests preload sequences of return values.
///
/// PDI buffers (outputs / inputs) live behind `Mutex` so the trait's
/// callback-shaped `with_subdevice_outputs_mut` /
/// `with_subdevice_inputs` can invoke their closures with the buffer
/// locked. Configure the buffers via
/// [`MockBusDriver::with_subdevice_outputs`] /
/// [`MockBusDriver::with_subdevice_inputs`].
#[derive(Debug, Default)]
pub struct MockBusDriver {
    state: Mutex<MockState>,
    subdevice_outputs: Mutex<HashMap<u16, Vec<u8>>>,
    subdevice_inputs: Mutex<HashMap<u16, Vec<u8>>>,
}

#[derive(Debug, Default)]
struct MockState {
    /// `Some(reason)` makes the next `bring_up` fail with
    /// [`ConnectorError::Down`] carrying `reason`.
    bring_up_fails: Option<String>,
    /// Returned by `bring_up`. Defaults to `expected_wkc = 3`,
    /// `subdevice_count = 1` so simple tests don't have to configure.
    bring_up_response: BringUp,
    /// Number of `bring_up` calls that have completed (success or
    /// failure). Useful in test assertions.
    bring_up_calls: u32,
    /// Per-`cycle` working counters, drained FIFO. When empty, every
    /// subsequent `cycle` call returns [`Self::default_cycle_wkc`].
    wkc_sequence: VecDeque<u16>,
    /// Fallback WKC when `Self::with_wkc_sequence` is empty.
    default_cycle_wkc: u16,
    /// Number of `cycle` calls that have completed.
    cycle_calls: u32,
    /// When `true`, every `cycle` call copies each SubDevice's
    /// outputs buffer over its inputs buffer (synthetic loopback).
    /// Used by `TEST_0222`.
    loopback: bool,
}

impl MockBusDriver {
    /// Construct a driver with sensible defaults — `bring_up` succeeds
    /// with `expected_wkc = 3`; every `cycle` returns `3`. No PDI
    /// buffers are configured by default; configure them via
    /// [`Self::with_subdevice_outputs`] /
    /// [`Self::with_subdevice_inputs`] when testing C7a / C7b
    /// integration paths.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MockState {
                bring_up_response: BringUp {
                    expected_wkc: 3,
                    subdevice_count: 1,
                },
                default_cycle_wkc: 3,
                ..Default::default()
            }),
            subdevice_outputs: Mutex::new(HashMap::new()),
            subdevice_inputs: Mutex::new(HashMap::new()),
        }
    }

    /// Configure the outputs buffer for a synthetic SubDevice at
    /// `address`. The buffer is initialised to the supplied bytes;
    /// the dispatcher (C7b) writes through
    /// [`BusDriver::with_subdevice_outputs_mut`].
    ///
    /// # Panics
    ///
    /// Panics if another thread has poisoned the internal mutex by
    /// panicking while holding it (build-only helper; not reached
    /// in well-behaved tests).
    #[must_use]
    pub fn with_subdevice_outputs(self, address: u16, initial: Vec<u8>) -> Self {
        self.subdevice_outputs
            .lock()
            .expect("not poisoned")
            .insert(address, initial);
        self
    }

    /// Configure the inputs buffer for a synthetic SubDevice at
    /// `address`. Used by inbound-path tests to drive synthetic PDI
    /// inputs that the dispatcher will read via
    /// [`BusDriver::with_subdevice_inputs`].
    ///
    /// # Panics
    ///
    /// Panics if another thread has poisoned the internal mutex by
    /// panicking while holding it.
    #[must_use]
    pub fn with_subdevice_inputs(self, address: u16, initial: Vec<u8>) -> Self {
        self.subdevice_inputs
            .lock()
            .expect("not poisoned")
            .insert(address, initial);
        self
    }

    /// Configure the [`BringUp`] response.
    #[must_use]
    pub fn with_bring_up(self, response: BringUp) -> Self {
        self.lock().bring_up_response = response;
        self
    }

    /// Make the next `bring_up` call fail with [`ConnectorError::Down`]
    /// carrying `reason`.
    #[must_use]
    pub fn failing_bring_up(self, reason: impl Into<String>) -> Self {
        self.lock().bring_up_fails = Some(reason.into());
        self
    }

    /// Override the fallback `cycle` WKC (used after
    /// `Self::with_wkc_sequence` is drained).
    #[must_use]
    pub fn with_default_cycle_wkc(self, wkc: u16) -> Self {
        self.lock().default_cycle_wkc = wkc;
        self
    }

    /// Preload a sequence of WKC values to return from successive
    /// `cycle` calls (FIFO).
    #[must_use]
    pub fn with_wkc_sequence<I>(self, seq: I) -> Self
    where
        I: IntoIterator<Item = u16>,
    {
        self.lock().wkc_sequence = seq.into_iter().collect();
        self
    }

    /// Enable synthetic loopback: every subsequent `cycle` call
    /// copies each SubDevice's outputs buffer over its inputs
    /// buffer. Used by `TEST_0222` to exercise the full
    /// iceoryx2 ↔ PDI ↔ iceoryx2 round-trip without hardware. The
    /// outputs and inputs buffers for the same SubDevice address
    /// must both be configured (via
    /// [`Self::with_subdevice_outputs`] / [`Self::with_subdevice_inputs`]);
    /// the inputs buffer is resized to match outputs on each cycle
    /// if the lengths differ.
    #[must_use]
    pub fn with_loopback(self) -> Self {
        self.lock().loopback = true;
        self
    }

    /// Snapshot the outputs buffer for `address`. Returns `None`
    /// when no buffer was configured for that SubDevice.
    ///
    /// # Panics
    ///
    /// Panics if another thread has poisoned the internal mutex by
    /// panicking while holding it (build-only helper; not reached
    /// in well-behaved tests).
    #[must_use]
    pub fn snapshot_outputs(&self, address: u16) -> Option<Vec<u8>> {
        self.subdevice_outputs
            .lock()
            .expect("not poisoned")
            .get(&address)
            .cloned()
    }

    /// Snapshot the inputs buffer for `address`. Returns `None`
    /// when no buffer was configured for that SubDevice.
    ///
    /// # Panics
    ///
    /// Panics if another thread has poisoned the internal mutex by
    /// panicking while holding it.
    #[must_use]
    pub fn snapshot_inputs(&self, address: u16) -> Option<Vec<u8>> {
        self.subdevice_inputs
            .lock()
            .expect("not poisoned")
            .get(&address)
            .cloned()
    }

    /// Number of `bring_up` calls completed since construction.
    pub fn bring_up_calls(&self) -> u32 {
        self.lock().bring_up_calls
    }

    /// Number of `cycle` calls completed since construction.
    pub fn cycle_calls(&self) -> u32 {
        self.lock().cycle_calls
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MockState> {
        self.state.lock().expect("MockBusDriver mutex not poisoned")
    }
}

impl BusDriver for MockBusDriver {
    async fn bring_up(&mut self) -> Result<BringUp, ConnectorError> {
        let mut state = self.lock();
        state.bring_up_calls += 1;
        if let Some(reason) = state.bring_up_fails.take() {
            return Err(ConnectorError::Down { reason });
        }
        Ok(state.bring_up_response)
    }

    async fn cycle(&mut self) -> Result<u16, ConnectorError> {
        let (wkc, loopback) = {
            let mut state = self.lock();
            state.cycle_calls += 1;
            let wkc = state
                .wkc_sequence
                .pop_front()
                .unwrap_or(state.default_cycle_wkc);
            (wkc, state.loopback)
        };
        if loopback {
            // Two short critical sections: snapshot outputs, then
            // write them into inputs. Splitting the locks keeps each
            // critical section bounded.
            let outputs_snapshot: Vec<(u16, Vec<u8>)> = {
                let guard = self.subdevice_outputs.lock().expect("not poisoned");
                guard.iter().map(|(a, b)| (*a, b.clone())).collect()
            };
            let mut inputs = self.subdevice_inputs.lock().expect("not poisoned");
            for (addr, bytes) in outputs_snapshot {
                let entry = inputs.entry(addr).or_default();
                if entry.len() != bytes.len() {
                    entry.resize(bytes.len(), 0);
                }
                entry.copy_from_slice(&bytes);
            }
            drop(inputs);
        }
        Ok(wkc)
    }

    fn with_subdevice_outputs_mut<R>(
        &self,
        subdevice_address: u16,
        f: impl FnOnce(&mut [u8]) -> R,
    ) -> Option<R> {
        let mut guard = self.subdevice_outputs.lock().expect("not poisoned");
        guard
            .get_mut(&subdevice_address)
            .map(|buf| f(buf.as_mut_slice()))
    }

    fn with_subdevice_inputs<R>(
        &self,
        subdevice_address: u16,
        f: impl FnOnce(&[u8]) -> R,
    ) -> Option<R> {
        let guard = self.subdevice_inputs.lock().expect("not poisoned");
        guard.get(&subdevice_address).map(|buf| f(buf.as_slice()))
    }
}
