use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use clap::Parser;

mod listener_arg;
use listener_arg::ListenerArg;
mod peer_arg;
use peer_arg::PeerArg;
mod otel_observer;
use otel_observer::OtelObserver;
use samod::{
    AcceptorHandle, ConcurrencyConfig, DocumentId, PeerId, Repo,
    storage::{InMemoryStorage, TokioFilesystemStorage},
    websocket::TungsteniteDialer,
};
use tokio::net::TcpListener;

#[derive(clap::Parser)]
pub(crate) struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    Serve(ServeCommand),
}

#[derive(clap::Parser)]
pub(crate) struct ServeCommand {
    #[arg(short, long, help = "URLS to listen on")]
    listeners: Vec<ListenerArg>,
    #[arg(short, long, help = "Peer URLs to connect to")]
    peers: Vec<PeerArg>,
    #[arg(
        short,
        long,
        help = "Path to the directory where samod should store its data"
    )]
    storage_dir: Option<PathBuf>,
    #[arg(
        long,
        help = "Peer ID prefixes to announce documents to (e.g. storage-server for the public sync server"
    )]
    relay_peer_id_prefixes: Vec<String>,
    #[arg(
        long,
        help = "Peer ID prefix to use for this server (e.g. storage-server for the public sync server)"
    )]
    peer_id_prefix: Option<String>,
    #[arg(
        long,
        help = "OTLP HTTP endpoint to export metrics to (e.g. http://localhost:4318 for otel-tui, or http://localhost:9090/api/v1/otlp for Prometheus)"
    )]
    otel_endpoint: Option<String>,
}

#[tokio::main]
async fn main() {
    let Args { command } = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    match command {
        Command::Serve(serve_command) => serve(serve_command).await,
    }
}

fn init_meter_provider(endpoint: &str) -> opentelemetry_sdk::metrics::SdkMeterProvider {
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::metrics::SdkMeterProvider;

    let endpoint = endpoint.strip_suffix('/').unwrap_or(endpoint);
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(format!("{endpoint}/v1/metrics"))
        .build()
        .expect("failed to build OTLP metric exporter");

    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .with_resource(Resource::builder().with_service_name("samod-cli").build())
        .build();

    opentelemetry::global::set_meter_provider(provider.clone());
    provider
}

