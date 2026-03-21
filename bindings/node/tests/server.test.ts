import { test, expect, afterEach } from 'vitest';
import { createServer, type Server } from 'node:http';
import { WebSocket } from 'ws';
import { createOutpunchMiddleware } from '../js/server.js';

let httpServer: Server | null = null;

afterEach(() => {
  httpServer?.close();
  httpServer = null;
});

function startServer(options: Parameters<typeof createOutpunchMiddleware>[0]): Promise<number> {
  return new Promise((resolve) => {
    const { handleRequest, handleUpgrade } = createOutpunchMiddleware(options);

    httpServer = createServer((req, res) => {
      if (!handleRequest(req, res)) {
        res.writeHead(404);
        res.end('not found');
      }
    });

    httpServer.on('upgrade', (req, socket, head) => {
      if (!handleUpgrade(req, socket, head)) {
        socket.destroy();
      }
    });

    httpServer.listen(0, () => {
      const addr = httpServer!.address();
      resolve(typeof addr === 'object' ? addr!.port : 0);
    });
  });
}

function connectClient(port: number, path = '/ws'): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}${path}`);
    ws.on('open', () => resolve(ws));
    ws.on('error', reject);
  });
}

function waitForMessage(ws: WebSocket): Promise<string> {
  return new Promise((resolve) => {
    ws.once('message', (data) => resolve(data.toString()));
  });
}

async function connectAndAuth(port: number, secret: string, service: string): Promise<WebSocket> {
  const ws = await connectClient(port);
  ws.send(JSON.stringify({ type: 'auth', token: secret, service }));
  const resp = await waitForMessage(ws);
  expect(JSON.parse(resp).type).toBe('auth_ok');
  return ws;
}

function echoClient(ws: WebSocket) {
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
}

// --- HTTP tunnel handler: routing ---

test('full GET round-trip through tunnel', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');
  echoClient(ws);

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/api/hello`);
  expect(resp.status).toBe(200);
  expect(await resp.text()).toBe('echo: GET /api/hello');
  ws.close();
});

test('returns 502 when no client connected', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 1000 });
  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/no-service/test`);
  expect(resp.status).toBe(502);
});

test('passes through non-tunnel requests', async () => {
  const port = await startServer({ secret: 's' });
  const resp = await fetch(`http://127.0.0.1:${port}/not-a-tunnel`);
  expect(resp.status).toBe(404);
  expect(await resp.text()).toBe('not found');
});

test('returns 400 for missing service name', async () => {
  const port = await startServer({ secret: 's' });
  // /tunnel/ with trailing slash but no service name
  const resp = await fetch(`http://127.0.0.1:${port}/tunnel//some-path`);
  expect(resp.status).toBe(400);
  expect(await resp.text()).toBe('missing service name');
});

test('handles service name with no subpath', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');
  echoClient(ws);

  // /tunnel/svc without trailing slash — still routed (path is "svc")
  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc`, { redirect: 'manual' });
  // Depends on whether the fetch redirect adds a slash
  // The URL /tunnel/svc starts with /tunnel/ so it matches
  expect(resp.status).toBe(200);
  ws.close();
});

test('handles service with empty subpath', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');

  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 200,
        headers: {},
        body: `path: [${msg.path}]`,
      }));
    }
  });

  // /tunnel/svc/ — service is "svc", path is ""
  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/`);
  expect(resp.status).toBe(200);
  expect(await resp.text()).toBe('path: []');
  ws.close();
});

// --- HTTP methods ---

test('forwards POST with body', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');

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
  expect(await resp.text()).toBe('got: hello world');
  ws.close();
});

test('forwards PUT request', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');
  echoClient(ws);

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/resource`, { method: 'PUT' });
  expect(resp.status).toBe(200);
  expect(await resp.text()).toBe('echo: PUT /resource');
  ws.close();
});

test('forwards DELETE request', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');
  echoClient(ws);

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/item/42`, { method: 'DELETE' });
  expect(resp.status).toBe(200);
  expect(await resp.text()).toBe('echo: DELETE /item/42');
  ws.close();
});

test('forwards PATCH request', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');
  echoClient(ws);

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/update`, { method: 'PATCH' });
  expect(resp.status).toBe(200);
  expect(await resp.text()).toBe('echo: PATCH /update');
  ws.close();
});

// --- Query parameters ---

test('forwards query parameters', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');

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

test('handles request with no query parameters', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');

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

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/no-query`);
  expect(resp.status).toBe(200);
  const body = JSON.parse(await resp.text());
  expect(Object.keys(body).length).toBe(0);
  ws.close();
});

// --- Headers ---

test('forwards request headers (skips hop-by-hop)', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');

  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 200,
        headers: {},
        body: JSON.stringify(msg.headers),
      }));
    }
  });

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/headers`, {
    headers: { 'x-custom': 'my-value', 'accept': 'application/json' },
  });
  expect(resp.status).toBe(200);
  const body = JSON.parse(await resp.text());
  expect(body['x-custom']).toBe('my-value');
  expect(body['accept']).toBe('application/json');
  // host should be filtered
  expect(body['host']).toBeUndefined();
  ws.close();
});

test('response headers are forwarded back to client', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');

  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 200,
        headers: { 'x-custom-resp': 'resp-value', 'content-type': 'application/json' },
        body: '{}',
      }));
    }
  });

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/with-headers`);
  expect(resp.status).toBe(200);
  expect(resp.headers.get('x-custom-resp')).toBe('resp-value');
  expect(resp.headers.get('content-type')).toBe('application/json');
  ws.close();
});

