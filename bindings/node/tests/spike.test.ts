import { test, expect } from 'vitest';
import { OutpunchServer } from '../index.js';

function waitForMessages(messages: string[], count: number, timeoutMs = 2000): Promise<void> {
  return new Promise((resolve, reject) => {
    const start = Date.now();
    const check = () => {
      if (messages.length >= count) return resolve();
      if (Date.now() - start > timeoutMs) return reject(new Error(`Timed out waiting for ${count} messages, got ${messages.length}`));
      setTimeout(check, 10);
    };
    check();
  });
}

function collectMessages(connection: ReturnType<OutpunchServer['createConnection']>): string[] {
  const messages: string[] = [];
  connection.onMessage((_err: null, msg: string) => { messages.push(msg); });
  return messages;
}

// --- Server and connection creation ---

test('create server with defaults', () => {
  const server = new OutpunchServer({ secret: 'test' });
  expect(server).toBeDefined();
});

test('create server with all options', () => {
  const server = new OutpunchServer({ secret: 's', timeoutMs: 1000, maxBodySize: 1024 });
  expect(server).toBeDefined();
});

test('create connection', () => {
  const server = new OutpunchServer({ secret: 'test' });
  const connection = server.createConnection();
  expect(connection).toBeDefined();
});

// --- Auth flow ---

test('auth success sends auth_ok', async () => {
  const server = new OutpunchServer({ secret: 'test-secret' });
  const connection = server.createConnection();
  const messages = collectMessages(connection);

  await connection.pushMessage(JSON.stringify({
    type: 'auth',
    token: 'test-secret',
    service: 'my-service',
  }));

  const runPromise = connection.run();
  await waitForMessages(messages, 1);

  const authOk = JSON.parse(messages[0]);
  expect(authOk.type).toBe('auth_ok');

  connection.close();
  await runPromise;
});

test('auth failure sends auth_error', async () => {
  const server = new OutpunchServer({ secret: 'correct' });
  const connection = server.createConnection();
  const messages = collectMessages(connection);

  await connection.pushMessage(JSON.stringify({
    type: 'auth',
    token: 'wrong',
    service: 'svc',
  }));

  // run() should complete after auth failure
  await connection.run();

  expect(messages.length).toBeGreaterThanOrEqual(1);
  const authError = JSON.parse(messages[0]);
  expect(authError.type).toBe('auth_error');
  expect(authError.message).toContain('invalid token');
});

test('non-auth first message sends auth_error', async () => {
  const server = new OutpunchServer({ secret: 's' });
  const connection = server.createConnection();
  const messages = collectMessages(connection);

  await connection.pushMessage(JSON.stringify({
    type: 'response',
    request_id: 'abc',
    status: 200,
  }));

  await connection.run();

  expect(messages.length).toBeGreaterThanOrEqual(1);
  const authError = JSON.parse(messages[0]);
  expect(authError.type).toBe('auth_error');
  expect(authError.message).toContain('expected auth');
});

// --- Request handling ---

test('full request/response round-trip', async () => {
  const server = new OutpunchServer({ secret: 's', timeoutMs: 5000 });
  const connection = server.createConnection();
  const messages = collectMessages(connection);

  // Wire up echo: respond to requests that come through on_message
  connection.onMessage((_err: null, msg: string) => {
    messages.push(msg);
    try {
      const parsed = JSON.parse(msg);
      if (parsed.type === 'request') {
        connection.pushMessage(JSON.stringify({
          type: 'response',
          request_id: parsed.request_id,
          status: 200,
          headers: {},
          body: `echo: ${parsed.method} /${parsed.path}`,
        }));
      }
    } catch {}
  });

  await connection.pushMessage(JSON.stringify({ type: 'auth', token: 's', service: 'svc' }));
  const runPromise = connection.run();
  await waitForMessages(messages, 1); // auth_ok

  const response = await server.handleRequest({
    service: 'svc',
    method: 'GET',
    path: 'api/test',
  });

  expect(response.status).toBe(200);
  expect(response.body).toBe('echo: GET /api/test');

  connection.close();
  await runPromise;
});

