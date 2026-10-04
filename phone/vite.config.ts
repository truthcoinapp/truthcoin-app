import { defineConfig, type Plugin } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { readFileSync } from 'node:fs';

const VERSION: string = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')).version;

// `npm run build` makes the page for GitHub Pages (or any static host): relays must be wss://.
// `npm run build:local` (mode "localrelay", output dist-local/) also accepts ws://127.0.0.1 and ws://localhost relays, for
// end-to-end checks against dev/test-relay.mjs; its CSP allows those two and nothing else extra.
const LOCAL_CONNECT = 'ws://127.0.0.1:* ws://localhost:*';

function csp(local: boolean): Plugin {
  return {
    name: 'csp-local-relays',
    transformIndexHtml(html) {
      return local ? html.replace("connect-src wss:;", `connect-src wss: ${LOCAL_CONNECT};`) : html;
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
