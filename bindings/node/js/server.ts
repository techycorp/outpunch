import { IncomingMessage, ServerResponse } from 'node:http';
import { Duplex } from 'node:stream';
import { WebSocketServer, WebSocket } from 'ws';
import { OutpunchServer, type OutpunchConnection } from '../index.js';

export interface OutpunchAdapterOptions {
  secret: string;
  timeoutMs?: number;
  maxBodySize?: number;
  tunnelPrefix?: string;
  wsPath?: string;
}

export function createOutpunchMiddleware(options: OutpunchAdapterOptions) {
  const server = new OutpunchServer({
    secret: options.secret,
    timeoutMs: options.timeoutMs,
    maxBodySize: options.maxBodySize,
  });
  const tunnelPrefix = options.tunnelPrefix ?? '/tunnel';
  const wsPath = options.wsPath ?? '/ws';
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

      connection.onMessage((_err: null, msg: string) => {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(msg);
        }
      });

      ws.on('message', (data: Buffer) => {
        connection.pushMessage(data.toString());
      });

      ws.on('close', () => {
        connection.close();
      });

      connection.run().catch(() => {});
    });

    return true;
  }

  return { handleRequest, handleUpgrade, server };
}
