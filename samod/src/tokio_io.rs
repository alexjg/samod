use std::net::SocketAddr;

use futures::FutureExt;

use crate::{Dialer, Transport};

/// A [`Dialer`] that connects to a TCP endpoint
///
/// The dialer can be constructed from either a `SocketAddr` or a host/port
/// pair. In the latter case, the host will be resolved to an IP address using
/// [`tokio::net::lookup_host`].
///
/// The dialer will use a simple length delimited framing to connect to the
/// other end. Accepting connections on the other end should be done using
/// [`Transport::from_tokio_io`].
///
/// # Example
///
/// ```rust
/// use std::sync::Arc;
/// use samod::{Repo, Transport, BackoffConfig};
/// use samod::tokio_io::TcpDialer;
/// use tokio::net::TcpListener;
///
/// # async fn example() {
///
/// // First start a server
/// let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
/// let port = listener.local_addr().unwrap().port();
/// tokio::spawn(async move {
///     let repo: Repo = Repo::build_tokio().load().await;
///     let acceptor = repo.make_acceptor(url::Url::parse("tcp://someserver").unwrap()).unwrap();
///     let (io, _) = listener.accept().await.unwrap();
///     acceptor.accept(Transport::from_tokio_io(io));
/// });
///
/// // Now make a client which dials the server
/// let repo: Repo = Repo::build_tokio().load().await;
/// let dialer = TcpDialer::new_host_port("0.0.0.0", port);
/// repo.dial(BackoffConfig::default(), Arc::new(dialer)).unwrap();
/// # }
/// ```
pub struct TcpDialer {
    host: Host,
}

impl TcpDialer {
    /// Create a dialer which will attempt to connect to the given socket address
    pub fn new_socket_addr(addr: SocketAddr) -> Self {
        Self {
            host: Host::SocketAddr(addr),
        }
    }

    /// Create a dialer which will first attempt to resolve the given host, and
    /// then connect to the resolved address and port.
    pub fn new_host_port<S: AsRef<str>>(host: S, port: u16) -> Self {
        Self {
            host: Host::HostPort(host.as_ref().to_string(), port),
        }
    }
}

#[derive(Clone)]
enum Host {
    SocketAddr(SocketAddr),
    HostPort(String, u16),
}

impl Dialer for TcpDialer {
    fn url(&self) -> url::Url {
        match self.host {
            Host::SocketAddr(addr) => {
                url::Url::parse(&format!("tcp://{}:{}", addr.ip(), addr.port())).unwrap()
            }
            Host::HostPort(ref host, port) => {
                url::Url::parse(&format!("tcp://{}:{}", host, port)).unwrap()
            }
        }
    }

    fn connect(
        &self,
    ) -> futures::future::BoxFuture<
        'static,
        Result<crate::Transport, Box<dyn std::error::Error + Send + Sync + 'static>>,
    > {
        let host = self.host.clone();
        async move {
            let addr = match host {
                Host::SocketAddr(addr) => addr,
                Host::HostPort(ref host, port) => {
                    tracing::trace!("resolving {}:{}", host, port);
                    let mut addrs = tokio::net::lookup_host((host.as_str(), port))
                        .await?
                        .filter(|a| a.is_ipv4());
                    let Some(addr) = addrs.next() else {
                        return Err::<_, Box<dyn std::error::Error + Send + Sync + 'static>>(
                            Box::new(std::io::Error::new(
                                std::io::ErrorKind::NotFound,
                                format!("no addresses found for {}:{}", host, port),
                            )),
                        );
                    };
                    tracing::trace!("resolved {}:{} to {}", host, port, addr);
                    addr
                }
            };
            tracing::debug!("dialing {}:{}", addr.ip(), addr.port());
            let io = tokio::net::TcpStream::connect(addr).await?;
            let transport = Transport::from_tokio_io(io);
            Ok(transport)
        }
        .boxed()
    }
}
