// End to end through dev/test-relay.mjs (a real NIP-01 relay process on 127.0.0.1) that repeats, reorders, drops
// and rate-limits events: a test desktop pairs with this page's pairing code, answers `status`, holds a buy over the
// phone's limit until "confirmed", and never runs a repeated trade twice.
import { spawn, type ChildProcess } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import WebSocket from 'ws';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { Api } from './api';
import { TradeFlows } from './flows';
import { NoAnswerError, PhoneLink } from './link';
import { attemptFor, linkKeys, pairPhone, parsePairValue, PairRefusedError, type Pairing } from './pairing';
import { b64u } from './bytes';
import type { WsFactory, WsLike } from './relaypool';
import { loadAttempt, memoryKv, saveAttempt } from './store';
import { TestDesktop } from './testing/desktop';
import { waitFor } from './testing/fakews';

const RELAY = fileURLToPath(new URL('../../../dev/test-relay.mjs', import.meta.url));
const ws: WsFactory = (url) => new WebSocket(url) as unknown as WsLike;

interface Relay {
  url: string;
  proc: ChildProcess;
}
const relays: Relay[] = [];

/** Start dev/test-relay.mjs on a free port with the given misbehaviour. */
function startRelay(env: Record<string, string>): Promise<Relay> {
  return new Promise((resolve, reject) => {
    const proc = spawn(process.execPath, [RELAY, '0'], { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'inherit'] });
    let out = '';
    proc.stdout!.on('data', (d) => {
      out += String(d);
      const m = out.match(/listening on (ws:\/\/127\.0\.0\.1:\d+)/);
      if (m) {
        const r = { url: m[1], proc };
        relays.push(r);
        resolve(r);
      }
    });
    proc.on('error', reject);
    proc.on('exit', (code) => reject(new Error(`relay exited (${code})`)));
  });
}

afterAll(() => {
  for (const r of relays) r.proc.kill('SIGTERM'); // by PID
});

async function pairWith(desktop: TestDesktop, name = 'Test phone') {
  const link = await parsePairValue(desktop.pairValue(), true);
  let code = '';
  const p = await pairPhone(link, name, { ws, onCode: (c) => (code = c), resendAt: [300, 900, 2000], probeEveryMs: 500 });
  return { pairing: p, code };
}

function phoneLink(p: Pairing, o: ConstructorParameters<typeof PhoneLink>[2] = {}) {
  const link = new PhoneLink(linkKeys(p), p.relays, { ws, backoffMin: 100, backoffMax: 1000, ...o });
  link.start();
  return link;
}

