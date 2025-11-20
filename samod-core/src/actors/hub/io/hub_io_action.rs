use crate::ConnectionId;

#[derive(Debug, Clone)]
pub enum HubIoAction {
    Send {
        connection_id: ConnectionId,
        msg: Vec<u8>,
    },

    Disconnect {
        connection_id: ConnectionId,
    },
}
