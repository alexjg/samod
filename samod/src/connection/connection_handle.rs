use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use futures::channel::oneshot;
use samod_core::ConnectionId;

use crate::{
    ConnFinishedReason, PeerInfo,
    unbounded::{self, UnboundedReceiver, UnboundedSender},
};

// A connection handle is per-connection state which is shared between:
//
// * The Repo::inner struct
// * The io loop
// * Instances of `crate::Connection` which are the public API
//
// The connection handle is created as in Inner::handle_event as soon as a
// `samod_core::CommandResult::CreateConnection` is seen. When it is created
// it owns both the send and receive channels for the messages to be sent
// on this connection. Repo::connect will then retrieve the handle from the
// `Inner` and pass the handle to the io loop, which will take ownership of
// the receive end of the channel using `ConnectionHandle::take_rx`. This
// ordering is necessary because the samod_core::Hub may (in general _will_)
// produce messages for the connection before the receive end has been taken
// and so those messages need to be buffered somewhere.
//
// Once the connection is running on the IO loop then it becomes a place to
// register listeners for events happening on the connection.
#[derive(Clone)]
pub(crate) struct ConnectionHandle {
    id: ConnectionId,
    tx: UnboundedSender<Vec<u8>>,
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    info: Option<PeerInfo>,
    handshake_listeners: Vec<oneshot::Sender<Result<PeerInfo, ConnFinishedReason>>>,
    finished_listeners: Vec<oneshot::Sender<ConnFinishedReason>>,
    finished_reason: Option<ConnFinishedReason>,
    rx: Option<UnboundedReceiver<Vec<u8>>>,
}

impl ConnectionHandle {
    pub(crate) fn new(id: ConnectionId) -> Self {
        let (tx, rx) = unbounded::channel();
        Self {
            id,
            tx,
            inner: Arc::new(RwLock::new(Inner {
                info: None,
                handshake_listeners: Vec::new(),
                finished_listeners: Vec::new(),
                finished_reason: None,
                rx: Some(rx),
            })),
        }
    }

    pub(crate) fn id(&self) -> ConnectionId {
        self.id
    }

    pub(crate) fn info(&self) -> Option<PeerInfo> {
        let inner = Self::read(&self.inner);
        inner.info.clone()
    }

    // Called in the io loop to take ownership of the outbound message channel
    pub(crate) fn take_rx(&self) -> UnboundedReceiver<Vec<u8>> {
        let mut inner = Self::write(&self.inner);
        inner
            .rx
            .take()
            .expect("receiver for connection already taken")
    }

    // Called in Inner::handle_event whenever there are outbound messages to send
    // The other end of the channel is owned by the io loop
    pub(crate) fn send(&self, msg: Vec<u8>) {
        let _ = self.tx.unbounded_send(msg);
    }

    // A future which completes either when the handshake completes, or the
    // connection is finished
    pub(crate) fn handshake_complete(
        &self,
    ) -> impl Future<Output = Result<PeerInfo, ConnFinishedReason>> + 'static {
        let inner = self.inner.clone();
        async move {
            let rx = {
                let mut inner = Self::write(&inner);
                if let Some(finished_reason) = inner.finished_reason.take() {
                    return Err(finished_reason);
                }
                if let Some(info) = &inner.info {
                    return Ok(info.clone());
                }
                let (tx, rx) = oneshot::channel();
                inner.handshake_listeners.push(tx);
                rx
            };
            rx.await.unwrap()
        }
    }

    // A future which completes when the connection is finished
    pub(crate) fn finished(
        &self,
    ) -> impl Future<Output = ConnFinishedReason> + Sync + Send + 'static {
        let inner = self.inner.clone();
        async move {
            {
                let rx = {
                    let mut inner = Self::write(&inner);
                    if let Some(reason) = inner.finished_reason.take() {
                        return reason;
                    }
                    let (tx, rx) = oneshot::channel();
                    inner.finished_listeners.push(tx);
                    rx
                };
                rx.await.unwrap()
            }
        }
    }

    // Called in Inner::handle_event when the handshake is complete
    pub(crate) fn notify_handshake_complete(&self, peer_info: PeerInfo) {
        let mut inner = Self::write(&self.inner);
        inner.info = Some(peer_info.clone());
        for l in inner.handshake_listeners.drain(..) {
            let _ = l.send(Ok(peer_info.clone()));
        }
    }

    // Called in the io loop when the connection has finished
    pub(crate) fn notify_finished(&self, reason: ConnFinishedReason) {
        let mut inner = Self::write(&self.inner);
        inner.finished_reason = Some(reason.clone());
        for l in inner.finished_listeners.drain(..) {
            let _ = l.send(reason.clone());
        }
    }

    fn read(inner: &Arc<RwLock<Inner>>) -> RwLockReadGuard<'_, Inner> {
        match inner.read() {
            Ok(r) => r,
            Err(e) => {
                // We don't care about poisoning because there are no invariants to
                // maintain on the Inner.
                inner.clear_poison();
                e.into_inner()
            }
        }
    }

    fn write(inner: &Arc<RwLock<Inner>>) -> RwLockWriteGuard<'_, Inner> {
        match inner.write() {
            Ok(r) => r,
            Err(e) => {
                // We don't care about poisoning because there are no invariants to
                // maintain on the Inner.
                inner.clear_poison();
                e.into_inner()
            }
        }
    }
}
