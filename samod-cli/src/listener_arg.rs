use std::net::{IpAddr, SocketAddr};

#[derive(Debug, Clone)]
pub(crate) enum ListenerArg {
    Tcp(SocketAddr),
    WebSocket(SocketAddr),
}

impl std::str::FromStr for ListenerArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url: url::Url = s.parse().map_err(|e: url::ParseError| e.to_string())?;

        let host = url.host().ok_or("URL must contain a host")?;
        let ip: IpAddr = match host {
            url::Host::Ipv4(addr) => addr.into(),
            url::Host::Ipv6(addr) => addr.into(),
            url::Host::Domain("localhost") => IpAddr::from([127, 0, 0, 1]),
            url::Host::Domain(other) => {
                return Err(format!(
                    "expected an IP address or 'localhost', not '{other}'"
                ));
            }
        };

        let port = url
            .port_or_known_default()
            .ok_or("URL must contain a port")?;
        let addr = SocketAddr::new(ip, port);

        match url.scheme() {
            "tcp" => Ok(ListenerArg::Tcp(addr)),
            "ws" | "wss" => Ok(ListenerArg::WebSocket(addr)),
            other => Err(format!(
                "unsupported scheme: '{other}', expected 'tcp', 'ws', or 'wss'"
            )),
        }
    }
}
