// A small Nostr client (NIP-01) for the phone link: one WebSocket per relay, one subscription on each, every event
// published to every relay. Relays are untrusted: every incoming event's id and signature are checked, then
// duplicates (by event id) dropped, before anything else sees it. Dropped connections reconnect with backoff, and a
// relay that answers `OK false "rate-limited: …"` gets nothing more for a while. One working relay is enough.
import { hex, randomBytes } from './bytes';
import { verifyEvent, type NostrEvent } from './nostr';

/** The part of the browser's WebSocket the pool uses (the tests pass `ws`'s). */
export interface WsLike {
  readonly readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  onopen: ((ev: any) => void) | null;
  onmessage: ((ev: any) => void) | null;
  onclose: ((ev: any) => void) | null;
  onerror: ((ev: any) => void) | null;
}
export type WsFactory = (url: string) => WsLike;

export const browserWs: WsFactory = (url) => new WebSocket(url) as unknown as WsLike;

export interface Filter {
  kinds: number[];
  '#p': string[];
  since: number;
}

/** `open`: connected and subscribed; `connecting`; `waiting`: backing off before connecting again. */
export type RelayState = 'connecting' | 'open' | 'waiting';

export interface RelayInfo {
  url: string;
  state: RelayState;
  /** Why it is waiting, or what it last said, for Settings. */
  note: string;
}

/** What publishing an event came to: how many relays took it, and what the others said. */
export interface Published {
  accepted: number;
  refused: string[];
}

export interface PoolOptions {
  relays: string[];
  /** The subscription's filter, made fresh at each (re)subscribe so `since` stays recent. */
  filter: () => Filter;
  onEvent: (e: NostrEvent) => void;
  ws?: WsFactory;
  onChange?: (relays: RelayInfo[]) => void;
  log?: (msg: string) => void;
  /** Reconnect backoff, ms (doubling, with jitter). */
  backoffMin?: number;
  backoffMax?: number;
  /** The first pause after `OK false "rate-limited: …"`, ms (doubling up to a minute). */
  rateBackoffMs?: number;
  connectTimeoutMs?: number;
}

const SEEN_MAX = 4096;
const MAX_FRAME = 256 * 1024;

interface Outgoing {
  ev: NostrEvent;
  until: number;
  rateTries: number;
}

class Relay {
  ws: WsLike | null = null;
  state: RelayState = 'waiting';
  note = '';
  private attempts = 0;
  private stopped = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  private connectTimer: ReturnType<typeof setTimeout> | undefined;
  private flushTimer: ReturnType<typeof setTimeout> | undefined;
  private resubTimer: ReturnType<typeof setTimeout> | undefined;
  private queue: Outgoing[] = [];
  private inflight = new Map<string, Outgoing>();
  private holdUntil = 0;
  private rateBackoff: number;
  private readonly sub = 'tc-' + hex(randomBytes(6));

  constructor(
    readonly url: string,
    private readonly pool: RelayPool,
  ) {
    this.rateBackoff = pool.rateBackoffMs;
  }

  connect() {
    if (this.stopped) return;
    clearTimeout(this.reconnectTimer);
    if (this.ws) return;
    this.state = 'connecting';
    this.pool.changed();
    let ws: WsLike;
    try {
      ws = this.pool.wsf(this.url);
    } catch {
      this.lost(null, 'could not connect');
      return;
    }
    this.ws = ws;
    this.connectTimer = setTimeout(() => {
      if (this.ws === ws && this.state === 'connecting') this.lost(ws, 'no answer');
    }, this.pool.connectTimeoutMs);
    ws.onopen = () => {
      if (this.ws !== ws) return;
      clearTimeout(this.connectTimer);
      this.state = 'open';
      this.attempts = 0;
      this.note = '';
      this.subscribe();
      this.flush();
      this.pool.changed();
    };
    ws.onmessage = (ev: { data: unknown }) => {
      if (this.ws === ws) this.message(ev.data);
    };
    ws.onclose = () => {
      if (this.ws === ws) this.lost(ws, 'connection closed');
    };
    ws.onerror = () => {
      if (this.ws === ws) this.lost(ws, 'connection failed');
    };
  }

