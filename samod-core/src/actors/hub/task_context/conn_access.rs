use std::{cell::RefCell, rc::Rc};

use crate::{
    ConnectionId, UnixTimestamp,
    actors::hub::{State, connection::ReceiveEvent},
    network::wire_protocol::WireMessage,
};

use super::IoAccess;

pub(crate) struct ConnectionAccess<'a> {
    pub(super) now: UnixTimestamp,
    pub(super) io: IoAccess,
    pub(super) state: &'a Rc<RefCell<State>>,
    pub(super) conn_id: ConnectionId,
}

impl ConnectionAccess<'_> {
    pub(crate) fn receive(&self, msg: WireMessage) -> Vec<ReceiveEvent> {
        let mut state = self.state.borrow_mut();
        let conn = state.get_connection_mut(&self.conn_id).unwrap();
        conn.receive_msg(self.io.clone(), self.now, msg)
    }
}
