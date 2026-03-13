use std::time::Duration;

use automerge::{Automerge, sync};

use crate::{
    UnixTimestamp,
    actors::{
        document::peer_doc_connection::{AnnouncePolicy, PeerDocConnection},
        messages::SyncMessage,
    },
};

#[derive(Debug)]
pub(crate) struct Ready;

impl Ready {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn receive_sync_message(
        &mut self,
        now: UnixTimestamp,
        doc: &mut Automerge,
        conn: &mut PeerDocConnection,
        msg: SyncMessage,
    ) -> Option<Duration> {
        match msg {
            SyncMessage::Request { data } | SyncMessage::Sync { data } => {
                let msg = match sync::Message::decode(&data) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(err=?e, conn_id=?conn.connection_id, "failed to decode sync message");
                        return None;
                    }
                };
                match conn.receive_sync_message(now, doc, msg) {
                    Ok(duration) => Some(duration),
                    Err(e) => {
                        tracing::warn!(conn_id=?conn.connection_id, err=?e, "failed to process sync message");
                        None
                    }
                }
            }
            SyncMessage::DocUnavailable => {
                tracing::debug!("received doc-unavailable message whilst we have a doc");
                None
            }
        }
    }

    pub(crate) fn generate_sync_message(
        &mut self,
        now: UnixTimestamp,
        doc: &mut Automerge,
        conn: &mut PeerDocConnection,
    ) -> Option<(SyncMessage, Duration)> {
        if conn.their_heads().is_none()
            && !conn.has_requested()
            && conn.announce_policy() != AnnouncePolicy::Announce
        {
            // if we haven't received a sync message from them, and they don't
            // already have the document and the announce policy is set to not
            // announce, then we don't want to send a sync message
            return None;
        }
        conn.generate_sync_message(now, doc)
            .map(|(msg, duration)| (SyncMessage::Sync { data: msg.encode() }, duration))
    }
}
