import { IncomingMessage, ServerResponse } from 'node:http';
import { Duplex } from 'node:stream';
import { WebSocketServer, WebSocket } from 'ws';
import { OutpunchServer, type OutpunchConnection } from '../index.js';

export interface OutpunchAdapterOptions {
  secret: string;
  timeoutMs?: number;
  maxBodySize?: number;
  /** How long a connected client may send nothing (not even a ping) before it is dropped. */
  clientIdleTimeoutMs?: number;
  tunnelPrefix?: string;
  wsPath?: string;
}

export function createOutpunchMiddleware(options: OutpunchAdapterOptions) {
  const server = new OutpunchServer({
    secret: options.secret,
    timeoutMs: options.timeoutMs,
    maxBodySize: options.maxBodySize,
    clientIdleTimeoutMs: options.clientIdleTimeoutMs,
  });
  const tunnelPrefix = options.tunnelPrefix ?? '/tunnel';
  const wsPath = options.wsPath ?? '/ws';
  // A frame not written within the request timeout means the client stopped reading: the
  // connection is closed (F7 in spec/FINDINGS.md).
  const writeTimeoutMs = options.timeoutMs ?? 25000;
  const wss = new WebSocketServer({ noServer: true });

  function handleRequest(req: IncomingMessage, res: ServerResponse): boolean {
    const url = new URL(req.url!, `http://${req.headers.host ?? 'localhost'}`);
    if (!url.pathname.startsWith(tunnelPrefix + '/')) return false;

    const remaining = url.pathname.slice(tunnelPrefix.length + 1);
    const slashIndex = remaining.indexOf('/');
    const service = slashIndex === -1 ? remaining : remaining.slice(0, slashIndex);
    const path = slashIndex === -1 ? '' : remaining.slice(slashIndex + 1);

    if (!service) {
      res.writeHead(400);
      res.end('missing service name');
      return true;
    }

    const query: Record<string, string> = {};
    url.searchParams.forEach((v, k) => { query[k] = v; });

    const headers: Record<string, string> = {};
    const skip = new Set(['host', 'connection', 'upgrade', 'transfer-encoding']);
    for (const [key, value] of Object.entries(req.headers)) {
      if (!skip.has(key) && typeof value === 'string') {
        headers[key] = value;
      }
    }

    const chunks: Buffer[] = [];
    req.on('data', (chunk: Buffer) => chunks.push(chunk));
    req.on('end', async () => {
      const bodyBuf = Buffer.concat(chunks);
      const body = bodyBuf.length > 0 ? bodyBuf.toString() : undefined;

      const response = await server.handleRequest({
        service,
        method: req.method ?? 'GET',
        path,
        query,
        headers,
        body,
      });

      res.writeHead(response.status, response.headers);
      if (response.body != null) {
        if (response.bodyEncoding === 'base64') {
          res.end(Buffer.from(response.body, 'base64'));
        } else {
          res.end(response.body);
        }
      } else {
        res.end();
      }
    });

    return true;
  }

  function handleUpgrade(req: IncomingMessage, socket: Duplex, head: Buffer): boolean {
    const url = new URL(req.url!, `http://${req.headers.host ?? 'localhost'}`);
    if (url.pathname !== wsPath) return false;

    wss.handleUpgrade(req, socket, head, (ws: WebSocket) => {
      const connection = server.createConnection();

      // One message on the socket at a time: ws.send's callback fires once the frame has
      // been written, so the rest wait here, where a request whose caller has already been
      // answered can still be dropped (F5 in spec/FINDINGS.md).
      const queue: string[] = [];
      let writing = false;
      const flush = () => {
        if (writing || ws.readyState !== WebSocket.OPEN) return;
        let msg = queue.shift();
        while (msg !== undefined && !server.isPending(msg)) msg = queue.shift();
        if (msg === undefined) return;
        writing = true;
        const stalled = setTimeout(() => {
          queue.length = 0;
          connection.close();
          ws.terminate();
        }, writeTimeoutMs);
        ws.send(msg, () => {
          clearTimeout(stalled);
          writing = false;
          flush();
        });
      };

      connection.onMessage((_err: null, msg: string) => {
        queue.push(msg);
        flush();
      });

      ws.on('message', (data: Buffer) => {
        connection.pushMessage(data.toString());
      });

      // A client ping (ws answers it) counts as a sign of life (R30).
      ws.on('ping', () => connection.heartbeat());
      ws.on('pong', () => connection.heartbeat());

      ws.on('close', () => {
        queue.length = 0;
        connection.close();
      });

      connection
        .run()
        .catch(() => {})
        .finally(() => ws.close());
    });

    return true;
  }

  return { handleRequest, handleUpgrade, server };
}
