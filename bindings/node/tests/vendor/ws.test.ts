// Vendor test: facts about the `ws` library that js/server.ts relies on. Not a test of our code.
// Run: just test-vendor (or npx vitest run --config vitest.vendor.config.ts).
import { test, expect } from 'vitest';
import { WebSocketServer, WebSocket } from 'ws';
import type { AddressInfo } from 'node:net';

const FRAMES = 50;
const FRAME = 'x'.repeat(256 * 1024);

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function pair(): Promise<{ server: WebSocket; client: WebSocket; close: () => void }> {
  const wss = new WebSocketServer({ port: 0 });
  await new Promise<void>((r) => wss.once('listening', () => r()));
  const { port } = wss.address() as AddressInfo;
  const accepted = new Promise<WebSocket>((r) => wss.once('connection', r));
  const client = new WebSocket(`ws://127.0.0.1:${port}`);
  await new Promise<void>((r) => client.once('open', () => r()));
  const server = await accepted;
  return { server, client, close: () => { client.terminate(); wss.close(); } };
}

// Claim: ws.send's callback is called only once the frame has been written to the socket, so it
// is held back while the peer is not reading and the socket buffers are full, and called once the
// peer reads again.
// Fails if: ws calls the callback when the frame is queued (all callbacks fire while the peer is
// paused), or never calls it after the peer resumes.
// Control: with a reading peer every callback fires.
test('ws send callback must wait while the peer is not reading', async () => {
  const { server, client, close } = await pair();
  try {
    let written = 0;
    client.pause();
    for (let i = 0; i < FRAMES; i++) server.send(FRAME, () => { written++; });
    await wait(500);
    const whilePaused = written;

    client.resume();
    await wait(2000);

    expect(whilePaused).toBeLessThan(FRAMES / 2);
    expect(written).toBe(FRAMES);
  } finally {
    close();
  }
}, 10000);

test('ws send callback must fire for every frame to a reading peer (control)', async () => {
  const { server, close } = await pair();
  try {
    let written = 0;
    for (let i = 0; i < FRAMES; i++) server.send(FRAME, () => { written++; });
    await wait(1000);
    expect(written).toBe(FRAMES);
  } finally {
    close();
  }
}, 10000);

// Claim:    a ws server emits 'ping' for each client ping and answers it with a pong carrying the same payload
// Fails if: ws answered pings without emitting 'ping' (the adapter could not count them as signs of
//           life), or sent no pong (the client's idle check would drop a healthy connection)
// Control:  both the server's 'ping' event and the client's 'pong' event carry the payload
test('ws server must emit ping events and answer pings with pongs', async () => {
  const { server, client, close } = await pair();
  try {
    const seen = new Promise<string>((r) => server.once('ping', (data: Buffer) => r(data.toString())));
    const pong = new Promise<string>((r) => client.once('pong', (data: Buffer) => r(data.toString())));
    client.ping('p');
    expect(await seen).toBe('p');
    expect(await pong).toBe('p');
  } finally {
    close();
  }
}, 10000);
