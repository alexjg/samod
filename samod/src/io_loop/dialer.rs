use std::sync::Arc;

use futures::future::BoxFuture;

use crate::{ConnectionId, DialError, Dialer, DialerHandle, PeerInfo, Transport};

pub(crate) enum ErasedDialError {
    Transient(Box<dyn std::error::Error + Send + Sync + 'static>),
    Permanent(String),
}

/// Object-safe representation of a dialer and its typed handle.
///
/// `Repo` supports dialers with different permanent error types, while its
/// internal registry must have a single value type. This trait erases that
/// generic parameter and forwards lifecycle notifications to the corresponding
/// typed handle.
pub(crate) trait ErasedDialer: Send + Sync {
    fn connect(&self) -> BoxFuture<'static, Result<Transport, ErasedDialError>>;
    fn notify_connected(&self, peer_info: PeerInfo, connection_id: ConnectionId);
    fn notify_disconnected(&self, reason: String);
    fn notify_reconnecting(&self, attempt: u32);
    fn notify_max_retries_reached(&self);
}

struct DialerAdapter<D: Dialer + ?Sized> {
    dialer: Arc<D>,
    handle: DialerHandle<D::Error>,
}

impl<D: Dialer + ?Sized> ErasedDialer for DialerAdapter<D> {
    fn connect(&self) -> BoxFuture<'static, Result<Transport, ErasedDialError>> {
        let connect = self.dialer.connect();
        let handle = self.handle.clone();
        Box::pin(async move {
            match connect.await {
                Ok(transport) => Ok(transport),
                Err(DialError::TransientFailure(error)) => Err(ErasedDialError::Transient(error)),
                Err(DialError::PermanentFailure(error)) => {
                    let message = error.to_string();
                    handle.notify_permanent_failure(error);
                    Err(ErasedDialError::Permanent(message))
                }
            }
        })
    }

    fn notify_connected(&self, peer_info: PeerInfo, connection_id: ConnectionId) {
        self.handle.notify_connected(peer_info, connection_id);
    }

    fn notify_disconnected(&self, reason: String) {
        self.handle.notify_disconnected(reason);
    }

    fn notify_reconnecting(&self, attempt: u32) {
        self.handle.notify_reconnecting(attempt);
    }

    fn notify_max_retries_reached(&self) {
        self.handle.notify_max_retries_reached();
    }
}

/// Type alias for a shared, type-erased dialer.
pub(crate) type DynDialer = Arc<dyn ErasedDialer>;

pub(crate) fn erase<D: Dialer + ?Sized>(
    dialer: Arc<D>,
    handle: DialerHandle<D::Error>,
) -> DynDialer {
    Arc::new(DialerAdapter { dialer, handle })
}
