import { defineConfig, type Plugin } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { readFileSync } from 'node:fs';

const VERSION: string = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')).version;

// `npm run build` makes the page for GitHub Pages (or any static host): relays must be wss://.
// `npm run build:local` (mode "localrelay", output dist-local/) also accepts ws://127.0.0.1 and ws://localhost relays, for
// end-to-end checks against dev/test-relay.mjs; its CSP allows those two and nothing else extra.
const LOCAL_CONNECT = 'ws://127.0.0.1:* ws://localhost:*';

// The dev server (`npm run dev`) injects styles and talks to Vite over a WebSocket: only there does the CSP allow
// them. Built pages keep the strict one in index.html.
function csp(local: boolean): Plugin {
  return {
    name: 'csp-local-relays',
    transformIndexHtml(html, ctx) {
      let out = local ? html.replace('connect-src wss:;', `connect-src wss: ${LOCAL_CONNECT};`) : html;
      if (ctx.server) {
        out = out.replace("style-src 'self';", "style-src 'self' 'unsafe-inline';").replace('connect-src wss:', 'connect-src wss: ws:');
      }
      return out;
    },
  };
}

export default defineConfig(({ mode }) => {
  const local = mode === 'localrelay';
  return {
    plugins: [svelte({ hot: false }), csp(local)],
    base: './',
    define: {
      __LOCAL_RELAYS__: JSON.stringify(local || mode === 'test'),
      __APP_VERSION__: JSON.stringify(VERSION),
    },
    build: {
      outDir: local ? 'dist-local' : 'dist',
      target: ['safari16', 'chrome109', 'firefox115'],
      modulePreload: { polyfill: false },
      assetsInlineLimit: 0, // scripts and images as files: the CSP allows scripts only from 'self'
      sourcemap: false,
    },
    server: { host: '127.0.0.1', port: 5175, strictPort: true },
    test: {
      include: ['src/**/*.test.ts'],
      environment: 'node',
      setupFiles: ['src/test-setup.ts'],
      testTimeout: 20000,
    },
  };
});
