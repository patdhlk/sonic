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
/// `ZeroCopySend + Default + Debug + 'static`.
#[diagnostic::on_unimplemented(
    note = "`{Self}` must derive `ZeroCopySend`, `Debug`, and `Default`.",
    note = "Add `#[derive(Debug, Default, ZeroCopySend)] #[repr(C)]` and the trait will be implemented automatically.",
)]
pub trait Payload: ZeroCopySend + Default + core::fmt::Debug + 'static {}

impl<T> Payload for T where T: ZeroCopySend + Default + core::fmt::Debug + 'static {}