async fn serve(
    ServeCommand {
        listeners,
        peers,
        storage_dir,
        relay_peer_id_prefixes,
        peer_id_prefix,
        otel_endpoint,
    }: ServeCommand,
) {
    // Set up OpenTelemetry metrics if an endpoint is configured
    let _meter_provider = otel_endpoint.as_deref().map(|endpoint| {
        let provider = init_meter_provider(endpoint);
        tracing::info!("OpenTelemetry metrics exporting to {}", endpoint);
        provider
    });

    let observer = _meter_provider.as_ref().map(|_| {
        let meter = opentelemetry::global::meter("samod-cli");
        OtelObserver::new(&meter)
    });

    let announce_policy = move |_doc_id: DocumentId, peer_id: PeerId| {
        relay_peer_id_prefixes
            .iter()
            .any(|prefix| peer_id.to_string().starts_with(prefix))
    };
    let peer_id = if let Some(prefix) = peer_id_prefix {
        PeerId::from_string(format!("{}-{}", prefix, uuid::Uuid::new_v4()))
    } else {
        PeerId::new_with_rng(&mut rand::rng())
    };
    let threadpool = rayon::ThreadPoolBuilder::new()
        .build()
        .expect("failed to build threadpool");
    let repo = match storage_dir {
        Some(dir) => {
            tracing::info!("using file system storage at {}", dir.display());
            let storage = TokioFilesystemStorage::new(dir);
            let mut builder = samod::Repo::build_tokio()
                .with_storage(storage)
                .with_announce_policy(announce_policy)
                .with_peer_id(peer_id)
                .with_concurrency(ConcurrencyConfig::Threadpool(threadpool));
            if let Some(obs) = observer {
                builder = builder.with_observer(obs);
            }
            builder.load().await
        }
        None => {
            tracing::info!("using ephemeral in-memory storage");
            let storage = InMemoryStorage::new();
            let mut builder = samod::Repo::build_tokio()
                .with_storage(storage)
                .with_announce_policy(announce_policy)
                .with_peer_id(peer_id)
                .with_concurrency(ConcurrencyConfig::Threadpool(threadpool));
            if let Some(obs) = observer {
                builder = builder.with_observer(obs);
            }
            builder.load().await
        }
    };

    for listener in listeners {
        match listener {
            ListenerArg::WebSocket(addr) => {
                tracing::info!("starting websocket listener on {}", addr);
                listen_websocket(repo.clone(), addr).await;
            }
            ListenerArg::Tcp(addr) => {
                tracing::info!("starting tcp listener on {addr}");
                listen_tcp(repo.clone(), addr).await;
            }
        }
    }

    for peer in peers {
        match peer {
            PeerArg::Tcp { host, port } => {
                tracing::info!("creating outbound connection to {}:{}", host, port);
                repo.dial(
                    samod::BackoffConfig::default(),
                    Arc::new(samod::tokio_io::TcpDialer::new_host_port(
                        host.clone(),
                        port,
                    )),
                )
                .inspect_err(|e| {
                    tracing::warn!("error dialing tcp peer {}:{}: {e}", host, port);
                })
                .ok();
            }
            PeerArg::WebSocket(url) => {
                tracing::info!("creating outbound connection to {}", url);
                repo.dial(
                    samod::BackoffConfig::default(),
                    Arc::new(TungsteniteDialer::new(url.clone())),
                )
                .inspect_err(|e| {
                    tracing::warn!("error dialing websocket peer {}: {e}", url);
                })
                .ok();
            }
        }
    }

    // Now wait for termination
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl-c signal");

    // Flush metrics on shutdown
    if let Some(provider) = _meter_provider
        && let Err(e) = provider.shutdown()
    {
        tracing::warn!("error shutting down meter provider: {e}");
    }
}

async fn listen_tcp(repo: Repo, addr: SocketAddr) {
    let url = url::Url::parse(&format!("tcp://{}:{}", addr.ip(), addr.port())).unwrap();
    let Ok(listener) = TcpListener::bind(addr).await.inspect_err(|e| {
        tracing::error!("unable to listen on {url}: {e}");
    }) else {
        return;
    };
    let Ok(acceptor) = repo.make_acceptor(url.clone()).inspect_err(|e| {
        tracing::warn!("error creating acceptor for {url}: {e}");
    }) else {
        return;
    };
    tokio::spawn(async move {
        loop {
            let Ok((io, addr)) = listener.accept().await.inspect_err(|e| {
                tracing::warn!("error accepting tcp connection on {url}: {e}");
            }) else {
                continue;
            };
            tracing::info!("accepted tcp connection from {}", addr);
            if let Err(e) = acceptor.accept_tokio_io(io) {
                tracing::error!(?e, "failed to accept tcp connection from {}", addr);
            }
        }
    });
}

async fn listen_websocket(repo: Repo, addr: SocketAddr) {
    let listener = TcpListener::bind(addr)
        .await
        .expect("unable to bind socket");
    let Ok(acceptor) = repo
        .make_acceptor(url::Url::parse(&format!("ws://{}", addr)).unwrap())
        .inspect_err(|e| {
            tracing::warn!("error creating acceptor for {}: {e}", addr);
        })
    else {
        return;
    };

    let app = axum::Router::new()
        .route("/", axum::routing::get(websocket_handler))
        .with_state(acceptor.clone());
    let server = axum::serve(listener, app).into_future();
    tokio::spawn(server);
}

async fn websocket_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    axum::extract::State(acceptor): axum::extract::State<AcceptorHandle>,
) -> axum::response::Response {
    ws.on_upgrade(|socket| async move {
        if let Err(e) = acceptor.accept_axum(socket) {
            tracing::error!(?e, "failed to accept axum websocket");
        }
    })
}