// --- Response body handling ---

test('handles base64-encoded response body', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');

  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 200,
        headers: {},
        body: Buffer.from('binary content here').toString('base64'),
        body_encoding: 'base64',
      }));
    }
  });

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/binary`);
  expect(resp.status).toBe(200);
  expect(await resp.text()).toBe('binary content here');
  ws.close();
});

test('handles empty response body', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');

  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 204,
        headers: {},
      }));
    }
  });

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/no-content`);
  expect(resp.status).toBe(204);
  ws.close();
});

test('handles non-200 status codes', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');

  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 404,
        headers: {},
        body: 'not found',
      }));
    }
  });

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/missing`);
  expect(resp.status).toBe(404);
  expect(await resp.text()).toBe('not found');
  ws.close();
});

// --- Request body handling ---

test('handles request with no body', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');

  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 200,
        headers: {},
        body: `body is: ${msg.body ?? 'undefined'}`,
      }));
    }
  });

  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/nobody`);
  expect(resp.status).toBe(200);
  expect(await resp.text()).toBe('body is: undefined');
  ws.close();
});

test('handles large request body', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');

  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.type === 'request') {
      ws.send(JSON.stringify({
        type: 'response',
        request_id: msg.request_id,
        status: 200,
        headers: {},
        body: `length: ${msg.body?.length ?? 0}`,
      }));
    }
  });

  const largeBody = 'x'.repeat(100_000);
  const resp = await fetch(`http://127.0.0.1:${port}/tunnel/svc/big`, {
    method: 'POST',
    body: largeBody,
  });
  expect(resp.status).toBe(200);
  expect(await resp.text()).toBe('length: 100000');
  ws.close();
});

// --- Multiple concurrent requests ---

test('handles multiple concurrent requests', async () => {
  const port = await startServer({ secret: 's', timeoutMs: 5000 });
  const ws = await connectAndAuth(port, 's', 'svc');
  echoClient(ws);

  const results = await Promise.all([
    fetch(`http://127.0.0.1:${port}/tunnel/svc/one`).then(r => r.text()),
    fetch(`http://127.0.0.1:${port}/tunnel/svc/two`).then(r => r.text()),
    fetch(`http://127.0.0.1:${port}/tunnel/svc/three`).then(r => r.text()),
  ]);

  expect(results).toContain('echo: GET /one');
  expect(results).toContain('echo: GET /two');
  expect(results).toContain('echo: GET /three');
  ws.close();
});

// --- WebSocket auth ---

test('auth rejection over WebSocket', async () => {
  const port = await startServer({ secret: 'correct' });
  const ws = await connectClient(port);

  ws.send(JSON.stringify({ type: 'auth', token: 'wrong', service: 'svc' }));
  const resp = await waitForMessage(ws);
  const parsed = JSON.parse(resp);
  expect(parsed.type).toBe('auth_error');
  expect(parsed.message).toContain('invalid token');
  ws.close();
});

// --- WebSocket upgrade filtering ---

test('rejects upgrade on non-ws path', async () => {
  const port = await startServer({ secret: 's' });

  const ws = new WebSocket(`ws://127.0.0.1:${port}/not-ws`);
  const error = await new Promise<Error>((resolve) => {
    ws.on('error', resolve);
  });
  expect(error).toBeDefined();
});

// --- Custom options ---

test('custom tunnel prefix', async () => {
  const port = await new Promise<number>((resolve) => {
    const { handleRequest, handleUpgrade } = createOutpunchMiddleware({
      secret: 's',
      timeoutMs: 5000,
      tunnelPrefix: '/api/proxy',
    });

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
      resolve((httpServer!.address() as any).port);
    });
  });

  const ws = await connectAndAuth(port, 's', 'svc');
  echoClient(ws);

  // Default prefix should not work
  const resp1 = await fetch(`http://127.0.0.1:${port}/tunnel/svc/test`);
  expect(resp1.status).toBe(404);

  // Custom prefix should work
  const resp2 = await fetch(`http://127.0.0.1:${port}/api/proxy/svc/test`);
  expect(resp2.status).toBe(200);
  expect(await resp2.text()).toBe('echo: GET /test');

  ws.close();
});

test('custom ws path', async () => {
  const port = await new Promise<number>((resolve) => {
    const { handleRequest, handleUpgrade } = createOutpunchMiddleware({
      secret: 's',
      wsPath: '/custom-ws',
    });

    httpServer = createServer((req, res) => {
      if (!handleRequest(req, res)) {
        res.writeHead(404);
        res.end();
      }
    });
    httpServer.on('upgrade', (req, socket, head) => {
      if (!handleUpgrade(req, socket, head)) {
        socket.destroy();
      }
    });
    httpServer.listen(0, () => {
      resolve((httpServer!.address() as any).port);
    });
  });

  // Default /ws should fail
  const ws1 = new WebSocket(`ws://127.0.0.1:${port}/ws`);
  const err = await new Promise<Error>(resolve => ws1.on('error', resolve));
  expect(err).toBeDefined();

  // Custom path should work
  const ws2 = await connectClient(port, '/custom-ws');
  ws2.send(JSON.stringify({ type: 'auth', token: 's', service: 'svc' }));
  const resp = await waitForMessage(ws2);
  expect(JSON.parse(resp).type).toBe('auth_ok');
  ws2.close();
});
