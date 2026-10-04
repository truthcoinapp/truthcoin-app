// A stand-in for the desktop app, for tests: it pairs phones, answers `status`, `quote` and `trade` (holding trades
// over the phone's limit until "confirmed"), keeps every request id with its answers and answers repeats from that
// record without running them again, as PROTOCOL.md asks of the real desktop. It uses this page's own crypto.
import { b64u, fromHex, fromUtf8, hex, isRequestId, randomBytes, type Bytes } from '../bytes';
import { envelopeJson, importPrivate, openMsg, openPair, padJson, pairCode, parseEnvelope, pubBytes, sealMsg, unpadJson } from '../crypto';
import { KIND, messageEvent, newNostrSecret, nostrPub, tag, type NostrEvent } from '../nostr';
import { RelayPool, type WsFactory } from '../relaypool';
import { cleanName } from '../text';

export const VECTOR_D = {
  secret: '1111111111111111111111111111111111111111111111111111111111111111',
  public: 'BAIX5hfwtkQ5KCePlpmeaaI6TywVK99tbN9m5bgCgtTtGUp968uXcS0t2jyoWqh2Wlb0X8dYWZZS8ol8ZTBuV5Q',
};

export interface DesktopOptions {
  relays: string[];
  ws?: WsFactory;
  allow?: boolean;
  limitSats?: number;
  /** Trades over this many sats are held until `confirmAfterMs` passes. */
  heldAfterMs?: number;
  /** Time a trade takes at the node (answers come after it). */
  tradeMs?: number;
  /** Don't answer at all (the app is closed). */
  silent?: boolean;
  /** Allow pairing but lose the answer (every relay dropped it). */
  losePairAnswer?: boolean;
  /** Take this long to "click Allow" after the request arrives. */
  allowAfterMs?: number;
  /** Refuse pairing with these words (instead of "not allowed"). */
  refuseWith?: string;
}

interface Phone {
  pPub: Bytes;
  name: string;
  left: number;
}

export class TestDesktop {
  dPriv!: CryptoKey;
  readonly dPub = pubBytes(VECTOR_D.public);
  readonly nsec = newNostrSecret();
  readonly npub = nostrPub(this.nsec);
  c: Bytes = randomBytes(16);
  x = Math.floor(Date.now() / 1000) + 300;
  codeLive = true;
  lastCode: string | null = null;
  readonly phones = new Map<string, Phone>();
  /** Every request id seen, with the answers sent under it (held first, then final). */
  readonly record = new Map<string, unknown[]>();
  /** How many times each trade really ran. */
  readonly runs = new Map<string, number>();
  readonly requests: { id: string; ts: number; m: string; a: Record<string, unknown> }[] = [];
  pool!: RelayPool;
  /** The wallet as the answers show it; each trade that runs is "mined" into the next block. */
  height = 42;
  total = 4_905_000;
  boughtYes = 0;

  constructor(readonly o: DesktopOptions) {}

  async start(): Promise<this> {
    this.dPriv = await importPrivate(fromHex(VECTOR_D.secret), this.dPub);
    this.pool = new RelayPool({
      relays: this.o.relays,
      ws: this.o.ws,
      filter: () => ({ kinds: [KIND], '#p': [this.npub], since: Math.floor(Date.now() / 1000) - 120 }),
      onEvent: (e) => void this.onEvent(e).catch(() => undefined),
      backoffMin: 50,
      backoffMax: 500,
    });
    this.pool.start();
    return this;
  }

  stop() {
    this.pool?.stop();
  }

  /** The QR code's `#pair=` value. */
  pairValue(relays = this.o.relays): string {
    const json = JSON.stringify({ v: 1, r: relays, n: this.npub, d: VECTOR_D.public, c: b64u(this.c), x: this.x });
    return b64u(new TextEncoder().encode(json));
  }

  private async send(to: string, pPub: Bytes, obj: unknown) {
    const env = await sealMsg({ sPriv: this.dPriv, sPub: this.dPub, rPub: pPub }, padJson(obj));
    await this.pool.publish(messageEvent(this.nsec, to, envelopeJson(env), Math.floor(Date.now() / 1000)));
  }