test('handleRequest with body', async () => {
  const server = new OutpunchServer({ secret: 's', timeoutMs: 5000 });
  const connection = server.createConnection();
  const messages = collectMessages(connection);

  connection.onMessage((_err: null, msg: string) => {
    messages.push(msg);
    try {
      const parsed = JSON.parse(msg);
      if (parsed.type === 'request') {
        connection.pushMessage(JSON.stringify({
          type: 'response',
          request_id: parsed.request_id,
          status: 200,
          headers: {},
          body: `got: ${parsed.body}`,
        }));
      }
    } catch {}
  });

  await connection.pushMessage(JSON.stringify({ type: 'auth', token: 's', service: 'svc' }));
  const runPromise = connection.run();
  await waitForMessages(messages, 1);

  const response = await server.handleRequest({
    service: 'svc',
    method: 'POST',
    path: 'echo',
    body: 'request body',
  });

  expect(response.status).toBe(200);
  expect(response.body).toBe('got: request body');

  connection.close();
  await runPromise;
});

test('handleRequest with query and headers', async () => {
  const server = new OutpunchServer({ secret: 's', timeoutMs: 5000 });
  const connection = server.createConnection();
  const messages = collectMessages(connection);

  connection.onMessage((_err: null, msg: string) => {
    messages.push(msg);
    try {
      const parsed = JSON.parse(msg);
      if (parsed.type === 'request') {
        connection.pushMessage(JSON.stringify({
          type: 'response',
          request_id: parsed.request_id,
          status: 200,
          headers: {},
          body: JSON.stringify({ query: parsed.query, headers: parsed.headers }),
        }));
      }
    } catch {}
  });

  await connection.pushMessage(JSON.stringify({ type: 'auth', token: 's', service: 'svc' }));
  const runPromise = connection.run();
  await waitForMessages(messages, 1);

  const response = await server.handleRequest({
    service: 'svc',
    method: 'GET',
    path: 'test',
    query: { foo: 'bar' },
    headers: { 'x-custom': 'value' },
  });

  expect(response.status).toBe(200);
  const body = JSON.parse(response.body!);
  expect(body.query.foo).toBe('bar');
  expect(body.headers['x-custom']).toBe('value');

  connection.close();
  await runPromise;
});

test('handleRequest returns 502 when no client', async () => {
  const server = new OutpunchServer({ secret: 's', timeoutMs: 1000 });

  const response = await server.handleRequest({
    service: 'nonexistent',
    method: 'GET',
    path: 'test',
  });

  expect(response.status).toBe(502);
});

// --- Connection lifecycle ---

test('close() causes run() to exit', async () => {
  const server = new OutpunchServer({ secret: 's' });
  const connection = server.createConnection();
  collectMessages(connection);

  await connection.pushMessage(JSON.stringify({ type: 'auth', token: 's', service: 'svc' }));

  const runPromise = connection.run();

  // Give time for auth
  await new Promise(resolve => setTimeout(resolve, 50));

  connection.close();
  // run() should resolve
  await runPromise;
});

test('connection ignores malformed messages after auth', async () => {
  const server = new OutpunchServer({ secret: 's', timeoutMs: 5000 });
  const connection = server.createConnection();
  const messages = collectMessages(connection);

  connection.onMessage((_err: null, msg: string) => {
    messages.push(msg);
    try {
      const parsed = JSON.parse(msg);
      if (parsed.type === 'request') {
        connection.pushMessage(JSON.stringify({
          type: 'response',
          request_id: parsed.request_id,
          status: 200,
          headers: {},
          body: 'ok',
        }));
      }
    } catch {}
  });

  await connection.pushMessage(JSON.stringify({ type: 'auth', token: 's', service: 'svc' }));
  const runPromise = connection.run();
  await waitForMessages(messages, 1);

  // Send garbage
  await connection.pushMessage('not json');
  await connection.pushMessage('{"type": "unknown"}');

  // Should still work
  const response = await server.handleRequest({
    service: 'svc',
    method: 'GET',
    path: 'still-works',
  });
  expect(response.status).toBe(200);

  connection.close();
  await runPromise;
});
