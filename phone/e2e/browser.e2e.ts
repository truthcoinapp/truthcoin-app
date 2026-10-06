// The built page in real browsers (Playwright's Chromium and WebKit, headless, 390 px wide):
//  - dist/ (the GitHub Pages build): the not-paired screen renders without console errors, and `#pair=` is read and
//    stripped from the address bar before anything renders;
//  - dist-local/ (the build that accepts ws://127.0.0.1 relays): a full run through dev/test-relay.mjs against the
//    test desktop: pair (same comparison code on both ends), Home, Markets, a market, a buy held over the limit until
//    the desktop confirms it, a reload that stays paired with a non-extractable key, and "Forget this computer".
// Screenshots go to e2e/shots/<engine>/. Browsers: PLAYWRIGHT_BROWSERS_PATH, else Playwright's default folder; an
// engine whose browser isn't installed is skipped.
import http from 'node:http';
import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, statSync } from 'node:fs';
import { extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, devices, webkit, type Browser, type BrowserContext, type BrowserType, type Page } from 'playwright';
import WebSocket from 'ws';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { b64u, utf8 } from '../src/lib/bytes';
import type { WsFactory, WsLike } from '../src/lib/relaypool';
import { TestDesktop, VECTOR_D } from '../src/lib/testing/desktop';

const here = fileURLToPath(new URL('.', import.meta.url));
const DIST = resolve(here, '../dist');
const DIST_LOCAL = resolve(here, '../dist-local');
const RELAY = resolve(here, '../../dev/test-relay.mjs');
const nodeWs: WsFactory = (url) => new WebSocket(url) as unknown as WsLike;

const TYPES: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript',
  '.css': 'text/css',
  '.png': 'image/png',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
};

/** A static server for a built folder on 127.0.0.1 (no extra headers, like GitHub Pages: the page's own CSP rules). */
function serve(dir: string): Promise<{ base: string; close: () => void }> {
  const server = http.createServer((req, res) => {
    const p = normalize(decodeURIComponent(new URL(req.url ?? '/', 'http://x').pathname)).replace(/^(\.\.[/\\])+/, '');
    let f = join(dir, p === '/' ? 'index.html' : p);
    if (!f.startsWith(dir) || !existsSync(f) || statSync(f).isDirectory()) f = join(dir, 'index.html');
    res.writeHead(200, { 'content-type': TYPES[extname(f)] ?? 'application/octet-stream', 'cache-control': 'no-store' });
    res.end(readFileSync(f));
  });
  return new Promise((ok) =>
    server.listen(0, '127.0.0.1', () => {
      const port = (server.address() as { port: number }).port;
      ok({ base: `http://127.0.0.1:${port}`, close: () => server.close() });
    }),
  );
}

function startRelay(env: Record<string, string> = {}): Promise<{ url: string; proc: ChildProcess }> {
  return new Promise((ok, fail) => {
    const proc = spawn(process.execPath, [RELAY, '0'], { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'inherit'] });
    let out = '';
    proc.stdout!.on('data', (d) => {
      out += String(d);
      const m = out.match(/listening on (ws:\/\/127\.0\.0\.1:\d+)/);
      if (m) ok({ url: m[1], proc });
    });
    proc.on('exit', (c) => fail(new Error(`relay exited ${c}`)));
  });
}


