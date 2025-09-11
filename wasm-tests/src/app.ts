import init, {
  WasmRepo,
  WasmDocHandle,
  WasmWebSocketHandle,
} from '../pkg/samod.js';

interface AppState {
  repo?: WasmRepo;
  documents: Map<string, WasmDocHandle>;
  isConnected: boolean;
  messageCount: number;
  connectionHandle?: WasmWebSocketHandle;
  reconnectAttempts: number;
  reconnectTimeoutId?: number;
  autoReconnect: boolean;
}

class SamodTestApp {
  private state: AppState = {
    documents: new Map(),
    isConnected: false,
    messageCount: 0,
    reconnectAttempts: 0,
    autoReconnect: true,
  };

  private logElement: HTMLElement;
  private statusElement: HTMLElement;
  private documentListElement: HTMLElement;

  constructor() {
    this.logElement = document.getElementById('log')!;
    this.statusElement = document.getElementById('status')!;
    this.documentListElement = document.getElementById('documentList')!;

    this.initializeApp();
  }

  private async initializeApp() {
    try {
      this.log('Initializing WASM module...', 'info');
      await init();

      this.log('WASM module loaded successfully', 'success');
      this.updateStatus('WASM Ready - Not Connected', 'error');

      // Create samod repository with IndexedDB storage
      this.state.repo = await new WasmRepo();
      this.log('Samod repository created with IndexedDB storage', 'success');

      // Discover any documents that were persisted in IndexedDB
      await this.discoverDocuments();

      // Set up event handlers
      this.setupEventHandlers();

      // Enable buttons
      this.enableControls(true);

      // Expose repository and app for testing
      (window as any).testRepo = this.state.repo;
      (window as any).testApp = this;
    } catch (error) {
      this.log(`Failed to initialize: ${error}`, 'error');
      this.updateStatus('Initialization Failed', 'error');
    }
  }

  private setupEventHandlers() {
    // Connection buttons
    document
      .getElementById('connectBtn')!
      .addEventListener('click', () => this.connect());
    document
      .getElementById('disconnectBtn')!
      .addEventListener('click', () => this.disconnect());

    // Document operations
    document
      .getElementById('createDocBtn')!
      .addEventListener('click', () => this.createDocument());
    document
      .getElementById('syncBtn')!
      .addEventListener('click', () => this.syncDocuments());
  }

  private async connect() {
    try {
      this.log('Connecting to WebSocket server...', 'info');
      this.updateStatus('Connecting...', 'connecting');

      if (!this.state.repo) {
        throw new Error('Repository not initialized');
      }

      // Clear any pending reconnection timeout
      if (this.state.reconnectTimeoutId) {
        clearTimeout(this.state.reconnectTimeoutId);
        this.state.reconnectTimeoutId = undefined;
      }

      // Get WebSocket port from URL params or use default
      const urlParams = new URLSearchParams(window.location.search);
      const wsPort = urlParams.get('wsPort') || '3001';
      const wsUrl = `ws://localhost:${wsPort}`;
      this.log(
        `Connecting to ${wsUrl} (attempt ${this.state.reconnectAttempts + 1})`,
        'info'
      );

      // Create the WebSocket connection handle
      const connectionHandle = this.state.repo.connectWebSocket(wsUrl);
      this.state.connectionHandle = connectionHandle;

      // Connection successful - reset reconnect attempts
      this.state.reconnectAttempts = 0;
      this.state.isConnected = true;
      this.updateStatus('Connected', 'connected');
      this.log('WebSocket connection established', 'success');

      // Monitor for disconnection
      this.monitorConnection(connectionHandle);

      // Update button states
      document.getElementById('connectBtn')!.setAttribute('disabled', 'true');
      document.getElementById('disconnectBtn')!.removeAttribute('disabled');
    } catch (error) {
      this.log(`Connection failed: ${error}`, 'error');
      this.updateStatus('Connection Failed', 'error');

      // Trigger reconnection with exponential backoff if auto-reconnect is enabled
      if (this.state.autoReconnect) {
        this.scheduleReconnect();
      }
    }
  }

  private scheduleReconnect() {
    this.state.reconnectAttempts++;

    // Exponential backoff: 1s, 2s, 4s, 8s, 16s, max 30s
    const baseDelay = 1000;
    const maxDelay = 30000;
    const delay = Math.min(
      baseDelay * Math.pow(2, this.state.reconnectAttempts - 1),
      maxDelay
    );

    this.log(
      `Scheduling reconnection in ${delay / 1000}s (attempt ${this.state.reconnectAttempts})`,
      'info'
    );
    this.updateStatus(
      `Reconnecting in ${Math.ceil(delay / 1000)}s...`,
      'connecting'
    );

    this.state.reconnectTimeoutId = window.setTimeout(() => {
      if (this.state.autoReconnect && !this.state.isConnected) {
        this.connect();
      }
    }, delay);
  }

