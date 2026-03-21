import { test, expect, afterEach } from 'vitest';
import { createServer, type Server } from 'node:http';
import { WebSocket } from 'ws';
import { createOutpunchMiddleware } from '../js/server.js';

let httpServer: Server | null = null;

afterEach(() => {
  httpServer?.close();
  httpServer = null;
});

function startServer(options: { secret: string; timeoutMs?: number }): Promise<number> {
  return new Promise((resolve) => {
    const { handleRequest, handleUpgrade } = createOutpunchMiddleware(options);

    httpServer = createServer((req, res) => {
      if (!handleRequest(req, res)) {
        res.writeHead(404);
        res.end('not found');
      }
    });

    httpServer.on('upgrade', (req, socket, head) => {
      handleUpgrade(req, socket, head);
    });

    httpServer.listen(0, () => {
      const addr = httpServer!.address();
      resolve(typeof addr === 'object' ? addr!.port : 0);
    });
  });
}

function connectClient(port: number): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`);
    ws.on('open', () => resolve(ws));
    ws.on('error', reject);
  });
}

function waitForMessage(ws: WebSocket): Promise<string> {
  return new Promise((resolve) => {
    ws.once('message', (data) => resolve(data.toString()));
  });
}

test('full round-trip through http.Server adapter', async () => {
  const port = await startServer({ secret: 'test-secret', timeoutMs: 5000 });
  const ws = await connectClient(port);

  // Auth
  ws.send(JSON.stringify({ type: 'auth', token: 'test-secret', service: 'my-svc' }));
  const authResp = await waitForMessage(ws);
  expect(JSON.parse(authResp).type).toBe('auth_ok');

  // Echo client: respond to requests
  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 200,
        headers: { 'content-type': 'text/plain' },
        body: `echo: ${msg.method} /${msg.path}`,
      }));
    }
  });

  // HTTP request through tunnel
  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/my-svc/api/hello`);
  expect(resp.status).toBe(200);
  const body = await resp.text();
  expect(body).toBe('echo: GET /api/hello');

  ws.close();
});

test('returns 502 when no client connected', async () => {
  const port = await startServer({ secret: 'test-secret', timeoutMs: 1000 });

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/no-service/test`);
  expect(resp.status).toBe(502);
});

test('passes through non-tunnel requests', async () => {
  const port = await startServer({ secret: 'test-secret' });

  const resp = await fetch(`http://127.0.0.1:${port}/not-a-tunnel`);
  expect(resp.status).toBe(404);
  const body = await resp.text();
  expect(body).toBe('not found');
});

test('auth rejection', async () => {
  const port = await startServer({ secret: 'correct-secret' });
  const ws = await connectClient(port);

  ws.send(JSON.stringify({ type: 'auth', token: 'wrong-secret', service: 'svc' }));
  const resp = await waitForMessage(ws);
  const parsed = JSON.parse(resp);
  expect(parsed.type).toBe('auth_error');
  expect(parsed.message).toContain('invalid token');

  ws.close();
});

test('forwards POST with body', async () => {
  const port = await startServer({ secret: 'test-secret', timeoutMs: 5000 });
  const ws = await connectClient(port);

  ws.send(JSON.stringify({ type: 'auth', token: 'test-secret', service: 'svc' }));
  await waitForMessage(ws); // auth_ok

  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 200,
        headers: {},
        body: `got: ${msg.body}`,
      }));
    }
  });

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/echo`, {
    method: 'POST',
    body: 'hello world',
  });
  expect(resp.status).toBe(200);
  const body = await resp.text();
  expect(body).toBe('got: hello world');

  ws.close();
});

test('forwards query parameters', async () => {
  const port = await startServer({ secret: 'test-secret', timeoutMs: 5000 });
  const ws = await connectClient(port);

  ws.send(JSON.stringify({ type: 'auth', token: 'test-secret', service: 'svc' }));
  await waitForMessage(ws); // auth_ok

  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 200,
        headers: {},
        body: JSON.stringify(msg.query),
      }));
    }
  });

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/search?foo=bar&n=42`);
  expect(resp.status).toBe(200);
  const body = JSON.parse(await resp.text());
  expect(body.foo).toBe('bar');
  expect(body.n).toBe('42');

  ws.close();
});
