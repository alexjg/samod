# samod-cli

A command line tool for running a [`samod`](../samod) sync server. It accepts inbound connections from other automerge-repo peers, dials outbound peers, and persists documents either to the filesystem or in-memory.

## Install

From the workspace root:

```sh
cargo install --path samod-cli
```

## Usage

The only subcommand is `serve`:

```sh
samod-cli serve \
    --listeners ws://0.0.0.0:8080 \
    --listeners tcp://0.0.0.0:8081 \
    --peers wss://sync.example.com \
    --storage-dir ./data
```

### Options

- `-l`, `--listeners <URL>` — Address to listen on. May be repeated. Schemes: `tcp://`, `ws://`, `wss://`. The host must be an IP address or `localhost`.
- `-p`, `--peers <URL>` — Outbound peer to dial. May be repeated. Schemes: `tcp://`, `ws://`, `wss://`.
- `-s`, `--storage-dir <PATH>` — Directory for on-disk storage. If omitted, an ephemeral in-memory store is used.
- `--peer-id-prefix <PREFIX>` — Prefix to use for this server's peer ID. The full peer ID is `<prefix>-<uuid>`. If omitted, a random peer ID is generated.
- `--relay-peer-id-prefixes <PREFIX>` — Announce documents to peers whose peer ID starts with one of these prefixes (e.g. `storage-server`). May be repeated. Other peers will only receive documents they explicitly request.
- `--otel-endpoint <URL>` — OTLP HTTP endpoint to export metrics to (e.g. `http://localhost:4318` for [otel-tui](https://github.com/ymtdzzz/otel-tui), or `http://localhost:9090/api/v1/otlp` for Prometheus).
- `--log-format <text|json>` — Log output format. Defaults to `text`.

Log level is controlled via the `RUST_LOG` environment variable and defaults to `info`.

## Status

Like the rest of `samod`, this is a work in progress. See the [top-level README](../README.md) for context.
