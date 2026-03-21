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

test('create server and connection', () => {
  const server = new OutpunchServer({ secret: 'test-secret' });
  const connection = server.createConnection();
  expect(connection).toBeDefined();
});

test('auth flow via pushMessage and onMessage', async () => {
  const server = new OutpunchServer({ secret: 'test-secret', timeoutMs: 5000 });
  const connection = server.createConnection();

  const messages: string[] = [];
  connection.onMessage((_err: null, msg: string) => {
    messages.push(msg);
  });

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

test('auth rejection sends auth_error', async () => {
  const server = new OutpunchServer({ secret: 'correct-secret' });
  const connection = server.createConnection();

  const messages: string[] = [];
  connection.onMessage((_err: null, msg: string) => {
    messages.push(msg);
  });

  await connection.pushMessage(JSON.stringify({
    type: 'auth',
    token: 'wrong-secret',
    service: 'my-service',
  }));

  await connection.run();

  expect(messages.length).toBeGreaterThanOrEqual(1);
  const authError = JSON.parse(messages[0]);
  expect(authError.type).toBe('auth_error');
  expect(authError.message).toContain('invalid token');
});

test('full request/response round-trip', async () => {
  const server = new OutpunchServer({ secret: 'test-secret', timeoutMs: 5000 });
  const connection = server.createConnection();

  const messages: string[] = [];
  connection.onMessage((_err: null, msg: string) => {
    messages.push(msg);

    try {
      const parsed = JSON.parse(msg);
      if (parsed.type === 'request') {
        connection.pushMessage(JSON.stringify({
          type: 'response',
          request_id: parsed.request_id,
          status: 200,
          headers: { 'content-type': 'text/plain' },
          body: `echo: ${parsed.method} /${parsed.path}`,
        }));
      }
    } catch {
      // ignore parse errors
    }
  });

  await connection.pushMessage(JSON.stringify({
    type: 'auth',
    token: 'test-secret',
    service: 'my-service',
  }));

  const runPromise = connection.run();

  await waitForMessages(messages, 1);
  expect(JSON.parse(messages[0]).type).toBe('auth_ok');

  const response = await server.handleRequest({
    service: 'my-service',
    method: 'GET',
    path: 'api/test',
  });

  expect(response.status).toBe(200);
  expect(response.body).toBe('echo: GET /api/test');

  connection.close();
  await runPromise;
});
