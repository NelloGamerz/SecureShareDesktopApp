//! What a `Vilsend` actually is.
//!
//! A `Vilsend` is a thin object over a backend. The backend seam exists from
//! day one even though only one backend has ever been written, because the
//! whole point of a facade is that the thing behind it can be replaced — and
//! because it makes the *gap* explicit: a `native()` Vilsend is not a
//! differently-configured `in_memory()` one, it is a backend that has not been
//! written.

pub(crate) mod memory;

use std::future::Future;
use std::pin::Pin;

use vilsend_core::VilsendError;

use crate::auth::AuthState;
use crate::events::EventStream;
use crate::handle::TransferHandle;
use crate::request::{ReceiveRequest, SendRequest};

/// A boxed, `Send` future.
///
/// `async fn` in a trait is stable, but a *dyn-safe* one is not, and the
/// backend has to be a trait object so that `Vilsend` can hold one. Boxing is
/// what buys that, and it costs one allocation per call — on a per-transfer
/// path, not a per-byte one, which `04-sdk-cli-mobile-build-plan.md` §3.1
/// already argues is the right place to spend it.
pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Everything the four verbs do.
///
/// There is one implementation. The trait is here because the *second* one —
/// the native backend, wired to real files, the OS keyring and a real
/// transport — is what this phase is a prerequisite for, and because a facade
/// whose backend is a concrete type is not a facade.
pub(crate) trait Backend: Send + Sync + 'static {
    fn send<'a>(
        &'a self,
        request: SendRequest,
    ) -> BoxFuture<'a, Result<TransferHandle, VilsendError>>;

    fn receive<'a>(
        &'a self,
        request: ReceiveRequest,
    ) -> BoxFuture<'a, Result<TransferHandle, VilsendError>>;

    fn auth_state(&self) -> AuthState;

    fn events(&self) -> EventStream;
}
