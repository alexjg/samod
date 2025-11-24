import { spawn, ChildProcess } from 'child_process';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';
import net from 'net';

const __dirname = dirname(fileURLToPath(import.meta.url));

export class TestServer {
  private process: ChildProcess | null = null;
  private port: number = 0;

  async start(): Promise<number> {
    // Find an available port
    this.port = await this.findAvailablePort();

    // Start the server
    return new Promise((resolve, reject) => {
      const serverPath = join(
        __dirname,
        '../../samod/interop-test-server/server.js'
      );

      this.process = spawn('node', [serverPath, '--wasm', String(this.port)], {
        stdio: ['ignore', 'pipe', 'pipe'],
        env: { ...process.env, NODE_ENV: 'test' },
      });

      let started = false;

      // Listen for startup message
      this.process.stdout?.on('data', data => {
        const output = data.toString();
        console.log(`[Server ${this.port}] ${output.trim()}`);

        if (output.includes('Listening on port') && !started) {
          started = true;
          resolve(this.port);
        }
      });

      this.process.stderr?.on('data', data => {
        console.error(`[Server ${this.port} Error] ${data.toString().trim()}`);
      });

      this.process.on('error', error => {
        reject(new Error(`Failed to start server: ${error.message}`));
      });

      this.process.on('exit', code => {
        if (!started) {
          reject(new Error(`Server exited with code ${code} before starting`));
        }
      });

      // Timeout if server doesn't start
      setTimeout(() => {
        if (!started) {
          this.stop();
          reject(new Error('Server failed to start within timeout'));
        }
      }, 10000);
    });
  }

  async stop(): Promise<void> {
    if (this.process) {
      return new Promise(resolve => {
        this.process!.once('exit', () => {
          this.process = null;
          resolve();
        });

        this.process!.kill('SIGTERM');

        // Force kill after timeout
        setTimeout(() => {
          if (this.process) {
            this.process.kill('SIGKILL');
          }
        }, 5000);
      });
    }
  }

  getPort(): number {
    return this.port;
  }

  getUrl(): string {
    return `http://localhost:${this.port}`;
  }

  getWsUrl(): string {
    return `ws://localhost:${this.port}`;
  }

  private async findAvailablePort(): Promise<number> {
    return new Promise((resolve, reject) => {
      const server = net.createServer();

      server.listen(0, () => {
        const address = server.address();
        if (address && typeof address !== 'string') {
          const port = address.port;
          server.close(() => resolve(port));
        } else {
          reject(new Error('Failed to get port'));
        }
      });

      server.on('error', reject);
    });
  }
}

// Helper function to create and start a server
export async function createTestServer(): Promise<TestServer> {
  const server = new TestServer();
  await server.start();
  return server;
}
