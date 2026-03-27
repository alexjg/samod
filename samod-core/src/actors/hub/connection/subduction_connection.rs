use crate::{ConnectionId, ConnectionOwner, UnixTimestamp};

#[derive(Debug, Clone)]
pub(crate) struct SubductionConnection {
    id: ConnectionId,
    /// The dialer or listener that owns this connection.
    owner: ConnectionOwner,
    /// When the connection was created
    #[allow(dead_code)]
    created_at: UnixTimestamp,
    last_received: Option<UnixTimestamp>,
    last_sent: Option<UnixTimestamp>,
}

impl SubductionConnection {
    pub(crate) fn new(
        id: ConnectionId,
        owner: ConnectionOwner,
        created_at: UnixTimestamp,
    ) -> Self {
        Self {
            id,
            owner,
            created_at,
            last_received: None,
            last_sent: None,
        }
    }
}
