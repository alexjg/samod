/// The access granted to a remote peer for a document.
///
/// These permissions apply to the connection delivering changes, not their
/// original author. Changes rejected on one connection can still arrive through
/// another connection with write access. Local document edits are unaffected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShareAccess {
    /// Do not exchange document changes or ephemera with this peer.
    None,
    /// Send document changes, but ignore incoming changes from this peer.
    /// Ephemeral messages can still flow in both directions.
    ReadOnly,
    /// Exchange document changes and ephemera in both directions.
    ReadWrite,
}

/// How to share a document with a particular remote peer.
///
/// A decision is made before processing document messages and is cached until
/// explicitly updated or the connection closes. A new connection requires a
/// new decision. These permissions are not persisted with the document.
/// The default grants read-write access with announcements enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShareConfig {
    /// Whether the remote peer may read and write this document.
    pub access: ShareAccess,
    /// Proactively announce the document, or request it if we do not have it.
    ///
    /// Disabling announcement does not deny access: an allowed peer can still
    /// initiate sync. Access takes precedence: denied peers are never announced
    /// to, and peers without write access are never used to fetch missing data.
    pub announce: bool,
}

impl ShareConfig {
    /// Deny all sharing, including announcements.
    pub const fn none() -> Self {
        Self {
            access: ShareAccess::None,
            announce: false,
        }
    }

    /// Publish changes without accepting incoming changes. Announce by default.
    pub const fn read_only() -> Self {
        Self {
            access: ShareAccess::ReadOnly,
            announce: true,
        }
    }

    /// Share changes in both directions. Announce by default.
    pub const fn read_write() -> Self {
        Self {
            access: ShareAccess::ReadWrite,
            announce: true,
        }
    }

    /// Enable or disable proactive document announcements and requests.
    pub const fn with_announce(mut self, announce: bool) -> Self {
        self.announce = announce;
        self
    }

    pub(crate) fn can_read(self) -> bool {
        self.access != ShareAccess::None
    }

    pub(crate) fn can_write(self) -> bool {
        self.access == ShareAccess::ReadWrite
    }

    pub(crate) fn can_request(self) -> bool {
        self.can_write() && self.announce
    }
}

impl Default for ShareConfig {
    fn default() -> Self {
        Self::read_write()
    }
}
