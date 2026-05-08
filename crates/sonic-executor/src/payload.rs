//! Helper trait that bundles the payload-type bounds required by
//! [`Channel<T>`](crate::Channel) and [`Service<Req, Resp>`](crate::Service).
//!
//! Users don't impl this trait themselves — there's a blanket impl for
//! every `T` that satisfies the underlying iceoryx2 + sonic requirements.
//! It exists purely so the compiler can show a clear error message when
//! a user tries to use a type that's missing one of the required bounds.

use iceoryx2::prelude::ZeroCopySend;

/// Marker trait for types that can be carried over a [`Channel<T>`](crate::Channel)
/// or [`Service<Req, Resp>`](crate::Service) — i.e., types that are
/// `ZeroCopySend + Debug + 'static`.
///
/// `Default` is **not** required by the trait; it is only needed by
/// [`Publisher::loan_send`](crate::Publisher::loan_send), which initialises the
/// shared-memory slot via `T::default()` before handing it to the caller's
/// closure. If your type does not implement `Default`, use
/// [`Publisher::loan`](crate::Publisher::loan) instead.
#[diagnostic::on_unimplemented(
    note = "`{Self}` must derive `ZeroCopySend` and `Debug`.",
    note = "Add `#[derive(Debug, ZeroCopySend)] #[repr(C)]` and the trait will be implemented automatically."
)]
pub trait Payload: ZeroCopySend + core::fmt::Debug + 'static {}

impl<T> Payload for T where T: ZeroCopySend + core::fmt::Debug + 'static {}
