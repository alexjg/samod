import { test, expect, Page } from '@playwright/test';
import { TestServer, createTestServer } from './server-helper';

// Helper functions for direct repo API access
async function createDocumentViaAPI(page: Page, content: any): Promise<string> {
  return await page.evaluate(async (docContent) => {
    const repo = (window as any).testRepo;
    if (!repo) throw new Error('Repository not available');
    const handle = await repo.createDocument(docContent);
    return handle.documentId();
  }, content);
}

async function getDocumentViaAPI(page: Page, docId: string): Promise<any> {
  return await page.evaluate(async (id) => {
    const repo = (window as any).testRepo;
    if (!repo) throw new Error('Repository not available');
    const handle = await repo.findDocument(id);
    if (!handle) return null;
    return handle.getDocument();
  }, docId);
}

async function documentExistsViaAPI(page: Page, docId: string): Promise<boolean> {
  return await page.evaluate(async (id) => {
    const repo = (window as any).testRepo;
    if (!repo) return false;
    try {
      const handle = await repo.findDocument(id);
      return handle !== null && handle !== undefined;
    } catch (e) {
      return false;
    }
  }, docId);
}

test.describe('Samod WASM Integration Tests', () => {
  let page: Page;
  let server: TestServer;

  test.beforeEach(async ({ page: testPage }) => {
    page = testPage;

    server = await createTestServer();
    const port = server.getPort();

    // Navigate to the page with the server port
    await page.goto(`/?wsPort=${port}`);

    // Wait for WASM initialization
    await page.waitForSelector('.status:not(.connecting)', { timeout: 10000 });
  });

  test.afterEach(async () => {
    if (server) await server.stop();
  });

  test('WASM module initializes successfully', async () => {
    const status = await page.locator('#status').textContent();
    expect(status).toContain('WASM Ready');

    const connectBtn = await page.locator('#connectBtn');
    expect(await connectBtn.isEnabled()).toBe(true);

    const logEntries = await page.locator('.log-entry.success').count();
    expect(logEntries).toBeGreaterThan(0);
  });

  test('can connect to WebSocket server', async () => {
    await page.click('#connectBtn');

    await page.waitForSelector('.status.connected', { timeout: 5000 });

    const status = await page.locator('#status').textContent();
    expect(status).toBe('Connected');

    const disconnectBtn = await page.locator('#disconnectBtn');
    expect(await disconnectBtn.isEnabled()).toBe(true);
  });

  test('can create and list documents', async () => {
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    await page.fill('#docTitle', 'Test Document');
    await page.fill('#docContent', 'This is test content');
    await page.click('#createDocBtn');

    // Wait for document to be created
    await page.waitForSelector('.document-item', { timeout: 5000 });

    // Verify document appears in list
    const docItem = await page.locator('.document-item').first();
    expect(await docItem.textContent()).toContain('Test Document');

    // Verify metrics update
    const docCount = await page.locator('#docCount').textContent();
    expect(docCount).toBe('1');
  });

  test('can sync documents between clients', async ({ browser }) => {
    // Client A: Connect and create document
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    // Client B: Open new browser context
    const context = await browser.newContext();
    const pageB = await context.newPage();
    const port = server.getPort();
    await pageB.goto(`/?wsPort=${port}`);
    await pageB.waitForSelector('.status:not(.connecting)');
    await pageB.click('#connectBtn');
    await pageB.waitForSelector('.status.connected');

    // Create document in Client A
    await page.fill('#docTitle', 'Cross-Client Sync Test');
    await page.fill('#docContent', 'This document should sync between clients');
    await page.click('#createDocBtn');
    await page.waitForSelector('.document-item');

    // Verify document was created in Client A
    const docItemA = await page.locator('.document-item').first();
    expect(await docItemA.textContent()).toContain('Cross-Client Sync Test');

    // Client B: Poll sync button until document appears
    const maxAttempts = 20; // 10 seconds total with 500ms intervals
    let found = false;

    for (let attempt = 1; attempt <= maxAttempts; attempt++) {
      await pageB.click('#syncBtn');
      await pageB.waitForTimeout(500);

      const targetDocCount = await pageB
        .locator('.document-item')
        .filter({ hasText: 'Cross-Client Sync Test' })
        .count();

      if (targetDocCount > 0) {
        found = true;
        break;
      }
    }

    expect(found).toBe(true);

    // Verify the document is visible
    const docItems = await pageB
      .locator('.document-item')
      .filter({ hasText: 'Cross-Client Sync Test' })
      .first();
    await expect(docItems).toBeVisible();

    // Verify both clients show the same document count
    await page.click('#syncBtn');
    await pageB.click('#syncBtn');

    const countA = await page.locator('#docCount').textContent();
    const countB = await pageB.locator('#docCount').textContent();
    expect(countA).toBe(countB);

    await context.close();
  });

  test('handles disconnect and reconnect', async () => {
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    // Test client-side disconnection
    await page.click('#disconnectBtn');
    await page.waitForSelector('.status.error');

    let status = await page.locator('#status').textContent();
    expect(status).toBe('Disconnected');

    // Reconnect
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    expect(await page.locator('#status').textContent()).toBe('Connected');
  });

  test('handles server-side disconnection', async () => {
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    // Trigger server-side disconnection
    const response = await page.request.post(
      `http://localhost:${server.getPort()}/test/disconnect-all`
    );
    expect(response.ok()).toBe(true);

    const result = await response.json();
    expect(result.disconnected).toBeGreaterThan(0);

    // Wait for client to detect the disconnection
    await page.waitForSelector('.error', { timeout: 5000 });

    const status = await page.locator('.error').allTextContents();
    expect(status.join(' ')).toContain('Detected server-side disconnection');

    // Test reconnection after server-side disconnect
    await page.waitForSelector('.status.connected');
    expect(await page.locator('#status').textContent()).toBe('Connected');
  });

  test('handles errors gracefully', async () => {
    // Try to create document without required fields
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    // Clear inputs and try to create
    await page.fill('#docTitle', '');
    await page.fill('#docContent', '');
    await page.click('#createDocBtn');

    // Should create with default values
    await page.waitForSelector('.document-item');
    const docText = await page.locator('.document-item').textContent();
    expect(docText).toContain('Document 1');
  });

  test('metrics update correctly', async () => {
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    // Create multiple documents
    for (let i = 0; i < 3; i++) {
      await page.fill('#docTitle', `Doc ${i + 1}`);
      await page.click('#createDocBtn');
      await page.waitForTimeout(100);
    }

    // Check metrics
    const docCount = await page.locator('#docCount').textContent();
    expect(docCount).toBe('3');

    const storageSize = await page.locator('#storageSize').textContent();
    expect(storageSize).toContain('KB');
  });

  test('IndexedDB persistence across page reloads using direct API', async () => {
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    const testDocuments = [
      { 
        title: 'Simple Document', 
        content: 'Simple content with value: 123',
        createdAt: new Date().toISOString(),
      },
      {
        title: 'Large Document',
        content: 'Large document content: ' + 'x'.repeat(10000),
        createdAt: new Date().toISOString(),
      },
    ];

    // Create documents using direct API and store their IDs
    const documentIds: string[] = [];
    for (const docData of testDocuments) {
      const docId = await createDocumentViaAPI(page, docData);
      documentIds.push(docId);
      expect(docId).toBeTruthy();
    }

    // Wait for docs to persist
    await page.waitForTimeout(500);

    // Verify all documents are created via API
    for (let i = 0; i < documentIds.length; i++) {
      const docContent = await getDocumentViaAPI(page, documentIds[i]);
      expect(docContent).toBeTruthy();
      expect(docContent.title).toBe(testDocuments[i].title);
    }

    // Clear server storage to ensure persistence is from IndexedDB
    const clearResponse = await page.request.post(
      `http://localhost:${server.getPort()}/test/clear-storage`
    );
    expect(clearResponse.ok()).toBe(true);

    // Reload the page multiple times to test persistence
    for (let reloadCount = 0; reloadCount < 3; reloadCount++) {
      // Disconnect before reload to ensure no sync happens during reload
      await page.click('#disconnectBtn');
      await page.waitForSelector('.status.error');

      await page.reload();
      await page.waitForSelector('.status:not(.connecting)');

      // Wait for discovery to complete during initialization
      await page.waitForTimeout(1000);

      // Verify documents exist via API (must come from IndexedDB since server was cleared)
      for (let i = 0; i < documentIds.length; i++) {
        const exists = await documentExistsViaAPI(page, documentIds[i]);
        expect(exists).toBe(true);

        const docContent = await getDocumentViaAPI(page, documentIds[i]);
        expect(docContent).toBeTruthy();
        expect(docContent.title).toBe(testDocuments[i].title);
        expect(docContent.content).toBe(testDocuments[i].content);
      }

      // Now reconnect to server
      await page.click('#connectBtn');
      await page.waitForSelector('.status.connected');

      // Verify documents are still accessible after reconnecting
      for (let i = 0; i < documentIds.length; i++) {
        const docContent = await getDocumentViaAPI(page, documentIds[i]);
        expect(docContent).toBeTruthy();
        expect(docContent.title).toBe(testDocuments[i].title);
        expect(docContent.content).toBe(testDocuments[i].content);
      }
    }
  });

  test('IndexedDB storage size calculations are accurate', async () => {
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    // Get initial metrics
    let initialDocCount = await page.locator('#docCount').textContent();
    let initialStorageSize = await page.locator('#storageSize').textContent();
    expect(initialDocCount).toBe('0');
    expect(initialStorageSize).toBe('0 KB');

    // Create a small document
    await page.fill('#docTitle', 'Small Document');
    await page.fill('#docContent', 'Small amount of data');
    await page.click('#createDocBtn');
    await page.waitForSelector('.document-item');

    // Check metrics after small document
    await page.click('#syncBtn');
    let docCount = await page.locator('#docCount').textContent();
    let storageSize = await page.locator('#storageSize').textContent();
    expect(docCount).toBe('1');
    expect(storageSize).toContain('KB');
    expect(storageSize).not.toBe('0 KB');

    // Create a medium document with more content
    const mediumContent = Array.from(
      { length: 10 },
      (_, i) =>
        `Field ${i}: This is field ${i} with some content that makes it longer`
    ).join('\n');

    await page.fill('#docTitle', 'Medium Document');
    await page.fill('#docContent', mediumContent);
    await page.click('#createDocBtn');
    await page.waitForSelector('.document-item:nth-child(2)');

    // Check metrics after medium document
    await page.click('#syncBtn');
    docCount = await page.locator('#docCount').textContent();
    storageSize = await page.locator('#storageSize').textContent();
    expect(docCount).toBe('2');

    // Storage should have increased
    const mediumStorageValue = parseInt(storageSize!.replace(' KB', ''));
    expect(mediumStorageValue).toBeGreaterThan(0);

    // Create a large document with substantial content
    const largeContent = Array.from(
      { length: 50 },
      (_, i) =>
        `Entry ${i}: This is a longer piece of text for entry ${i}. It contains more characters to simulate larger documents with substantial content that would take up more storage space in IndexedDB.`
    ).join('\n');

    await page.fill('#docTitle', 'Large Document');
    await page.fill('#docContent', largeContent);
    await page.click('#createDocBtn');
    await page.waitForSelector('.document-item:nth-child(3)');

    // Check final metrics
    await page.click('#syncBtn');
    const finalDocCount = await page.locator('#docCount').textContent();
    const finalStorageSize = await page.locator('#storageSize').textContent();

    expect(finalDocCount).toBe('3');
    expect(finalStorageSize).toContain('KB');

    // Final storage should be significantly larger than medium
    const finalStorageValue = parseInt(finalStorageSize!.replace(' KB', ''));
    expect(finalStorageValue).toBeGreaterThan(mediumStorageValue);

    // Verify all documents are visible in the UI
    const documentTitles = await page
      .locator('.document-item strong')
      .allTextContents();
    expect(documentTitles).toContain('Small Document');
    expect(documentTitles).toContain('Medium Document');
    expect(documentTitles).toContain('Large Document');
  });

  test('IndexedDB persistence without server using direct API', async () => {
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    // Create test documents using direct API
    const testDocuments = [
      {
        title: 'API Document 1',
        content: 'This should persist without server',
        createdAt: new Date().toISOString(),
      },
      {
        title: 'API Document 2', 
        content: 'Another API document',
        createdAt: new Date().toISOString(),
      },
    ];

    const documentIds: string[] = [];
    for (const docData of testDocuments) {
      const docId = await createDocumentViaAPI(page, docData);
      documentIds.push(docId);
      expect(docId).toBeTruthy();
    }

    // Verify documents exist via API
    for (let i = 0; i < documentIds.length; i++) {
      const docContent = await getDocumentViaAPI(page, documentIds[i]);
      expect(docContent).toBeTruthy();
      expect(docContent.title).toBe(testDocuments[i].title);
      expect(docContent.content).toBe(testDocuments[i].content);
    }

    // Clear server storage to ensure documents can't come from there
    const clearResponse = await page.request.post(
      `http://localhost:${server.getPort()}/test/clear-storage`
    );
    expect(clearResponse.ok()).toBe(true);
    const clearResult = await clearResponse.json();
    expect(clearResult.cleared).toBe(true);

    // Disconnect from server
    await page.click('#disconnectBtn');
    await page.waitForSelector('.status.error');

    // Stop the server completely
    await server.stop();

    // Wait a moment for cleanup
    await page.waitForTimeout(1000);

    // Reload the page with server down
    await page.reload();
    await page.waitForSelector('.status:not(.connecting)', { timeout: 10000 });

    // Wait for discovery to complete during initialization
    await page.waitForTimeout(1000);

    // Verify documents still exist via API after reload (must come from IndexedDB)
    for (let i = 0; i < documentIds.length; i++) {
      const exists = await documentExistsViaAPI(page, documentIds[i]);
      expect(exists).toBe(true);

      const docContent = await getDocumentViaAPI(page, documentIds[i]);
      expect(docContent).toBeTruthy();
      expect(docContent.title).toBe(testDocuments[i].title);
      expect(docContent.content).toBe(testDocuments[i].content);
    }

    // Restart server for cleanup
    server = await createTestServer();
  });

  test('concurrent IndexedDB access from multiple tabs works', async ({
    browser,
  }) => {
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    // Create a document in tab 1
    await page.fill('#docTitle', 'Tab 1 Document');
    await page.fill('#docContent', 'Created in first tab');
    await page.click('#createDocBtn');
    await page.waitForSelector('.document-item');

    // Verify document was created in tab 1
    let tab1DocCount = await page.locator('.document-item').count();
    expect(tab1DocCount).toBe(1);

    const tab1DocTitle = await page
      .locator('.document-item strong')
      .textContent();
    expect(tab1DocTitle).toBe('Tab 1 Document');

    // Open a second tab with the same application
    const page2 = await browser.newPage();
    await page2.goto(page.url());
    await page2.waitForSelector('.status:not(.connecting)');
    await page2.click('#connectBtn');
    await page2.waitForSelector('.status.connected');

    // Sync in tab 2 to load documents from IndexedDB
    await page2.click('#syncBtn');
    await page2.waitForTimeout(1000);

    // Verify tab 2 can see the document created in tab 1
    const tab2InitialDocCount = await page2.locator('.document-item').count();
    expect(tab2InitialDocCount).toBe(1);

    const tab2SeesTab1Doc = await page2
      .locator('.document-item strong')
      .textContent();
    expect(tab2SeesTab1Doc).toBe('Tab 1 Document');

    // Create a document in tab 2 using UI
    await page2.fill('#docTitle', 'Tab 2 Document');
    await page2.fill('#docContent', 'Created in second tab');
    await page2.click('#createDocBtn');
    await page2.waitForSelector('.document-item:nth-child(2)');

    // Verify both documents are visible in tab 2
    const tab2FinalDocCount = await page2.locator('.document-item').count();
    expect(tab2FinalDocCount).toBe(2);

    const tab2DocTitles = await page2
      .locator('.document-item strong')
      .allTextContents();
    expect(tab2DocTitles).toContain('Tab 1 Document');
    expect(tab2DocTitles).toContain('Tab 2 Document');

    // Sync in tab 1 to see the document created in tab 2
    await page.click('#syncBtn');
    await page.waitForTimeout(1000);

    // Verify both documents are now visible in tab 1
    const tab1FinalDocCount = await page.locator('.document-item').count();
    expect(tab1FinalDocCount).toBe(2);

    const tab1DocTitles = await page
      .locator('.document-item strong')
      .allTextContents();
    expect(tab1DocTitles).toContain('Tab 1 Document');
    expect(tab1DocTitles).toContain('Tab 2 Document');

    // Verify metrics are consistent across tabs
    const tab1DocCountMetric = await page.locator('#docCount').textContent();
    const tab2DocCountMetric = await page2.locator('#docCount').textContent();

    expect(tab1DocCountMetric).toBe('2');
    expect(tab2DocCountMetric).toBe('2');
  });

  test('manual disconnect disables auto-reconnect', async () => {
    await page.click('#connectBtn');
    await page.waitForSelector('.status.connected');

    // Verify auto-reconnect is enabled initially
    const connectedState = await page.evaluate(() => {
      const app = (window as any).testApp;
      return app.getConnectionState();
    });
    expect(connectedState.autoReconnect).toBe(true);

    // Manual disconnect
    await page.click('#disconnectBtn');
    await page.waitForSelector('.status.error');

    // Verify auto-reconnect is disabled after manual disconnect
    const disconnectedState = await page.evaluate(() => {
      const app = (window as any).testApp;
      return app.getConnectionState();
    });

    expect(disconnectedState.isConnected).toBe(false);
    expect(disconnectedState.autoReconnect).toBe(false);
    expect(disconnectedState.hasReconnectTimeout).toBe(false);
    expect(disconnectedState.reconnectAttempts).toBe(0);
  });
});