  /** The connection is gone: put events without an answer back in the queue, and try again after a pause. */
  private lost(ws: WsLike | null, why: string) {
    clearTimeout(this.connectTimer);
    clearTimeout(this.resubTimer);
    if (ws) {
      ws.onopen = ws.onmessage = ws.onclose = ws.onerror = null;
      try {
        ws.close();
      } catch {
        // already closed
      }
    }
    if (this.ws === ws) this.ws = null;
    this.queue.unshift(...this.inflight.values());
    this.inflight.clear();
    if (this.stopped) return;
    this.state = 'waiting';
    this.note = why;
    const base = Math.min(this.pool.backoffMax, this.pool.backoffMin * 2 ** Math.min(this.attempts, 16));
    this.attempts++;
    this.reconnectTimer = setTimeout(() => this.connect(), base * (0.75 + Math.random() * 0.5));
    this.pool.changed();
  }

  /** Connect again now, when the page comes back to the front or the network returns. */
  kick() {
    if (this.stopped || this.ws) return;
    this.attempts = 0;
    this.connect();
  }

  private send(msg: unknown) {
    try {
      this.ws?.send(JSON.stringify(msg));
    } catch {
      // a send on a closing socket: its close handler takes over
    }
  }

  private subscribe() {
    this.send(['REQ', this.sub, this.pool.filter()]);
  }

  publish(ev: NostrEvent, ttlMs: number) {
    this.queue.push({ ev, until: Date.now() + ttlMs, rateTries: 0 });
    this.flush();
  }

  private flush() {
    if (this.state !== 'open' || !this.ws) return;
    const now = Date.now();
    if (now < this.holdUntil) {
      clearTimeout(this.flushTimer);
      this.flushTimer = setTimeout(() => this.flush(), this.holdUntil - now);
      return;
    }
    while (this.queue.length) {
      const item = this.queue.shift()!;
      if (item.until < now) {
        this.pool.result(item.ev.id, this.url, false, 'not sent in time');
        continue;
      }
      this.inflight.set(item.ev.id, item);
      this.send(['EVENT', item.ev]);
    }
  }

  private message(data: unknown) {
    const text = typeof data === 'string' ? data : String(data);
    if (text.length > MAX_FRAME) return;
    let m: unknown;
    try {
      m = JSON.parse(text);
    } catch {
      return;
    }
    if (!Array.isArray(m) || typeof m[0] !== 'string') return;
    const say = (s: unknown) => (typeof s === 'string' ? s.slice(0, 200) : '');
    switch (m[0]) {
      case 'EVENT':
        if (m[1] === this.sub) this.pool.incoming(m[2]);
        break;
      case 'OK': {
        const item = typeof m[1] === 'string' ? this.inflight.get(m[1]) : undefined;
        if (!item) break;
        this.inflight.delete(item.ev.id);
        const ok = m[2] === true;
        const msg = say(m[3]);
        if (!ok && /^rate-limited/i.test(msg) && item.rateTries < 3) {
          // Back off: nothing more to this relay for a while, then this event again.
          item.rateTries++;
          this.queue.unshift(item);
          this.holdUntil = Date.now() + this.rateBackoff;
          this.rateBackoff = Math.min(this.rateBackoff * 2, 60_000);
          this.note = 'rate-limited';
          this.pool.log(`${this.url}: ${msg}`);
          this.flush();
          this.pool.changed();
          break;
        }
        if (ok) this.rateBackoff = this.pool.rateBackoffMs;
        this.pool.result(item.ev.id, this.url, ok, msg);
        break;
      }
      case 'EOSE':
        break;
      case 'CLOSED':
        if (m[1] === this.sub) {
          // The relay ended the subscription: ask again after a pause.
          this.note = say(m[2]) || 'subscription closed';
          clearTimeout(this.resubTimer);
          const wait = /^rate-limited/i.test(this.note) ? this.rateBackoff : 15_000;
          this.resubTimer = setTimeout(() => {
            if (this.state === 'open') this.subscribe();
          }, wait);
          this.pool.changed();
        }
        break;
      case 'NOTICE':
        this.pool.log(`${this.url}: ${say(m[1])}`);
        break;
      default:
        // AUTH and anything else: not used by the link.
        break;
    }
  }

  stop() {
    this.stopped = true;
    clearTimeout(this.reconnectTimer);
    clearTimeout(this.connectTimer);
    clearTimeout(this.flushTimer);
    clearTimeout(this.resubTimer);
    for (const item of [...this.queue, ...this.inflight.values()]) this.pool.result(item.ev.id, this.url, false, 'stopped');
    this.queue = [];
    this.inflight.clear();
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      if (this.state === 'open') this.send(['CLOSE', this.sub]);
      ws.onopen = ws.onmessage = ws.onclose = ws.onerror = null;
      try {
        ws.close();
      } catch {
        // already closed
      }
    }
  }
}

interface Tally {
  accepted: number;
  refused: string[];
  waitingOn: Set<string>;
  done: (p: Published) => void;
  timer: ReturnType<typeof setTimeout>;
}