  private async onEvent(e: NostrEvent) {
    if (this.o.silent || tag(e, 'p') !== this.npub) return;
    const env = parseEnvelope(e.content);
    if (!env) return;
    const phone = this.phones.get(e.pubkey);
    if (!phone) {
      if (env.k === 'pair' && this.codeLive) await this.onPair(e, env);
      return;
    }
    if (env.k !== 'msg') return;
    let req: { id: string; ts: number; m: string; a: Record<string, unknown> };
    try {
      req = unpadJson(await openMsg({ rPriv: this.dPriv, rPub: this.dPub, sPub: phone.pPub }, env)) as typeof req;
    } catch {
      return;
    }
    if (!isRequestId(req.id)) return;
    this.requests.push(req);
    const known = this.record.get(req.id);
    if (known) {
      for (const r of known) await this.send(e.pubkey, phone.pPub, r);
      return;
    }
    const answers: unknown[] = [];
    this.record.set(req.id, answers); // kept before anything runs
    const answer = async (r: unknown) => {
      answers.push(r);
      await this.send(e.pubkey, phone.pPub, r);
    };
    if (Math.abs(Math.floor(Date.now() / 1000) - req.ts) > 300) {
      await answer({ re: req.id, err: 'This request is too old, so it was not done.' });
      return;
    }
    if (req.m === 'status') {
      await answer({
        re: req.id,
        ok: {
          app: '0.1.0',
          node: 'running',
          height: this.height,
          synced: true,
          network: 'betanet',
          name: phone.name,
          limit_sats: this.o.limitSats ?? 100_000,
          left_sats: phone.left,
          relays: this.o.relays,
        },
      });
    } else if (req.m === 'trade') {
      const cap = Number(req.a.limit);
      const run = async () => {
        await new Promise((r) => setTimeout(r, this.o.tradeMs ?? 0));
        this.runs.set(req.id, (this.runs.get(req.id) ?? 0) + 1);
        if (req.a.side === 'buy') phone.left = Math.max(0, phone.left - cap);
        this.height += 1;
        if (req.a.side === 'buy') {
          this.total -= cap;
          if (Number(req.a.outcome) === 1) this.boughtYes += Number(req.a.shares);
        }
        await answer({ re: req.id, ok: { status: 'pending', txid: hex(randomBytes(32)) } });
      };
      if (req.a.side === 'buy' && cap > phone.left) {
        await answer({ re: req.id, held: { text: `Phone ${phone.name} wants to buy for at most ${cap} sats` } });
        setTimeout(() => void run(), this.o.heldAfterMs ?? 300);
      } else {
        await run();
      }
    } else if (req.m in FIXTURES) {
      await answer({ re: req.id, ok: FIXTURES[req.m](req.a, this) });
    } else {
      await answer({ re: req.id, err: `unknown method ${String(req.m).slice(0, 20)}` });
    }
  }

  /** The phone that opened the code first (shown as "Allow this phone?"), and whether another key opened it too. */
  claim: { p: string; np: string; id: string; name: string } | null = null;
  contested = false;

  private async onPair(e: NostrEvent, env: ReturnType<typeof parseEnvelope> & object) {
    let req: { t: string; p: string; np: string; name: string; id: string };
    let ePub: Bytes;
    try {
      const o = await openPair({ dPriv: this.dPriv, dPub: this.dPub, c: this.c }, env);
      req = JSON.parse(fromUtf8(o.pt));
      ePub = o.ePub;
    } catch {
      return;
    }
    if (this.claim) {
      // The live code opened again: by the same phone (a retry), or by another key (someone else saw the code).
      if (typeof req.p === 'string' && req.p !== this.claim.p) this.contested = true;
      return;
    }
    if (req.t !== 'pair' || req.np !== e.pubkey || !isRequestId(req.id)) return;
    const pPub = pubBytes(req.p);
    const name = cleanName(req.name);
    this.claim = { p: b64u(pPub), np: e.pubkey, id: req.id, name };
    this.lastCode = await pairCode(this.dPub, pPub, ePub, this.c);
    if (this.o.allowAfterMs) await new Promise((r) => setTimeout(r, this.o.allowAfterMs));
    this.codeLive = false; // allowing or refusing uses the code up
    if (this.contested || this.o.allow === false) {
      // A contested code can't be allowed: the owner refuses it, and the phone hears "not allowed".
      const err = this.contested ? 'not allowed' : this.o.refuseWith ?? 'not allowed';
      await this.send(e.pubkey, pPub, { re: req.id, err });
      return;
    }
    this.phones.set(e.pubkey, { pPub, name, left: this.o.limitSats ?? 100_000 });
    if (this.o.losePairAnswer) return;
    await this.send(e.pubkey, pPub, { re: req.id, ok: { paired: true, name, limit_sats: this.o.limitSats ?? 100_000 } });
  }
}