  private enableAutoReconnect(enable: boolean) {
    this.state.autoReconnect = enable;
    this.log(`Auto-reconnect ${enable ? 'enabled' : 'disabled'}`, 'info');

    if (!enable && this.state.reconnectTimeoutId) {
      clearTimeout(this.state.reconnectTimeoutId);
      this.state.reconnectTimeoutId = undefined;
    }
  }

  private async disconnect() {
    try {
      this.log('Disconnecting from WebSocket server...', 'info');

      // Disable auto-reconnect for manual disconnection
      this.enableAutoReconnect(false);

      // Close the WebSocket connection using the handle
      if (this.state.connectionHandle) {
        this.state.connectionHandle.close();
        this.state.connectionHandle = undefined;
      }

      this.state.isConnected = false;
      this.state.reconnectAttempts = 0;
      this.updateStatus('Disconnected', 'error');
      this.log('Disconnected from WebSocket server', 'info');

      // Update button states
      document.getElementById('connectBtn')!.removeAttribute('disabled');
      document
        .getElementById('disconnectBtn')!
        .setAttribute('disabled', 'true');
    } catch (error) {
      this.log(`Disconnect failed: ${error}`, 'error');
    }
  }

  private async createDocument() {
    try {
      const titleInput = document.getElementById(
        'docTitle'
      ) as HTMLInputElement;
      const contentInput = document.getElementById(
        'docContent'
      ) as HTMLTextAreaElement;

      const title =
        titleInput.value || `Document ${this.state.documents.size + 1}`;
      const content = contentInput.value || 'Empty document';

      this.log(`Creating document: ${title}`, 'info');

      if (!this.state.repo) {
        throw new Error('Repository not initialized');
      }

      // Create new document with initial content
      const initialContent = {
        title: title,
        content: content,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
      };

      const docHandle = await this.state.repo.createDocument(initialContent);
      const docId = docHandle.documentId();

      // Store document reference
      this.state.documents.set(docId, docHandle);

      this.log(`Document created: ${docId}`, 'success');
      this.updateDocumentList();
      this.updateMetrics();

      // Clear inputs
      titleInput.value = '';
      contentInput.value = '';
    } catch (error) {
      this.log(`Failed to create document: ${error}`, 'error');
    }
  }

  private async syncDocuments() {
    try {
      this.log('Syncing documents...', 'info');
      const startTime = performance.now();

      if (!this.state.repo) {
        throw new Error('Repository not initialized');
      }

      // Discover any new documents from IndexedDB or network sync
      await this.discoverDocuments();

      // Verify all known documents are accessible
      let accessibleDocs = 0;
      for (const [docId, docHandle] of this.state.documents) {
        try {
          const doc = docHandle.getDocument();
          if (doc) {
            accessibleDocs++;
          }
        } catch (e) {
          this.log(`Document ${docId} not accessible`, 'error');
        }
      }

      const syncTime = Math.round(performance.now() - startTime);
      this.log(
        `Sync completed in ${syncTime}ms - ${accessibleDocs} documents accessible`,
        'success'
      );

      // Always update sync time metrics
      const syncTimeElement = document.getElementById('syncTime');
      if (syncTimeElement) {
        syncTimeElement.textContent = syncTime.toString();
      }
    } catch (error) {
      this.log(`Sync failed: ${error}`, 'error');
    }
  }

  private updateDocumentList() {
    this.documentListElement.innerHTML = '';

    this.state.documents.forEach((docHandle, id) => {
      try {
        const doc = docHandle.getDocument();
        const title = doc?.title || 'Untitled Document';

        const item = document.createElement('li');
        item.className = 'document-item';
        item.innerHTML = `
                  <strong>${title}</strong>
                  <br>
                  <small>ID: ${id.substring(0, 8)}...</small>
              `;
        item.addEventListener('click', () => this.selectDocument(id));
        this.documentListElement.appendChild(item);
      } catch (error) {
        this.log(`Error displaying document ${id}: ${error}`, 'error');
      }
    });
  }

  private selectDocument(docId: string) {
    // Update UI to show selected document
    const items = this.documentListElement.querySelectorAll('.document-item');
    items.forEach(item => item.classList.remove('active'));

    const selectedIndex = Array.from(this.state.documents.keys()).indexOf(
      docId
    );
    if (selectedIndex >= 0) {
      items[selectedIndex].classList.add('active');
    }

    this.log(`Selected document: ${docId}`, 'info');
  }

