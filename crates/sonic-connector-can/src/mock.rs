//! [`MockCanInterface`] — in-process loopback implementing
//! [`CanInterfaceLike`]. `BB_0075`, `REQ_0504`.
//!
//! Used by every layer-1 test in the corpus (TEST_0500–TEST_0510,
//! TEST_0513, TEST_0514). Ships unfeature-gated so layer-1 work
//! happens on Linux, macOS, and Windows without the `socketcan`
//! kernel module.
//!
//! Semantics:
//!
//! * `send_classical` / `send_fd` push the frame into the mock's
//!   own internal queue. Loopback — a paired `recv` returns the
//!   frame on the same interface. Multi-iface tests instantiate
//!   one `MockCanInterface` per iface; each has its own queue so
//!   frames sent on `vcan0` are invisible on `vcan1` (matches real
//!   isolated SocketCAN behaviour).
//! * Tests inject "external" data frames via
//!   [`MockCanInterface::inject_frame`] and error conditions via
//!   [`MockCanInterface::inject_error`]; both surface through
//!   `recv`.
//! * `apply_filter` records the filter set for test inspection but
//!   does not drop non-matching frames — kernel filtering is real
//!   `socketcan`'s job. The dispatcher's reader-level match
//!   (`filter::matches`) is the user-visible filter regardless of
//!   back-end.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::driver::{CanData, CanFilter, CanFrame, CanIfaceState, CanInterfaceLike, CanIoError};
use crate::routing::{CanFrameKind, CanIface};

/// Test-programmable mock state shared between
/// [`MockCanInterface`] and the test fixture.
#[derive(Debug, Default)]
pub struct MockCanState {
    /// Most recent filter set the dispatcher applied via
    /// `apply_filter`. Tests inspect this to assert filter
    /// compilation correctness.
    pub last_applied_filter: Vec<CanFilter>,
    /// Current bus state. Tests modify directly to simulate
    /// external state changes (e.g. driver-level reset).
    pub state: CanIfaceState,
    /// Cumulative count of `apply_filter` calls — useful for
    /// asserting recompute behaviour (`REQ_0623`).
    pub apply_filter_count: u32,
    /// Cumulative count of frames sent through this iface.
    pub send_count: u32,
}

/// In-process loopback CAN interface.
#[derive(Debug)]
pub struct MockCanInterface {
    iface: CanIface,
    inner: Arc<Mutex<MockCanState>>,
    tx: mpsc::UnboundedSender<CanFrame>,
    rx: mpsc::UnboundedReceiver<CanFrame>,
}

impl MockCanInterface {
    /// Construct a fresh mock interface in `Active` state with an
    /// empty filter set.
    #[must_use]
    pub fn new(iface: CanIface) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            iface,
            inner: Arc::new(Mutex::new(MockCanState {
                state: CanIfaceState::Active,
                ..Default::default()
            })),
            tx,
            rx,
        }
    }

    /// Construct from an explicit shared state. Multiple
    /// `MockCanInterface` instances may share state when the test
    /// wants to simulate a shared bus.
    #[must_use]
    pub fn from_shared_state(iface: CanIface, inner: Arc<Mutex<MockCanState>>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            iface,
            inner,
            tx,
            rx,
        }
    }

    /// Clone of the shared state handle for test inspection.
    #[must_use]
    pub fn state_handle(&self) -> Arc<Mutex<MockCanState>> {
        Arc::clone(&self.inner)
    }

    /// Clone of the internal sender, so tests can inject frames or
    /// errors that look like external bus traffic.
    #[must_use]
    pub fn tx_handle(&self) -> mpsc::UnboundedSender<CanFrame> {
        self.tx.clone()
    }

    /// Inject a data frame as if it arrived from the bus.
    pub fn inject_frame(&self, frame: CanData) {
        let _ = self.tx.send(CanFrame::Data(frame));
    }

    /// Inject an error condition as if a kernel error frame arrived.
    pub fn inject_error(&self, err: crate::driver::CanErrorKind) {
        let _ = self.tx.send(CanFrame::Error(err));
    }
}

impl CanInterfaceLike for MockCanInterface {
    fn iface(&self) -> &CanIface {
        &self.iface
    }

    fn state(&self) -> CanIfaceState {
        self.inner
            .lock()
            .expect("mock state lock not poisoned")
            .state
    }

    fn apply_filter(&mut self, filters: &[CanFilter]) -> Result<(), CanIoError> {
        let mut guard = self.inner.lock().expect("mock state lock not poisoned");
        guard.last_applied_filter = filters.to_vec();
        guard.apply_filter_count = guard.apply_filter_count.saturating_add(1);
        Ok(())
    }

