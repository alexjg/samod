use url::Url;

use crate::{ConnectionId, DialerId, UnixTimestamp, network::BackoffConfig};

/// Internal state for a dialer tracked by the hub.
///
/// A dialer actively establishes outgoing connections and automatically
/// reconnects with exponential backoff when a connection is lost.
/// At most one active connection at a time.
#[derive(Debug, Clone)]
pub(crate) struct DialerState {
    #[expect(dead_code)]
    pub(crate) dialer_id: DialerId,
    pub(crate) url: Url,
    pub(crate) backoff_config: BackoffConfig,
    pub(crate) status: DialerStatus,
    pub(crate) attempts: u32,
}

/// The status of a dialer.
#[derive(Debug, Clone)]
pub(crate) enum DialerStatus {
    /// We need the IO layer to establish a transport.
    NeedTransport,
    /// Transport establishment is in progress.
    TransportPending,
    /// An active connection exists.
    Connected { connection_id: ConnectionId },
    /// Waiting for backoff timer before next retry.
    WaitingToRetry { retry_at: UnixTimestamp },
    /// Permanently failed (max retries exceeded).
    Failed,
}

impl DialerState {
    pub(crate) fn new(dialer_id: DialerId, url: Url, backoff: BackoffConfig) -> Self {
        Self {
            dialer_id,
            url,
            backoff_config: backoff,
            status: DialerStatus::NeedTransport,
            attempts: 0,
        }
    }

    /// Handle a connection being lost for this dialer.
    ///
    /// Returns `NotOurs` if the given `connection_id` doesn't match the
    /// dialer's current active connection.
    pub(crate) fn handle_connection_lost<R: rand::Rng>(
        &mut self,
        rng: &mut R,
        now: UnixTimestamp,
        connection_id: ConnectionId,
    ) -> ConnectionLostOutcome {
        // Only handle if this is our active connection
        if !matches!(self.status, DialerStatus::Connected { connection_id: cid } if cid == connection_id)
        {
            return ConnectionLostOutcome::NotOurs;
        }

        self.attempts += 1;

        // Check if we've exceeded max retries
        if let Some(max) = self.backoff_config.max_retries
            && self.attempts > max
        {
            self.status = DialerStatus::Failed;
            return ConnectionLostOutcome::MaxRetriesReached;
        }

        // Compute backoff with jitter
        let retry_at = compute_retry_time(rng, now, &self.backoff_config, self.attempts);
        self.status = DialerStatus::WaitingToRetry { retry_at };
        ConnectionLostOutcome::WillRetry { retry_at }
    }

    /// Handle a transport establishment failure.
    pub(crate) fn handle_dial_failed<R: rand::Rng>(
        &mut self,
        rng: &mut R,
        now: UnixTimestamp,
    ) -> ConnectionLostOutcome {
        if !matches!(self.status, DialerStatus::TransportPending) {
            return ConnectionLostOutcome::NotOurs;
        }

        self.attempts += 1;

        if let Some(max) = self.backoff_config.max_retries
            && self.attempts > max
        {
            self.status = DialerStatus::Failed;
            return ConnectionLostOutcome::MaxRetriesReached;
        }

        let retry_at = compute_retry_time(rng, now, &self.backoff_config, self.attempts);
        self.status = DialerStatus::WaitingToRetry { retry_at };
        ConnectionLostOutcome::WillRetry { retry_at }
    }

    /// Associate a connection with this dialer (called during create_dialer_connection).
    ///
    /// Returns `true` if the association succeeded (dialer was in TransportPending state).
    pub(crate) fn set_connected(&mut self, connection_id: ConnectionId) -> bool {
        if !matches!(self.status, DialerStatus::TransportPending) {
            return false;
        }
        self.status = DialerStatus::Connected { connection_id };
        true
    }

    /// Reset backoff counter on successful handshake.
    pub(crate) fn reset_backoff(&mut self) {
        self.attempts = 0;
    }

    /// Check if the retry timer has expired. If so, transitions to NeedTransport
    /// and returns true.
    pub(crate) fn check_retry(&mut self, now: UnixTimestamp) -> bool {
        if let DialerStatus::WaitingToRetry { retry_at } = self.status
            && now >= retry_at
        {
            self.status = DialerStatus::NeedTransport;
            return true;
        }
        false
    }

    /// Transition from NeedTransport to TransportPending.
    /// Returns true if the transition succeeded.
    pub(crate) fn mark_transport_pending(&mut self) -> bool {
        if matches!(self.status, DialerStatus::NeedTransport) {
            self.status = DialerStatus::TransportPending;
            true
        } else {
            false
        }
    }

    /// Get the active connection ID, if connected.
    pub(crate) fn active_connection(&self) -> Option<ConnectionId> {
        if let DialerStatus::Connected { connection_id } = &self.status {
            Some(*connection_id)
        } else {
            None
        }
    }
}

pub(crate) enum ConnectionLostOutcome {
    /// This connection didn't belong to this dialer.
    NotOurs,
    /// Dialer: will retry at the given time.
    WillRetry { retry_at: UnixTimestamp },
    /// Dialer: max retries reached, dialer is now Failed.
    MaxRetriesReached,
}

/// Compute the next retry time using exponential backoff with jitter.
fn compute_retry_time<R: rand::Rng>(
    rng: &mut R,
    now: UnixTimestamp,
    config: &BackoffConfig,
    attempts: u32,
) -> UnixTimestamp {
    use std::time::Duration;

    // delay = min(initial_delay * 2^attempts, max_delay)
    // Use saturating arithmetic to avoid overflow
    let base_delay = config
        .initial_delay
        .saturating_mul(2u32.saturating_pow(attempts.saturating_sub(1)));
    let capped_delay = std::cmp::min(base_delay, config.max_delay);

    // jittered_delay = delay * random(0.5, 1.0)
    let jitter: f64 = rng.random_range(0.5..1.0);
    let jittered_millis = (capped_delay.as_millis() as f64 * jitter) as u64;
    let jittered_delay = Duration::from_millis(jittered_millis);

    now + jittered_delay
}
