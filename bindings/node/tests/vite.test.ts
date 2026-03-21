import { test, expect, afterEach } from 'vitest';
import { createServer, type ViteDevServer } from 'vite';
import { WebSocket } from 'ws';
import { outpunch } from '../js/vite.js';

let viteServer: ViteDevServer | null = null;

afterEach(async () => {
  await viteServer?.close();
  viteServer = null;
});

function waitForMessage(ws: WebSocket): Promise<string> {
  return new Promise((resolve) => {
    ws.once('message', (data) => resolve(data.toString()));
  });
}

test('vite plugin full round-trip', async () => {
  viteServer = await createServer({
    plugins: [outpunch({ secret: 'test-secret', timeoutMs: 5000 })],
    configFile: false,
    server: { host: '127.0.0.1' },
    logLevel: 'silent',
  });
  await viteServer.listen(0);

  const resolved = viteServer.resolvedUrls!;
  const localUrl = resolved.local[0] ?? `http://127.0.0.1:${(viteServer.httpServer!.address() as any).port}`;
  const url = new URL(localUrl);
  const port = parseInt(url.port, 10);

  // Connect tunnel client
  const ws = await new Promise<WebSocket>((resolve, reject) => {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`);
    socket.on('open', () => resolve(socket));
    socket.on('error', reject);
  });

  // Auth
  ws.send(JSON.stringify({ type: 'auth', token: 'test-secret', service: 'my-app' }));
  const authResp = await waitForMessage(ws);
  expect(JSON.parse(authResp).type).toBe('auth_ok');

  // Echo client
  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 200,
        headers: { 'x-via': 'outpunch' },
        body: `vite: ${msg.method} /${msg.path}`,
      }));
    }
  });

  // HTTP request through tunnel
  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/my-app/api/data`);
  expect(resp.status).toBe(200);
  expect(resp.headers.get('x-via')).toBe('outpunch');
  const body = await resp.text();
  expect(body).toBe('vite: GET /api/data');

  ws.close();
});