    fn recv(
        &mut self,
    ) -> impl core::future::Future<Output = Result<CanFrame, CanIoError>> + Send + '_ {
        async move {
            // Honour Closed / BusOff — recv fails fast.
            match self.state() {
                CanIfaceState::Closed => return Err(CanIoError::Closed),
                CanIfaceState::BusOff => return Err(CanIoError::BusOff),
                CanIfaceState::Active => {}
            }
            match self.rx.recv().await {
                Some(frame) => Ok(frame),
                None => Err(CanIoError::Closed),
            }
        }
    }

    fn send_classical(
        &mut self,
        frame: &CanData,
    ) -> impl core::future::Future<Output = Result<(), CanIoError>> + Send + '_ {
        let owned = *frame;
        let tx = self.tx.clone();
        let inner = Arc::clone(&self.inner);
        async move {
            let state = inner.lock().expect("mock state lock not poisoned").state;
            match state {
                CanIfaceState::Closed => return Err(CanIoError::Closed),
                CanIfaceState::BusOff => return Err(CanIoError::BusOff),
                CanIfaceState::Active => {}
            }
            if !matches!(owned.kind, CanFrameKind::Classical) {
                return Err(CanIoError::Io(
                    "send_classical called with non-classical frame".to_string(),
                ));
            }
            tx.send(CanFrame::Data(owned))
                .map_err(|_| CanIoError::Closed)?;
            inner
                .lock()
                .expect("mock state lock not poisoned")
                .send_count += 1;
            Ok(())
        }
    }

    fn send_fd(
        &mut self,
        frame: &CanData,
    ) -> impl core::future::Future<Output = Result<(), CanIoError>> + Send + '_ {
        let owned = *frame;
        let tx = self.tx.clone();
        let inner = Arc::clone(&self.inner);
        async move {
            let state = inner.lock().expect("mock state lock not poisoned").state;
            match state {
                CanIfaceState::Closed => return Err(CanIoError::Closed),
                CanIfaceState::BusOff => return Err(CanIoError::BusOff),
                CanIfaceState::Active => {}
            }
            if !matches!(owned.kind, CanFrameKind::Fd) {
                return Err(CanIoError::Io(
                    "send_fd called with non-FD frame".to_string(),
                ));
            }
            tx.send(CanFrame::Data(owned))
                .map_err(|_| CanIoError::Closed)?;
            inner
                .lock()
                .expect("mock state lock not poisoned")
                .send_count += 1;
            Ok(())
        }
    }

    fn reopen(&mut self) -> impl core::future::Future<Output = Result<(), CanIoError>> + Send + '_ {
        let inner = Arc::clone(&self.inner);
        async move {
            let mut guard = inner.lock().expect("mock state lock not poisoned");
            guard.state = CanIfaceState::Active;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::CanErrorKind;
    use crate::routing::{CanFdFlags, CanId};

    #[tokio::test]
    async fn loopback_round_trip_classical() {
        let mut m = MockCanInterface::new(CanIface::new("vcan0").unwrap());
        let frame = CanData::new(
            CanId::standard(0x100).unwrap(),
            CanFrameKind::Classical,
            CanFdFlags::empty(),
            &[1, 2, 3, 4],
        )
        .unwrap();
        m.send_classical(&frame).await.unwrap();
        let received = m.recv().await.unwrap();
        match received {
            CanFrame::Data(d) => assert_eq!(d.payload(), &[1, 2, 3, 4]),
            other => panic!("unexpected frame: {other:?}"),
        }
    }

    #[tokio::test]
    async fn injected_error_surfaces_on_recv() {
        let mut m = MockCanInterface::new(CanIface::new("vcan0").unwrap());
        m.inject_error(CanErrorKind::BusOff);
        let received = m.recv().await.unwrap();
        assert!(matches!(received, CanFrame::Error(CanErrorKind::BusOff)));
    }

    #[tokio::test]
    async fn apply_filter_is_recorded() {
        let mut m = MockCanInterface::new(CanIface::new("vcan0").unwrap());
        let filters = vec![CanFilter {
            can_id: 0x123,
            can_mask: 0x7FF,
        }];
        m.apply_filter(&filters).unwrap();
        let state = m.state_handle();
        let guard = state.lock().unwrap();
        assert_eq!(guard.last_applied_filter, filters);
        assert_eq!(guard.apply_filter_count, 1);
    }
}
