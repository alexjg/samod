import express from "express";
import { WebSocketServer } from "ws";
import {
    Chunk,
    Repo,
    RepoConfig,
    StorageAdapterInterface,
    StorageKey,
} from "@automerge/automerge-repo";
import { NodeWSServerAdapter } from "@automerge/automerge-repo-network-websocket";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

class Server {
    #socket: WebSocketServer;

    #server: ReturnType<import("express").Express["listen"]>;
    #storage: InMemoryStorageAdapter;

    #repo: Repo;
    #isWasmMode: boolean;

    constructor(port: number, options: { wasm?: boolean } = {}) {
        this.#isWasmMode = options.wasm || false;
        this.#socket = new WebSocketServer({ noServer: true });

        const PORT = port;
        const app = express();

        // Serve static files for WASM mode
        if (this.#isWasmMode) {
            // Enable CORS for WASM testing
            app.use((req, res, next) => {
                res.header('Access-Control-Allow-Origin', '*');
                res.header('Access-Control-Allow-Headers', 'Content-Type');
                res.header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
                next();
            });

            // Serve WASM test files
            app.use('/wasm', express.static(path.join(__dirname, '../../wasm-tests/src')));
            app.use('/pkg', express.static(path.join(__dirname, '../../wasm-tests/pkg')));
        }

        app.use(express.static("public"));
        this.#storage = new InMemoryStorageAdapter();

        const config: RepoConfig = {
            // network: [new NodeWSServerAdapter(this.#socket) as any],
            network: [new NodeWSServerAdapter(this.#socket as any)],
            storage: this.#storage,
            /** @ts-ignore @type {(import("automerge-repo").PeerId)}  */
            peerId: `storage-server` as PeerId,
            // Since this is a server, we don't share generously — meaning we only sync documents they already
            // know about and can ask for by ID.
            sharePolicy: async () => true,
        };
        const serverRepo = new Repo(config);
        this.#repo = serverRepo;

        app.get("/", (req, res) => {
            const mode = this.#isWasmMode ? ' (WASM mode)' : '';
            res.send(`👍 @automerge/automerge-repo-sync-server is running${mode}`);
        });

        // WASM-specific endpoints
        if (this.#isWasmMode) {
            app.get('/health', (req, res) => {
                res.json({
                    status: 'ok',
                    mode: 'wasm',
                    storage: this.#storage.size(),
                    connections: this.#socket.clients.size
                });
            });

            app.get('/stats', (req, res) => {
                res.json({
                    documentsStored: this.#storage.size(),
                    activeConnections: this.#socket.clients.size,
                    uptime: process.uptime()
                });
            });

            // Testing endpoints for disconnection
            app.post('/test/disconnect-all', (req, res) => {
                let disconnectedCount = 0;
                for (const ws of this.#socket.clients) {
                    if (ws.readyState === ws.OPEN) {
                        ws.close(1000, 'Server-initiated disconnect for testing');
                        disconnectedCount++;
                    }
                }
                res.json({ disconnected: disconnectedCount });
            });

            app.post('/test/disconnect-random', (req, res) => {
                const clients = Array.from(this.#socket.clients).filter(ws => ws.readyState === ws.OPEN);
                if (clients.length === 0) {
                    res.json({ disconnected: 0, error: 'No active connections' });
                    return;
                }

                const randomClient = clients[Math.floor(Math.random() * clients.length)];
                randomClient.close(1000, 'Server-initiated disconnect for testing');
                res.json({ disconnected: 1 });
            });

            // Clear all documents from server storage
            app.post('/test/clear-storage', (req, res) => {
                const previousSize = this.#storage.size();
                this.#storage.clear();
                res.json({ 
                    previousSize, 
                    currentSize: this.#storage.size(),
                    cleared: true 
                });
            });
        }

        this.#server = app.listen(PORT, () => {
            const mode = this.#isWasmMode ? ' (WASM mode)' : '';
            console.log(`Listening on port ${this.#server.address().port}${mode}`);
        });

        this.#server.on("upgrade", (request, socket, head) => {
            console.log(`Upgrading to websocket${this.#isWasmMode ? ' (WASM client)' : ''}`);
            this.#socket.handleUpgrade(request, socket, head, (socket) => {
                this.#socket.emit("connection", socket, request);
            });
        });
    }

    close() {
        this.#storage.log();
        this.#socket.close();
        this.#server.close();
    }
}

class InMemoryStorageAdapter implements StorageAdapterInterface {
    #data: Map<StorageKey, Uint8Array> = new Map();

    async load(key: StorageKey): Promise<Uint8Array | undefined> {
        return this.#data.get(key);
    }
    async save(key: StorageKey, data: Uint8Array): Promise<void> {
        this.#data.set(key, data);
    }
    async remove(key: StorageKey): Promise<void> {
        this.#data.delete(key);
    }
    async loadRange(keyPrefix: StorageKey): Promise<Chunk[]> {
        let result: Chunk[] = [];
        for (const [key, value] of this.#data.entries()) {
            if (isPrefixOf(keyPrefix, key)) {
                result.push({
                    key,
                    data: value,
                });
            }
        }
        return result;
    }

    removeRange(keyPrefix: StorageKey): Promise<void> {
        for (const [key] of this.#data.entries()) {
            if (isPrefixOf(keyPrefix, key)) {
                this.#data.delete(key);
            }
        }
        return Promise.resolve();
    }

    log() {
        console.log(`InMemoryStorageAdapter has ${this.#data.size} items:`);
        for (const [key, value] of this.#data.entries()) {
            console.log(`  ${key.join("/")}: ${value.length} bytes`);
        }
    }

    size(): number {
        return this.#data.size;
    }

    clear(): void {
        this.#data.clear();
    }
}

function isPrefixOf(prefix: StorageKey, candidate: StorageKey): boolean {
    return (
        prefix.length <= candidate.length &&
        prefix.every((segment, index) => segment === candidate[index])
    );
}

export { Server };

// Only start server if run directly
if (import.meta.url === `file://${process.argv[1]}`) {
    // Parse command line arguments
    const args = process.argv.slice(2);
    const port = args.find(arg => !isNaN(parseInt(arg))) ? parseInt(args.find(arg => !isNaN(parseInt(arg)))!) : 3001;
    const isWasmMode = args.includes('--wasm');

    if (isWasmMode) {
        console.log('Starting server in WASM test mode...');
    }

    const server = new Server(port, { wasm: isWasmMode });

    process.on("SIGINT", () => {
        server.close();
        process.exit(0);
    });

    process.on("SIGTERM", () => {
        server.close();
        process.exit(0);
    });
}