export class RelayPool {
  readonly wsf: WsFactory;
  readonly filter: () => Filter;
  readonly backoffMin: number;
  readonly backoffMax: number;
  readonly connectTimeoutMs: number;
  readonly rateBackoffMs: number;
  private relays = new Map<string, Relay>();
  private seen = new Set<string>();
  private tallies = new Map<string, Tally>();
  private running = false;

  constructor(private readonly opts: PoolOptions) {
    this.wsf = opts.ws ?? browserWs;
    this.filter = opts.filter;
    this.backoffMin = opts.backoffMin ?? 1000;
    this.backoffMax = opts.backoffMax ?? 60_000;
    this.connectTimeoutMs = opts.connectTimeoutMs ?? 10_000;
    this.rateBackoffMs = opts.rateBackoffMs ?? 2000;
    for (const url of opts.relays) this.relays.set(url, new Relay(url, this));
  }

  start() {
    this.running = true;
    for (const r of this.relays.values()) r.connect();
  }

  stop() {
    this.running = false;
    for (const r of this.relays.values()) r.stop();
    this.relays.clear();
    for (const t of [...this.tallies.values()]) {
      clearTimeout(t.timer);
      t.done({ accepted: t.accepted, refused: t.refused });
    }
    this.tallies.clear();
  }

  /** Follow a new relay list (the desktop's `status` carries it). */
  setRelays(urls: string[]) {
    for (const [url, r] of this.relays) {
      if (!urls.includes(url)) {
        r.stop();
        this.relays.delete(url);
      }
    }
    for (const url of urls) {
      if (!this.relays.has(url)) {
        const r = new Relay(url, this);
        this.relays.set(url, r);
        if (this.running) r.connect();
      }
    }
    this.changed();
  }

  urls(): string[] {
    return [...this.relays.keys()];
  }

  /** Reconnect every relay that is waiting, now. */
  kick() {
    for (const r of this.relays.values()) r.kick();
  }

  info(): RelayInfo[] {
    return [...this.relays.values()].map((r) => ({ url: r.url, state: r.state, note: r.note }));
  }

  openCount(): number {
    let n = 0;
    for (const r of this.relays.values()) if (r.state === 'open') n++;
    return n;
  }

  /**
   * Send an event to every relay (relays not connected get it when they connect, within `ttlMs`). Resolves once each
   * relay has answered, or after `waitMs`, with how many took it.
   */
  publish(ev: NostrEvent, o: { ttlMs?: number; waitMs?: number } = {}): Promise<Published> {
    const relays = [...this.relays.values()];
    return new Promise((resolve) => {
      const prev = this.tallies.get(ev.id);
      if (prev) {
        // The same event again (a resend): count afresh.
        clearTimeout(prev.timer);
        this.tallies.delete(ev.id);
        prev.done({ accepted: prev.accepted, refused: prev.refused });
      }
      const t: Tally = {
        accepted: 0,
        refused: [],
        waitingOn: new Set(relays.map((r) => r.url)),
        done: resolve,
        timer: setTimeout(() => this.settle(ev.id), o.waitMs ?? 8000),
      };
      this.tallies.set(ev.id, t);
      if (!relays.length) this.settle(ev.id);
      for (const r of relays) r.publish(ev, o.ttlMs ?? 60_000);
    });
  }

  private settle(id: string) {
    const t = this.tallies.get(id);
    if (!t) return;
    clearTimeout(t.timer);
    this.tallies.delete(id);
    t.done({ accepted: t.accepted, refused: t.refused });
  }

  /** A relay's answer to one of our events. */
  result(id: string, url: string, ok: boolean, msg: string) {
    const t = this.tallies.get(id);
    if (!t || !t.waitingOn.delete(url)) return;
    if (ok) t.accepted++;
    else t.refused.push(`${url}: ${msg}`);
    if (!t.waitingOn.size) this.settle(id);
  }

  /** An event a relay sent: checked, then deduplicated, then passed on. */
  incoming(raw: unknown) {
    if (!verifyEvent(raw)) {
      this.log('dropped an event with a bad id or signature');
      return;
    }
    if (this.seen.has(raw.id)) return;
    this.seen.add(raw.id);
    if (this.seen.size > SEEN_MAX) {
      const first = this.seen.values().next().value as string;
      this.seen.delete(first);
    }
    this.opts.onEvent(raw);
  }

  changed() {
    this.opts.onChange?.(this.info());
  }

  log(msg: string) {
    this.opts.log?.(msg);
  }
}
