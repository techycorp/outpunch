import type { Plugin } from 'vite';
import { createOutpunchMiddleware, type OutpunchAdapterOptions } from './server.js';

export function outpunch(options: OutpunchAdapterOptions): Plugin {
  return {
    name: 'outpunch',
    configureServer(viteServer) {
      const { handleRequest, handleUpgrade } = createOutpunchMiddleware(options);

      viteServer.middlewares.use((req, res, next) => {
        if (!handleRequest(req, res)) {
          next();
        }
      });

      viteServer.httpServer?.on('upgrade', (req, socket, head) => {
        handleUpgrade(req, socket, head);
      });
    },
  };
}
