import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';

// The browser checks (e2e/*.e2e.ts): `npm run e2e`. They drive the built pages (dist/ and dist-local/) in Playwright's
// Chromium and WebKit, headless.
export default defineConfig({
  root: fileURLToPath(new URL('..', import.meta.url)),
  define: { __LOCAL_RELAYS__: 'true', __APP_VERSION__: '"e2e"' },
  test: {
    include: ['e2e/**/*.e2e.ts'],
    environment: 'node',
    setupFiles: ['src/test-setup.ts'],
    testTimeout: 90_000,
    hookTimeout: 60_000,
    fileParallelism: false,
  },
});