  private updateMetrics() {
    document.getElementById('docCount')!.textContent =
      this.state.documents.size.toString();
    document.getElementById('messageCount')!.textContent =
      this.state.messageCount.toString();

    // Estimate storage size (rough calculation)
    const storageSize = this.state.documents.size * 2; // Assume ~2KB per doc
    document.getElementById('storageSize')!.textContent = `${storageSize} KB`;
  }

  private updateStatus(
    message: string,
    className: 'connecting' | 'connected' | 'error'
  ) {
    this.statusElement.textContent = message;
    this.statusElement.className = `status ${className}`;
  }

  private log(message: string, type: 'info' | 'success' | 'error' = 'info') {
    const entry = document.createElement('div');
    entry.className = `log-entry ${type}`;
    entry.textContent = `[${new Date().toLocaleTimeString()}] ${message}`;
    this.logElement.appendChild(entry);
    this.logElement.scrollTop = this.logElement.scrollHeight;
  }

  private enableControls(enabled: boolean) {
    const buttons = ['connectBtn', 'createDocBtn', 'syncBtn'];
    buttons.forEach(id => {
      const btn = document.getElementById(id);
      if (btn) {
        if (enabled) {
          btn.removeAttribute('disabled');
        } else {
          btn.setAttribute('disabled', 'true');
        }
      }
    });
  }

  private async discoverDocuments() {
    try {
      if (!this.state.repo) {
        return;
      }

      this.log('Discovering available documents...', 'info');

      const documentIds = this.state.repo.listDocuments();
      this.log(`Found ${documentIds.length} document(s) in repository`, 'info');

      let newDocuments = 0;
      for (let i = 0; i < documentIds.length; i++) {
        const docId = documentIds[i];
        if (!this.state.documents.has(docId)) {
          try {
            const docHandle = await this.state.repo.findDocument(docId);
            if (docHandle) {
              this.state.documents.set(docId, docHandle);
              newDocuments++;
              this.log(`Loaded document: ${docId}`, 'success');
            }
          } catch (error) {
            this.log(`Failed to load document ${docId}: ${error}`, 'error');
          }
        }
      }

      if (newDocuments > 0) {
        this.log(`Discovered ${newDocuments} new document(s)`, 'success');
        this.updateDocumentList();
        this.updateMetrics();
      } else {
        this.log('No new documents found', 'info');
      }
    } catch (error) {
      this.log(`Document discovery failed: ${error}`, 'error');
    }
  }

  private async monitorConnection(connectionHandle: WasmWebSocketHandle) {
    try {
      // Wait for the connection to end
      const disconnectReason = await connectionHandle.waitForDisconnect();

      if (disconnectReason) {
        this.log(`Connection ended: ${disconnectReason}`, 'info');

        // Update UI state if this wasn't a client-initiated disconnect
        if (disconnectReason !== 'we_disconnected' && this.state.isConnected) {
          this.state.isConnected = false;
          this.updateStatus('Disconnected', 'error');
          this.log('Detected server-side disconnection', 'error');

          // Update button states
          document.getElementById('connectBtn')!.removeAttribute('disabled');
          document
            .getElementById('disconnectBtn')!
            .setAttribute('disabled', 'true');

          // Clear connection handle
          this.state.connectionHandle = undefined;

          // Re-enable auto-reconnect and schedule reconnection for unexpected disconnections
          if (disconnectReason !== 'we_disconnected') {
            this.enableAutoReconnect(true);
            this.scheduleReconnect();
          }
        }
      }
    } catch (error) {
      this.log(`Connection monitoring error: ${error}`, 'error');
      // On monitoring error, also try to reconnect
      if (this.state.isConnected && this.state.autoReconnect) {
        this.state.isConnected = false;
        this.updateStatus('Connection Error', 'error');
        this.scheduleReconnect();
      }
    }
  }

  // Public methods for testing
  public getConnectionState() {
    return {
      isConnected: this.state.isConnected,
      reconnectAttempts: this.state.reconnectAttempts,
      autoReconnect: this.state.autoReconnect,
      hasReconnectTimeout: !!this.state.reconnectTimeoutId,
    };
  }

  public setAutoReconnect(enable: boolean) {
    this.enableAutoReconnect(enable);
  }

  public forceReconnect() {
    if (!this.state.isConnected) {
      this.state.reconnectAttempts = 0;
      this.connect();
    }
  }

  public simulateDisconnection() {
    if (this.state.connectionHandle) {
      this.state.connectionHandle.close();
    }
  }
}

// Initialize app when DOM is ready
document.addEventListener('DOMContentLoaded', () => {
  new SamodTestApp();
});
