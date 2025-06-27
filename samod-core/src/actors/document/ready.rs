use automerge::{Automerge, sync};

use crate::{UnixTimestamp, actors::messages::SyncMessage};

use super::peer_doc_connection::{AnnouncePolicy, PeerDocConnection};

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
    ) {
        match msg {
            SyncMessage::Request { data } | SyncMessage::Sync { data } => {
                let msg = match sync::Message::decode(&data) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(err=?e, conn_id=?conn.connection_id, "failed to decode sync message");
                        return;
                    }
                };
                if let Err(e) = conn.receive_sync_message(now, doc, msg) {
                    tracing::warn!(conn_id=?conn.connection_id, err=?e, "failed to process sync message");
                }
            }
            SyncMessage::DocUnavailable => {
                tracing::debug!("received doc-unavailable message whilst we have a doc");
            }
        }
    }

    pub(crate) fn generate_sync_message(
        &mut self,
        now: UnixTimestamp,
        doc: &mut Automerge,
        conn: &mut PeerDocConnection,
    ) -> Option<SyncMessage> {
        if conn.their_heads().is_none() && conn.announce_policy() != AnnouncePolicy::Announce {
            // if we haven't received a sync message from them (indicicated by their heads being None)
            // and the announce policy is set to not announce, then we don't want to send a sync message
            return None;
        }
        conn.generate_sync_message(now, doc)
            .map(|msg| SyncMessage::Sync { data: msg.encode() })
    }
}