/** Ask `get` until `ok` holds; fail with the last value after `ms`. */
async function eventually<T>(get: () => T | Promise<T>, ok: (v: T) => boolean, ms = 10_000): Promise<T> {
  const end = Date.now() + ms;
  for (;;) {
    const v = await get();
    if (ok(v)) return v;
    if (Date.now() > end) throw new Error(`still: ${String(JSON.stringify(v)).slice(0, 400)}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

const engines: [string, BrowserType][] = [
  ['chromium', chromium],
  ['webkit', webkit],
];
const installed = (b: BrowserType) => {
  try {
    return existsSync(b.executablePath());
  } catch {
    return false;
  }
};

let prod: Awaited<ReturnType<typeof serve>>;
let local: Awaited<ReturnType<typeof serve>>;
let relay: { url: string; proc: ChildProcess };

beforeAll(async () => {
  if (!existsSync(join(DIST, 'index.html')) || !existsSync(join(DIST_LOCAL, 'index.html'))) {
    throw new Error('Build first: npm run build && npm run build:local');
  }
  prod = await serve(DIST);
  local = await serve(DIST_LOCAL);
  relay = await startRelay({ RELAY_DUP: '1', RELAY_REORDER_MS: '30' });
});
afterAll(() => {
  prod?.close();
  local?.close();
  relay?.proc.kill('SIGTERM');
});

for (const [name, type] of engines) {
  describe.skipIf(!installed(type))(name, () => {
    let browser: Browser;
    let ctx: BrowserContext;
    let page: Page;
    let errors: string[] = [];
    const shots = resolve(here, 'shots', name);
    let n = 0;
    let shotsInTest = 0;
    const shot = async (what: string) => {
      if (process.env.E2E_NO_SHOTS) return;
      shotsInTest++;
      mkdirSync(shots, { recursive: true });
      // caret 'initial': Playwright's default hides the caret with an injected stylesheet, which the page's CSP refuses.
      await page.screenshot({ path: join(shots, `${String(++n).padStart(2, '0')}-${what}.png`), fullPage: true, caret: 'initial' });
    };

    // WebKit's screenshots inject a stylesheet of Playwright's own, which the page's CSP refuses (one console error
    // per screenshot; none without screenshots). Only those, at most one per screenshot, are set aside: anything else,
    // CSP included, still fails the test.
    const SHOT_CSP = "Refused to apply a stylesheet because its hash, its nonce, or 'unsafe-inline' does not appear in the style-src directive of the Content Security Policy.";
    const pageErrors = () => {
      let allowance = name === 'webkit' ? shotsInTest : 0;
      return errors.filter((m) => !(m === SHOT_CSP && allowance-- > 0));
    };

    beforeAll(async () => {
      browser = await type.launch({ headless: true });
      // WebKit plays an iPhone (its user agent), so the page takes the iPhone path: Home Screen first.
      ctx = await browser.newContext({
        ...(name === 'webkit' ? devices['iPhone 14'] : { isMobile: true, hasTouch: true }),
        viewport: { width: 390, height: 844 },
        deviceScaleFactor: 2,
        colorScheme: 'dark',
      });
      page = await ctx.newPage();
      page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
      page.on('pageerror', (e) => errors.push(String(e)));
      // When the page first renders anything, note what the address bar said.
      await page.addInitScript(() => {
        new MutationObserver((_m, o) => {
          if (document.getElementById('app')?.childElementCount) {
            (window as unknown as { hashAtRender: string }).hashAtRender = location.hash;
            o.disconnect();
          }
        }).observe(document, { childList: true, subtree: true });
      });
    });
    afterAll(async () => {
      await browser?.close();
    });

    it('shows the not-paired screen at phone width, without console errors', async () => {
      errors = [];
      shotsInTest = 0;
      await page.goto(prod.base + '/');
      await page.getByRole('button', { name: 'Scan the code' }).waitFor();
      await page.getByText('A remote for the Truthcoin App on your computer.').waitFor();
      await page.getByText('Never use a pairing link someone sent you.').waitFor();
      const width = await page.evaluate(() => document.documentElement.scrollWidth);
      expect(width).toBeLessThanOrEqual(390);
      await shot('not-paired');
      await page.emulateMedia({ colorScheme: 'light' });
      await shot('not-paired-light');
      await page.emulateMedia({ colorScheme: 'dark' });
      expect(pageErrors(), errors.join('\n')).toEqual([]);
    });

    it('reads #pair= and strips it from the address bar before anything renders', async () => {
      errors = [];
      shotsInTest = 0;
      const value = b64u(
        utf8(
          JSON.stringify({
            v: 1,
            r: ['wss://relay.example'],
            n: 'ab'.repeat(32),
            d: VECTOR_D.public,
            c: b64u(new Uint8Array(16).fill(12)),
            x: Math.floor(Date.now() / 1000) + 300,
          }),
        ),
      );
      await page.goto(`${prod.base}/#pair=${value}`);
      await page.locator('#devname, [data-testid=homescreen-first]').first().waitFor();
      expect(await page.evaluate(() => (window as unknown as { hashAtRender: string }).hashAtRender)).toBe('');
      expect(page.url()).not.toContain('pair=');
      if (await page.locator('[data-testid=homescreen-first]').count()) {
        await page.getByRole('button', { name: 'Pair in this browser instead' }).click();
      }
      // The computer's key, as the desktop shows it in Settings › Phone (the vectors' D).
      expect(await page.getByTestId('pair-fingerprint').textContent()).toBe('2bad 0fd6 10d9');
      await shot('pair-form');
      expect(pageErrors(), errors.join('\n')).toEqual([]);
    });

    it('refuses an expiry more than 10 minutes ahead', async () => {
      errors = [];
      shotsInTest = 0;
      const value = b64u(
        utf8(
          JSON.stringify({
            v: 1,
            r: ['wss://relay.example'],
            n: 'ab'.repeat(32),
            d: VECTOR_D.public,
            c: b64u(new Uint8Array(16).fill(12)),
            x: Math.floor(Date.now() / 1000) + 86400,
          }),
        ),
      );
      await page.goto(`${prod.base}/#pair=${value}`);
      await page.getByText('lasts far longer').waitFor();
      expect(pageErrors(), errors.join('\n')).toEqual([]);
    });

    it("doesn't run inside another page's frame", async () => {
      // A fresh page (an about:blank reached from the phone page would carry the phone page's own CSP).
      const framer = await ctx.newPage();
      try {
        await framer.setContent(`<iframe src="${prod.base}/#pair=abc" width="390" height="600"></iframe>`);
        const frame = framer.frameLocator('iframe');
        await frame.getByText('Open this page directly').waitFor();
        expect(await frame.locator('[data-testid=pair]').count()).toBe(0);
      } finally {
        await framer.close();
      }
    });

    it('warns when no code has come, and after Cancel says the code is used', async () => {
      errors = [];
      shotsInTest = 0;
      const desktop = await new TestDesktop({ relays: [relay.url], ws: nodeWs, allowAfterMs: 120_000 }).start();
      desktop.withholdNonce = true;
      try {
        await page.goto(`${local.base}/#pair=${desktop.pairValue()}`);
        await page.locator('#devname, [data-testid=homescreen-first]').first().waitFor();
        if (await page.locator('[data-testid=homescreen-first]').count()) {
          await page.getByRole('button', { name: 'Pair in this browser instead' }).click();
        }
        await page.getByRole('button', { name: 'Pair', exact: true }).click();
        await page.getByTestId('pair-no-code-yet').waitFor();
        await page.getByTestId('pair-no-code-warning').waitFor({ timeout: 15_000 });
        expect(await page.getByTestId('pair-no-code-warning').textContent()).toContain(
          "No code yet: don't allow anything on your computer until this phone shows one.",
        );
        await shot('pair-no-code');
        await page.getByRole('button', { name: 'Cancel' }).click();
        await page.getByTestId('pair-stopped').waitFor();
        expect(await page.getByTestId('pair-stopped').textContent()).toContain('This code is used now');
        await shot('pair-stopped');
        expect(pageErrors(), errors.join('\n')).toEqual([]);
      } finally {
        desktop.stop();
      }
    });

    it('pairs through the relay, trades with a held buy, stays paired after a reload, and forgets', async () => {
      errors = [];
      shotsInTest = 0;
      const desktop = await new TestDesktop({
        relays: [relay.url],
        ws: nodeWs,
        limitSats: 1000,
        heldAfterMs: 2500,
        allowAfterMs: 5000,
      }).start();
      try {
        await page.goto(`${local.base}/#pair=${desktop.pairValue()}`);
        const first = page.locator('#devname, [data-testid=homescreen-first]').first();
        await first.waitFor();
        if (await page.locator('[data-testid=homescreen-first]').count()) {
          await shot('homescreen-first');
          await page.getByRole('button', { name: 'Pair in this browser instead' }).click();
        }
        await page.fill('#devname', 'E2E phone');
        await page.getByRole('button', { name: 'Pair', exact: true }).click();
        const code = page.getByTestId('pair-code');
        await code.waitFor(); // only once the computer's nonce N has come
        await eventually(() => desktop.lastCode, (v) => v !== null);
        expect(await code.textContent()).toBe(desktop.lastCode);
        await shot('pair-code');
        // Reload while the computer is still asking: the page asks before carrying on, then carries on with the same
        // keys and the same code.
        await page.reload();
        await page.getByTestId('pair-resume').waitFor();
        expect(await page.getByTestId('pair-fingerprint').textContent()).toBe('2bad 0fd6 10d9');
        await shot('pair-resume');
        await page.getByRole('button', { name: 'Carry on' }).click();
        await page.getByTestId('pair-code').waitFor();
        expect(await page.getByTestId('pair-code').textContent()).toBe(desktop.lastCode);
        expect(page.url()).not.toContain('pair=');

        // The computer said yes: nothing is kept until this phone's person confirms they allowed it.
        await page.getByTestId('pair-confirm').waitFor({ timeout: 20_000 });
        expect(await page.getByTestId('pair-confirm').textContent()).toContain(String(desktop.lastCode));
        await shot('pair-confirm');
        await page.getByRole('button', { name: 'Yes, I allowed it' }).click();
        await page.getByTestId('home').waitFor({ timeout: 20_000 });
        expect(desktop.contested).toBe(false);
        await eventually(() => page.getByTestId('balance').textContent(), (v) => String(v).includes('4,905,000 sats'), 15_000);
        await eventually(() => page.getByTestId('computer').textContent(), (v) => String(v).includes('block 42'));
        await eventually(() => page.getByTestId('settled').innerText().catch(() => ''), (v) =>
          v.replace(/\s+/g, ' ').includes('Settled: Yes · You got 50,000 sats (50,000 Yes shares; 20,000 No shares paid nothing)'),
        );
        const bal = (await page.getByTestId('balance').innerText()).replace(/\s+/g, ' ');
        expect(bal).toContain('Held by 1 waiting trade 100,000 sats');
        expect(bal).toContain('On its way to eCash 250,000 sats (pays out in days)');
        await eventually(() => page.getByTestId('positions').textContent(), (v) => String(v).includes('100,000 shares'));
        await shot('home');
        await page.emulateMedia({ colorScheme: 'light' });
        await shot('home-light');
        await page.emulateMedia({ colorScheme: 'dark' });

        await page.getByRole('button', { name: 'Markets' }).click();
        await page.getByText('Will it rain in Lisbon').waitFor();
        // A trading market's row shows its leading outcome's chance; a settled one's doesn't.
        const list = (await page.getByTestId('markets').innerText()).replace(/\s+/g, ' ');
        expect(list).toContain('Yes 53% · 2 outcomes');
        expect(list).not.toContain('Yes 98%');
        await shot('markets');
        // A settled market says what each share paid, not the last chances.
        await page.getByText('Will the betanet reach block 20,000').click();
        await page.getByTestId('resolution').waitFor();
        expect(await page.getByTestId('resolution').textContent()).toContain(
          'Settled: Yes. Each Yes share paid 1 sat; No paid nothing.',
        );
        const settled = String(await page.getByTestId('outcomes').textContent());
        expect(settled).toContain('paid 1 sat a share');
        expect(settled).not.toContain('pays 1 sat if');
        expect(await page.getByTestId('decisions').innerText()).toContain('Decided in period 3');
        await shot('market-settled');
        await page.getByRole('button', { name: 'Back' }).click();
        await page.getByText('Will it rain in Lisbon').click();
        await page.getByTestId('outcomes').waitFor();
        await eventually(() => page.getByTestId('outcomes').textContent(), (v) => String(v).includes('53%'));
        expect(await page.getByTestId('market-fees').textContent()).toContain('(at least 1,000 sats a trade) + 1,000');
        const how = (await page.getByTestId('decisions').innerText()).replace(/\s+/g, ' ');
        expect(how).toContain("How it's decided: IPMA records for Lisbon on 10 October 2026.");
        expect(how).toContain('Voters decide in period 3 (in about 20 blocks)');
        await shot('market');

        // Buy "Yes" (the second outcome's Buy). 2,000 shares can't pay back their fees: a red warning and a second tap.
        await page.getByTestId('outcomes').getByRole('button', { name: 'Buy' }).nth(1).click();
        expect(await page.locator('#shares').getAttribute('placeholder')).toBe('for example 50,000');
        await page.fill('#shares', '2,000');
        await page.getByRole('button', { name: 'Get a price' }).click();
        await page.getByTestId('quote-loses').waitFor();
        expect(await page.getByTestId('quote-loses').textContent()).toContain('This costs more than it can ever pay back');
        await shot('quote-loses');
        await page.getByRole('button', { name: 'Buy 2,000 Yes' }).click();
        await page.getByRole('button', { name: 'Buy anyway, at a loss' }).waitFor();
        expect(await page.locator('[data-testid=flow]').count()).toBe(0); // the first tap sent nothing
        // 50,000 shares instead.
        await page.fill('#shares', '50,000');
        await page.getByRole('button', { name: 'Get a price' }).click();
        await page.getByTestId('quote').waitFor();
        const q = String(await page.getByTestId('quote').textContent()).replace(/\s+/g, ' ');
        expect(q).toMatch(/About [\d,]+ sats, at most [\d,]+ sats/);
        expect(q).toMatch(/Fees [\d,]+ sats \(\d+% of this trade\)/);
        expect(q).toMatch(/This trade counts [\d,]+ sats against this phone's limit, more than the 1,000 sats left today/);
        expect(q).not.toContain('can ever pay back');
        await shot('quote');
        await page.getByRole('button', { name: 'Buy 50,000 Yes' }).click();
        await page.locator('[data-testid=flow][data-state=held]').waitFor({ timeout: 15_000 });
        expect(await page.getByTestId('last-line').textContent()).toContain('Waiting for your OK on the computer');
        await shot('trade-held');
        await page.locator('[data-testid=flow][data-state=pending]').waitFor({ timeout: 15_000 });
        expect((await page.getByTestId('flow').innerText()).replace(/\s+/g, ' ')).toMatch(
          /if the price is still within your most \([\d,]+ sats\)/,
        );
        await shot('trade-pending');
        const tradeIds = [...desktop.runs.keys()];
        expect(tradeIds.length).toBe(1);
        expect(desktop.runs.get(tradeIds[0])).toBe(1);

        // Back to Home: it asks again, and shows the new block, balance and shares.
        await page.getByRole('button', { name: 'Done' }).click();
        await page.getByTestId('home').waitFor();
        const total = desktop.total.toLocaleString('en-US');
        await eventually(() => page.getByTestId('balance').textContent(), (v) => String(v).includes(`${total} sats`), 15_000);
        await eventually(() => page.getByTestId('computer').textContent(), (v) => String(v).includes('block 43'), 15_000);
        await eventually(() => page.getByTestId('positions').textContent(), (v) => String(v).includes('150,000 shares'), 15_000);
        expect(await page.getByTestId('computer').textContent()).toContain('Left today for trades');
        // The "Sent…" card turns into what happened, in place, with the miner fee counted.
        const cost = (desktop.tradeSats.get(tradeIds[0]) ?? 0) + 1000;
        await eventually(
          () => page.getByTestId('flow-done').textContent(),
          (v) => String(v).includes(`Done: bought 50,000 Yes for ${cost.toLocaleString('en-US')} sats.`),
          15_000,
        );
        expect(await page.getByTestId('recent').innerText()).toContain(`about ${cost.toLocaleString('en-US')} sats`);
        await shot('home-after-trade');

        // The computer goes quiet (shut down, or the app closed there): the header, a banner and Home say so, with the
        // time of what's shown, and it all clears when the computer answers again (operator 2026-10-07).
        expect(await page.getByTestId('conn').textContent()).toContain('Computer connected');
        desktop.asleep = true;
        await page.getByRole('button', { name: 'Refresh' }).click();
        await page.getByTestId('reach').waitFor({ timeout: 45_000 });
        expect(await page.getByTestId('conn').textContent()).toContain('Computer not answering');
        expect(await page.getByTestId('reach').textContent()).toContain('The Truthcoin App has to be open on your computer');
        expect(await page.getByTestId('computer-silent').textContent()).toContain('Not answering since');
        expect(await page.getByTestId('as-of').textContent()).toMatch(/^as of /);
        expect(await page.getByTestId('balance').textContent()).toContain(`${total} sats`); // still shown, dated
        await shot('computer-silent');
        desktop.asleep = false;
        await page.getByTestId('reach').getByRole('button', { name: 'Ask again' }).click();
        await page.getByTestId('reach').waitFor({ state: 'detached', timeout: 30_000 });
        expect(await page.getByTestId('conn').textContent()).toContain('Computer connected');
        expect(await page.getByTestId('computer-silent').count()).toBe(0);

        // Reload: still paired, with a key no script can read.
        await page.reload();
        await page.getByTestId('home').waitFor();
        await eventually(() => page.getByTestId('balance').textContent(), (v) => String(v).includes(`${total} sats`), 15_000);
        const key = await page.evaluate(
          () =>
            new Promise<{ extractable: boolean; exported: boolean }>((ok, fail) => {
              const r = indexedDB.open('truthcoin-phone', 1);
              r.onerror = () => fail(r.error);
              r.onsuccess = () => {
                const g = r.result.transaction('kv').objectStore('kv').get('pairing');
                g.onsuccess = async () => {
                  r.result.close(); // an open connection would hold up "Forget" (deleteDatabase) later
                  const k = g.result.pPriv as CryptoKey;
                  const exported = await crypto.subtle.exportKey('jwk', k).then(
                    () => true,
                    () => false,
                  );
                  ok({ extractable: k.extractable, exported });
                };
              };
            }),
        );
        expect(key).toEqual({ extractable: false, exported: false });

        await page.getByRole('button', { name: 'Receive' }).click();
        await eventually(() => page.getByTestId('receive').textContent(), (v) => String(v).includes('s13_1TruthTestAddr9xyzQ4mK_5f2a1c'), 15_000);
        await shot('receive');

        await page.getByRole('button', { name: 'Settings' }).click();
        await page.getByTestId('settings').waitFor();
        expect(await page.getByTestId('settings').textContent()).toContain('E2E phone');
        await shot('settings');
        await page.getByRole('button', { name: 'Forget this computer' }).click();
        await page.getByTestId('forget-confirm').click();
        await page.getByRole('button', { name: 'Scan the code' }).waitFor({ timeout: 15_000 });
        expect(desktop.unpaired.length).toBe(1); // the computer was asked to forget this phone, and did
        expect(await page.locator('[data-testid=notice], [data-testid=load-error]').count()).toBe(0);
        await page.reload();
        await page.getByRole('button', { name: 'Scan the code' }).waitFor();
        expect(pageErrors(), errors.join('\n')).toEqual([]);
      } finally {
        desktop.stop();
      }
    });
  });
}
