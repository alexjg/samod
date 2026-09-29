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
    pub(crate) expected_peer_id: Option<crate::PeerId>,
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
            expected_peer_id: None,
            stream: stream
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)
                .boxed(),
            sink: Box::pin(sink.sink_map_err(|e| {
                Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>
            })),
        }
    }

    /// Bind this transport to an externally authenticated peer identity.
    ///
    /// ## Arguments
    ///
    /// - `peer_id` - The peer ID that this transport is expected to be associated with.
    ///
    /// ## Details
    ///
    /// The samod network protocol begins with a handshake where each peer
    /// introduces the peer ID it will use to identify itself going forward.
    /// This peer ID is what is used in the [`AnnouncePolicy`] to decide
    /// whether a given peer should have access to a document.
    ///
    /// By default samod will accept whatever peer ID the other ID claims to
    /// have. This is obviously insecure. If you have externally authenticated
    /// the peer (for example, via a header in a websocket upgrade) then you
    /// can pass the expected peer ID to this method. Samod will ensure that
    /// the peer ID the other side claims to have matches the expected peer ID
    /// and close the connection if it does not.
    pub fn with_expected_peer_id(mut self, peer_id: crate::PeerId) -> Self {
        self.expected_peer_id = Some(peer_id);
        self
    }

    /// Create a transport which uses a simple length-delimited framing protocol
    /// over a Tokio AsyncRead/AsyncWrite stream. This should be used to accept
    /// connections from the [`TcpDialer`](crate::tokio_io::TcpDialer). See the
    /// documentation for that type for examples.
    #[cfg(feature = "tokio")]
    pub fn from_tokio_io<S>(io: S) -> Self
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
    {
        use futures::{SinkExt, StreamExt, TryStreamExt};
        use tokio_util::codec::{Framed, LengthDelimitedCodec};

        // The default Tokio frame size is 8mb, this increases it to 8gb. Documents above that size won't sync.
        let codec = LengthDelimitedCodec::builder()
            .max_frame_length(8 * 1024 * 1024 * 1024)
            .new_codec();

        let framed = Framed::new(io, codec);
        let (msg_sink, msg_stream) = framed.split();
        let msg_stream = msg_stream.map(|result| result.map(|b| b.to_vec()));
        let msg_sink = msg_sink.with(|msg: Vec<u8>| {
            futures::future::ready(Ok::<_, std::io::Error>(bytes::Bytes::from(msg)))
        });
        Transport {
            expected_peer_id: None,
            stream: msg_stream
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)
                .boxed(),
            sink: Box::pin(msg_sink.sink_map_err(|e| {
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
        type Error = Infallible;

        fn url(&self) -> url::Url {
            self.url.clone()
        }

        fn connect(
            &self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<Transport, crate::DialError<Self::Error>>>
                    + Send,
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
                    .map_err(crate::DialError::transient)?;
                Ok(Transport {
                    expected_peer_id: None,
                    stream: Box::pin(acceptor_rx.map(Ok)),
                    sink: Box::pin(dialer_tx.with(|i| futures::future::ready(Ok(i)))),
                })
            })
        }
    }
}
