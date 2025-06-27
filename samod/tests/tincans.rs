#![allow(dead_code)]
use futures::{FutureExt, Sink, SinkExt, Stream, StreamExt, future::join, select};
use samod::{ConnDirection, Samod};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::{CancellationToken, PollSender};

/// A pair of [`TinCan`]s, one for each direction of a connection.
///
/// You know that thing you did as a kid where you connect two tin cans with a wire and then talk
/// into them? This is that, but with `tokio::sync::mpsc::Channel`s.
///
/// ## Example
///
/// ```no_run
/// use futures::{SinkExt, StreamExt};
///
/// // lets say you already have some repos around
/// let repo1: Samod = todo!();
/// let repo2: Samod = todo!();
///
/// // make some tincans and use them to connect the repos to each other
/// let TinCans{
///    left: TinCan{send: mut left_send, recv: mut left_recv, ..},
///    right: TinCan{send: mut right_send, recv: mut right_recv, ..},
/// };
///
/// repo1_handle.connect(left_recv, left_send);
/// repo2_handle.connect(right_recv, right_send);
///
/// ```
pub(crate) struct TinCans {
    pub left: TinCan,
    pub right: TinCan,
}

pub(crate) struct TinCanError(String);
impl std::fmt::Display for TinCanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::fmt::Debug for TinCanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for TinCanError {}

/// One side of a connection
pub(crate) struct TinCan {
    /// Send messages to the other side of the connection
    pub send: Box<dyn Send + Unpin + Sink<Vec<u8>, Error = TinCanError>>,
    /// Receive messages from the other side of the connection
    pub recv: Box<dyn Send + Unpin + Stream<Item = Result<Vec<u8>, TinCanError>>>,
}

/// Create a pair of [`TinCan`]s, one for each direction of a connection.
pub(crate) fn tincans() -> TinCans {
    let (left_send, right_recv) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
    let (right_send, left_recv) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
    TinCans {
        left: TinCan {
            send: Box::new(PollSender::new(left_send).sink_map_err(|e| {
                TinCanError(format!(
                    "unexpected failure to send on blocking channel: {e:?}"
                ))
            })),
            recv: Box::new(ReceiverStream::new(left_recv).map(Ok)),
        },
        right: TinCan {
            send: Box::new(PollSender::new(right_send).sink_map_err(|e| {
                TinCanError(format!(
                    "unexpected failure to send on blocking channel: {e:?}"
                ))
            })),
            recv: Box::new(ReceiverStream::new(right_recv).map(Ok)),
        },
    }
}

pub(crate) struct Connected {
    cancel: CancellationToken,
    left: JoinHandle<()>,
    right: JoinHandle<()>,
}

impl Connected {
    pub async fn disconnect(self) {
        self.cancel.cancel();
        let (left_finished, right_finished) = join(self.left, self.right).await;
        left_finished.unwrap();
        right_finished.unwrap();
    }
}

pub(crate) fn connect_repos(left: &Samod, right: &Samod) -> Connected {
    // This function connects two samod instances. We want to connect them in a
    // manner which allows us to simulate the loss of a connection. To do this
    // we create a "middle" process, then connect streams from the left and
    // right to this middle process. This means that we can then wait on a
    // cancellation token in the middle process. When the middle process is
    // cancelled it drops the ends of the streams it is holding, which simulates
    // the loss of connection.
    let cancel = CancellationToken::new();
    let TinCans {
        left: TinCan {
            send: left_send,
            recv: left_recv,
        },
        right:
            TinCan {
                send: mut middle_send_left,
                recv: mut middle_recv_left,
            },
    } = tincans();

    let TinCans {
        left: TinCan {
            send: right_send,
            recv: right_recv,
        },
        right:
            TinCan {
                send: mut middle_send_right,
                recv: mut middle_recv_right,
            },
    } = tincans();

    let middle_cancel = cancel.clone();
    let _middle = tokio::spawn(async move {
        // Pull stuff from right to left and left to right
        loop {
            select! {
                next_left_to_middle = middle_recv_left.next().fuse() => {
                    let Some(Ok(msg)) = next_left_to_middle else {
                        break;
                    };
                    middle_send_right.send(msg).await.unwrap();
                }
                next_right_to_middle = middle_recv_right.next().fuse() => {
                    let Some(Ok(msg)) = next_right_to_middle else {
                        break;
                    };
                    middle_send_left.send(msg).await.unwrap();
                },
                _ = middle_cancel.cancelled().fuse() => {
                    // This drops the middle ends of the streams, which will
                    // cause the left and right streams ends to drop.
                    break;
                }
            }
        }
        tracing::info!("middle task finished");
    });

    let left = left.clone();
    let left_fut = tokio::spawn(async move {
        let drive_conn = left.connect(left_recv, left_send, ConnDirection::Outgoing);
        drive_conn.await;
        tracing::info!("left finished");
    });
    let right = right.clone();
    let right_fut = tokio::spawn(async move {
        let drive_conn = right.connect(right_recv, right_send, ConnDirection::Incoming);
        drive_conn.await;
        tracing::info!("right finished");
    });
    Connected {
        cancel,
        left: left_fut,
        right: right_fut,
    }
}