// Answers for the read methods, so a page driven against this desktop shows every screen.
const MARKETS = [
  {
    id: 'a1b2c3d4e5f6',
    title: 'Will it rain in Lisbon on 10 October 2026?',
    description: 'Resolves Yes if the IPMA station at Lisbon records any rain on that day.\nOtherwise No.',
    state: 'trading',
    fee_rate: 0.01,
    volume: 52250,
    outcomes: [
      { i: 0, label: 'No', price: 0.475, volume: 0 },
      { i: 1, label: 'Yes', price: 0.525, volume: 52250 },
    ],
    resolution: null,
    created: 31,
  },
  {
    id: 'b2c3d4e5f6a1',
    title: 'Will the betanet reach block 20,000 before November?',
    description: '',
    state: 'settled',
    fee_rate: 0.01,
    volume: 1200000,
    outcomes: [
      { i: 0, label: 'No', price: 0.02, volume: 200000 },
      { i: 1, label: 'Yes', price: 0.98, volume: 1000000 },
    ],
    resolution: { summary: 'Resolved: Yes', winners: [1] },
    created: 12,
  },
];

const FIXTURES: Record<string, (a: Record<string, unknown>, d: TestDesktop) => unknown> = {
  markets: () => ({
    markets: MARKETS.map((m) => ({ id: m.id, title: m.title, state: m.state, outcomes: m.outcomes.length, volume: m.volume, created: m.created })),
    page: 0,
    pages: 1,
  }),
  market: (a) => {
    const m = MARKETS.find((x) => x.id === a.id) ?? MARKETS[0];
    const { created: _, ...rest } = m;
    return { ...rest, holdings: m.id === MARKETS[0].id ? [{ outcome: 1, shares: 100000, value: 52500 }] : [] };
  },
  positions: (_a, d) => {
    const shares = 100000 + d.boughtYes;
    const value = Math.round(shares * 0.525);
    return {
      positions: [
        { market_id: MARKETS[0].id, title: MARKETS[0].title, state: 'trading', outcome: 1, label: 'Yes', shares, price: 0.525, value, paid: 52250 },
      ],
      total_value: value,
    };
  },
  balance: (_a, d) => ({ total: d.total, available: d.total - 100_000, in_pending_trades: 100_000, pending_trades: 1 }),
  quote: (a) => {
    const m = MARKETS.find((x) => x.id === a.id) ?? MARKETS[0];
    const o = m.outcomes.find((x) => x.i === Number(a.outcome)) ?? m.outcomes[0];
    const shares = Number(a.shares);
    const gross = Math.round(shares * o.price);
    const fee = Math.round(gross * 0.01);
    const buy = a.side === 'buy';
    const sats = buy ? gross + fee : gross - fee;
    const margin = Math.round(sats * 0.02);
    return {
      side: a.side,
      sats,
      fee,
      miner_fee: 1000,
      price_now: o.price,
      price_after: Math.min(0.999, Math.max(0.001, o.price + (buy ? 0.025 : -0.025))),
      limit: buy ? sats + 1000 + margin : Math.max(0, sats - 1000 - margin),
    };
  },
  trades: (_a, d) => ({
    trades: d.requests
      .filter((r) => r.m === 'trade' && d.runs.has(r.id))
      .filter((r, i, l) => l.findIndex((x) => x.id === r.id) === i)
      .map((r) => ({
        id: r.id,
        time: r.ts,
        title: MARKETS[0].title,
        label: Number(r.a.outcome) === 1 ? 'Yes' : 'No',
        side: r.a.side,
        shares: r.a.shares,
        sats: null,
        limit: r.a.limit,
        status: 'pending',
        txid: null,
      })),
  }),
  receive: () => ({ address: '1TruthTestAddr9xyzQ4mK', deposit_address: 's13_1TruthTestAddr9xyzQ4mK_5f2a1c' }),
};
