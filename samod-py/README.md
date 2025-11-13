# samod-py

Python bindings for Automerge via [samod](https://github.com/alexjg/samod).

## Development

```bash
pip install maturin
cd samod-py
maturin develop --release
```

## Usage

```python
import asyncio
import samod

async def main():
    repo = samod.Repo()
    print(f"Peer ID: {repo.peer_id()}")

    # Connect to sync server
    await repo.connect_websocket("ws://localhost:3030")

    # Create a document
    doc = await repo.create()
    await doc.set_string("title", "Hello World")

    # Read it back
    title = await doc.get_string("title")
    print(f"Title: {title}")

    url = await doc.url()
    print(f"Autommerge URL: {doc_id}")

    # Find an existing document
    existing = await repo.find("automerge:...")
    if existing:
        keys = await existing.get_keys()
        print(f"Keys: {keys}")

    await repo.stop()

asyncio.run(main())
```

## API

### `Repo`

Repository managing documents, storage, and sync.

**Methods:**

- `Repo()` - Create repo with in-memory storage
- `peer_id() -> str` - Get this peer's ID
- `async connect_websocket(url: str)` - Connect to WebSocket sync server
- `async when_connected(peer_id: str)` - Block until connected to specific peer
- `async find(doc_id: str) -> Optional[DocHandle]` - Find document by AutomergeUrl
- `async create() -> DocHandle` - Create new document
- `async stop()` - Stop repo and abort background connections

**Connection handling:**

`connect_websocket()` spawns connections in the background. Use `when_connected(peer_id)` to wait for a specific peer, or `asyncio.sleep()` as a workaround when the peer ID is unknown.

### `DocHandle`

Handle to an Automerge document.

**Methods:**

- `async document_id() -> str` - Get document ID (AutomergeUrl)
- `async dump() -> bytes` - Serialize document to bytes
- `async get_keys() -> List[str]` - List all root-level keys
- `async get_string(key: str) -> Optional[str]` - Get string field
- `async set_string(key: str, value: str)` - Set string field

## Development

```bash
# Development build
maturin develop

# Release build
maturin develop --release

# Build wheel
maturin build --release
```

## Architecture Notes

This package relies on PyO3 and `pyo3-async-runtimes` to bind the core automerge/samod Rust code in Python with native async await support.

## License

MIT
