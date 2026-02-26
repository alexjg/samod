#[derive(Debug, Clone)]
pub(crate) enum PeerArg {
    Tcp { host: String, port: u16 },
    WebSocket(url::Url),
}

impl std::str::FromStr for PeerArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url: url::Url = s.parse().map_err(|e: url::ParseError| e.to_string())?;

        match url.scheme() {
            "tcp" => {
                let host = url.host_str().ok_or("URL must contain a host")?.to_string();
                let port = url.port().ok_or("URL must contain a port")?;
                Ok(PeerArg::Tcp { host, port })
            }
            "ws" | "wss" => Ok(PeerArg::WebSocket(url)),
            other => Err(format!(
                "unsupported scheme: '{other}', expected 'tcp', 'ws', or 'wss'"
            )),
        }
    }
}
