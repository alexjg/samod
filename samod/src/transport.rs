use std::pin::Pin;

use futures::{Sink, stream::BoxStream};

pub type BoxSink<T> = Pin<
    Box<
        dyn Sink<T, Error = Box<dyn std::error::Error + Send + Sync + 'static>>
            + Send
            + 'static
            + Unpin,
    >,
>;

/// A connected transport (stream + sink), returned by dialers or passed to
/// listeners
///
/// A `Transport` wraps a pair of byte streams that can be used to communicate
/// with a remote peer.
pub struct Transport {
    pub(crate) stream:
        BoxStream<'static, Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync + 'static>>>,
    pub(crate) sink: BoxSink<Vec<u8>>,
}

impl Transport {
    /// Create a new transport from a stream and sink.
    ///
    /// # Arguments
    ///
    /// * `stream` - The inbound byte stream.
    /// * `sink` - The outbound byte sink.
    pub fn new<Str, Snk, RecvErr, SendErr>(stream: Str, sink: Snk) -> Self
    where
        RecvErr: std::error::Error + Send + Sync + 'static,
        SendErr: std::error::Error + Send + Sync + 'static,
        Str: futures::Stream<Item = Result<Vec<u8>, RecvErr>> + Send + 'static + Unpin,
        Snk: futures::Sink<Vec<u8>, Error = SendErr> + Send + 'static + Unpin,
    {
        use futures::{SinkExt, StreamExt, TryStreamExt};
        Transport {
            stream: stream
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)
                .boxed(),
            sink: Box::pin(sink.sink_map_err(|e| {
                Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>
            })),
        }
    }
}

pub mod channel {
    use std::convert::Infallible;

    use futures::{SinkExt, StreamExt};
    use url::Url;

    use crate::{
        AcceptorHandle, Dialer, Transport,
        unbounded::{self},
    };
    pub use unbounded::ChanErr;

    pub struct ChannelDialer {
        url: Url,
        acceptor: AcceptorHandle,
    }

    impl ChannelDialer {
        pub fn new(acceptor: AcceptorHandle) -> ChannelDialer {
            let random_id: u64 = rand::random();

            ChannelDialer {
                url: Url::parse(&format!("channel://{}", random_id)).unwrap(),
                acceptor,
            }
        }
    }

    impl Dialer for ChannelDialer {
        fn url(&self) -> url::Url {
            self.url.clone()
        }

        fn connect(
            &self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            Transport,
                            Box<dyn std::error::Error + Send + Sync + 'static>,
                        >,
                    > + Send,
            >,
        > {
            let acceptor = self.acceptor.clone();
            Box::pin(async move {
                // dialer writes to dialer_tx; acceptor reads from dialer_rx
                let (dialer_tx, dialer_rx) = unbounded::channel::<Vec<u8>>();
                // acceptor writes to acceptor_tx; dialer reads from acceptor_rx
                let (acceptor_tx, acceptor_rx) = unbounded::channel::<Vec<u8>>();
                acceptor
                    .accept(Transport::new(
                        dialer_rx.map(Ok::<_, Infallible>),
                        acceptor_tx,
                    ))
                    .map_err(|e| {
                        Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>
                    })?;
                Ok(Transport {
                    stream: Box::pin(acceptor_rx.map(Ok)),
                    sink: Box::pin(dialer_tx.with(|i| futures::future::ready(Ok(i)))),
                })
            })
        }
    }
}