describe('through a relay that repeats and reorders everything', () => {
  let url = '';
  beforeAll(async () => {
    url = (await startRelay({ RELAY_DUP: '2', RELAY_REORDER_MS: '60' })).url;
  });

  it('pairs (same comparison code on both ends), answers status, and holds a buy until confirmed', async () => {
    const desktop = await new TestDesktop({ relays: [url], ws, limitSats: 1000, heldAfterMs: 400 }).start();
    try {
      const { pairing, code } = await pairWith(desktop, 'Test‮ phone');
      expect(code).toMatch(/^\d{3} \d{3}$/);
      expect(code).toBe(desktop.lastCode);
      expect(pairing.name).toBe('Test phone');
      expect(pairing.limitSats).toBe(1000);
      expect(pairing.nd).toBe(desktop.npub);

      const link = phoneLink(pairing);
      const api = new Api(link);
      const s = await api.status();
      expect(s).toMatchObject({ height: 42, synced: true, node: 'running', leftSats: 1000, relays: [url] });

      const flows = new TradeFlows(link, memoryKv());
      const seen: string[] = [];
      const unsub = flows.store.subscribe((l) => l[0] && seen.push(l[0].state.k));
      const f = await flows.start(
        { marketId: 'a1b2c3d4e5f6', outcome: 1, shares: 100000, side: 'buy', limit: 54743 },
        { title: 'Will it rain?', label: 'Yes' },
      );
      await waitFor(() => flows.get(f.id)!.state.k === 'pending', 10_000);
      unsub();
      expect(seen).toContain('held');
      expect(seen.indexOf('held')).toBeLessThan(seen.indexOf('pending'));
      expect(desktop.runs.get(f.id)).toBe(1);
      link.stop();
    } finally {
      desktop.stop();
    }
  });

  it('never runs a retried buy twice, however often it is asked', async () => {
    const desktop = await new TestDesktop({ relays: [url], ws, limitSats: 1_000_000, tradeMs: 800 }).start();
    try {
      const { pairing } = await pairWith(desktop);
      const link = phoneLink(pairing);
      const req = link.makeRequest('trade', { id: 'a1b2c3d4e5f6', outcome: 0, shares: 10, side: 'buy', limit: 20 });
      // The node is slow: the first ask gives up before the answer, after sending the request twice.
      await expect(link.ask(req, { timeoutMs: 400, resendAt: [150] })).rejects.toThrow(NoAnswerError);
      // Asking again (same id) gets the one answer.
      const r = (await link.ask(req, { timeoutMs: 5000, resendAt: [300, 600] })) as { txid: string };
      expect(r.txid).toMatch(/^[0-9a-f]{64}$/);
      expect(desktop.requests.filter((x) => x.id === req.id).length).toBeGreaterThanOrEqual(3);
      expect(desktop.runs.get(req.id)).toBe(1);
      link.stop();
    } finally {
      desktop.stop();
    }
  });

  it('asks again about a trade left without an answer when the page reopens, under the same id', async () => {
    const desktop = await new TestDesktop({ relays: [url], ws, limitSats: 1_000_000 }).start();
    try {
      const { pairing } = await pairWith(desktop);
      const kv = memoryKv();
      desktop.o.silent = true; // the app on the computer is busy or closed
      const link1 = phoneLink(pairing);
      const flows1 = new TradeFlows(link1, kv, undefined, 400);
      const f = await flows1.start(
        { marketId: 'a1b2c3d4e5f6', outcome: 1, shares: 5, side: 'buy', limit: 10 },
        { title: 'T', label: 'Yes' },
      );
      await waitFor(() => flows1.get(f.id)!.state.k === 'unconfirmed', 5000);
      link1.stop(); // the page closes

      desktop.o.silent = false;
      const link2 = phoneLink(pairing); // the page opens again
      const flows2 = new TradeFlows(link2, kv);
      await flows2.resume();
      expect(flows2.get(f.id)?.req).toEqual(f.req);
      await waitFor(() => flows2.get(f.id)!.state.k === 'pending', 10_000);
      // (Status probes from pairing may be there too: only trades count here.)
      const trades = desktop.requests.filter((x) => x.m === 'trade');
      expect(trades.length).toBeGreaterThan(0);
      expect(trades.every((x) => x.id === f.id && x.ts === f.req.ts)).toBe(true);
      expect(desktop.runs.get(f.id)).toBe(1);
      link2.stop();
    } finally {
      desktop.stop();
    }
  });

  it('keeps one key per pairing code: after a reload it carries on, and the code is not contested', async () => {
    const desktop = await new TestDesktop({ relays: [url], ws, allowAfterMs: 2500 }).start();
    try {
      const link = await parsePairValue(desktop.pairValue(), true);
      const kv = memoryKv();
      const fast = { ws, resendAt: [300, 900], probeEveryMs: 500 };
      // First load: the request reaches the desktop, then the page reloads before the answer.
      let gone = false;
      let code1 = '';
      const first = pairPhone(link, 'Phone A', { ...fast, keep: (a) => saveAttempt(kv, a), onCode: (c) => (code1 = c), cancelled: () => gone });
      await waitFor(() => desktop.claim !== null, 10_000);
      gone = true;
      await expect(first).rejects.toThrow('cancelled');
      // Second load: the kept attempt, the same keys, id and name (whatever the name field says now).
      const kept = attemptFor(await loadAttempt(kv), link, Math.floor(Date.now() / 1000));
      expect(kept).not.toBeNull();
      let code2 = '';
      const p = await pairPhone(link, 'Another name', { ...fast, attempt: kept, onCode: (c) => (code2 = c) });
      expect(desktop.contested).toBe(false);
      expect(code2).toBe(code1);
      expect(code2).toBe(desktop.lastCode);
      expect(p.pPub).toBe(b64u(kept!.pPub));
      expect(p.npub).toBe(kept!.npub);
      expect(p.name).toBe('Phone A');
      expect(desktop.claim?.id).toBe(kept!.id);
    } finally {
      desktop.stop();
    }
  });

  it('gets the pairing refused when a second key answers the same code', async () => {
    const desktop = await new TestDesktop({ relays: [url], ws, allowAfterMs: 2500 }).start();
    try {
      const link = await parsePairValue(desktop.pairValue(), true);
      const fast = { ws, resendAt: [300, 900], probeEveryMs: 500 };
      let gone = false;
      const first = pairPhone(link, 'Phone A', { ...fast, cancelled: () => gone });
      await waitFor(() => desktop.claim !== null, 10_000);
      gone = true;
      await expect(first).rejects.toThrow('cancelled');
      // New keys for the same code: to the desktop, a second phone. The code is contested and nobody is paired (the
      // owner can only refuse; the refusal goes to the phone that claimed the code first, so this one hears nothing).
      let gone2 = false;
      const second = pairPhone(link, 'Phone A', { ...fast, cancelled: () => gone2 });
      await waitFor(() => desktop.contested, 10_000);
      await waitFor(() => !desktop.codeLive, 10_000);
      gone2 = true;
      await expect(second).rejects.toThrow('cancelled');
      expect(desktop.phones.size).toBe(0);
    } finally {
      desktop.stop();
    }
  });

  it('finds out it was paired even when the pairing answer is lost', async () => {
    const desktop = await new TestDesktop({ relays: [url], ws, limitSats: 777, losePairAnswer: true }).start();
    try {
      const { pairing } = await pairWith(desktop, 'Lost answer');
      expect(pairing.name).toBe('Lost answer');
      expect(pairing.limitSats).toBe(777); // from the status the probe got
      expect(desktop.requests.some((r) => r.m === 'status')).toBe(true);
    } finally {
      desktop.stop();
    }
  });

  it('reports a refused pairing, with the desktop\'s words', async () => {
    const desktop = await new TestDesktop({ relays: [url], ws, allow: false }).start();
    try {
      await expect(pairWith(desktop)).rejects.toThrow(PairRefusedError);
    } finally {
      desktop.stop();
    }
    const words = 'Two phones tried to pair with this code: refuse, and start again';
    const contested = await new TestDesktop({ relays: [url], ws, allow: false, refuseWith: words }).start();
    try {
      await expect(pairWith(contested)).rejects.toThrow(words);
    } finally {
      contested.stop();
    }
  });
});

describe('through a relay that drops events or rate-limits', () => {
  it('still gets answers when a third of the events are lost', { timeout: 60_000 }, async () => {
    const { url } = await startRelay({ RELAY_DROP: '0.33', RELAY_REORDER_MS: '20' });
    const desktop = await new TestDesktop({ relays: [url], ws }).start();
    try {
      const { pairing } = await pairWith(desktop);
      const link = phoneLink(pairing, { resendAt: [300, 600, 900, 1200, 1500, 2000, 2500, 3500, 5000], timeoutMs: 9000 });
      const api = new Api(link);
      for (let i = 0; i < 3; i++) expect((await api.status()).height).toBe(42);
      link.stop();
    } finally {
      desktop.stop();
    }
  });

  it('backs off on "rate-limited" and sends again', { timeout: 60_000 }, async () => {
    const { url } = await startRelay({ RELAY_RATELIMIT_EVERY: '2' });
    const desktop = await new TestDesktop({ relays: [url], ws }).start();
    try {
      const { pairing } = await pairWith(desktop);
      const link = phoneLink(pairing, { timeoutMs: 15_000, resendAt: [] });
      expect((await new Api(link).status()).height).toBe(42);
      link.stop();
    } finally {
      desktop.stop();
    }
  });
});
