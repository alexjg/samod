use crate::{ConnectionId, PeerId, UnixTimestamp, actors::hub::HubResults};

/// A handle for sending messages on a connection that automatically tracks
/// the `last_sent` timestamp.
///
/// All outgoing messages for a connection should go through this struct
/// rather than calling `HubResults::send` directly, so that timestamp
/// tracking cannot be forgotten.
pub(crate) struct ConnOut<'a> {
    conn_id: ConnectionId,
    now: UnixTimestamp,
    last_sent: Option<UnixTimestamp>,
    out: &'a mut HubResults,
}

impl<'a> ConnOut<'a> {
    pub(crate) fn new(conn_id: ConnectionId, now: UnixTimestamp, out: &'a mut HubResults) -> Self {
        Self {
            conn_id,
            now,
            last_sent: None,
            out,
        }
    }

    pub(crate) fn send(&mut self, remote_peer_id: Option<&PeerId>, msg: Vec<u8>) {
        tracing::trace!(conn_id=?self.conn_id, remote_peer_id=?remote_peer_id, num_bytes=msg.len(), "sending message");
        self.last_sent = Some(self.now);
        self.out.send(self.conn_id, msg);
    }

    /// Returns the `last_sent` timestamp if any message was sent through this handle.
    pub(crate) fn last_sent(&self) -> Option<UnixTimestamp> {
        self.last_sent
    }
}
