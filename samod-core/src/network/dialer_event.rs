use url::Url;

use super::DialerId;

/// Events related to dialer lifecycle.
///
/// These events are emitted by the hub to notify the IO layer of significant
/// changes in dialer state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialerEvent {
    /// A dialer has exhausted its retry budget.
    ///
    /// No further dial requests will be emitted for this dialer.
    /// The dialer remains registered but in a terminal `Failed` state.
    /// The IO layer may choose to remove it or present the failure to the user.
    MaxRetriesReached { dialer_id: DialerId, url: Url },
}
