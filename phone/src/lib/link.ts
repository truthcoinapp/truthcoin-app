// Requests and replies (PROTOCOL.md, "Requests and replies"): each request is sealed from P to D on its own and sent
// to every relay; the desktop answers under the request's id, sealed from D to P. Relays may drop, repeat, reorder or
// delay anything, so a request without an answer is sent again (sealed afresh, same id, same `ts`, same args): the
// desktop answers a repeat from its record and never runs it twice. Keeping the first `ts` means a request can only
// ever start within the desktop's 5-minute window after it was made; later repeats can only fetch its answer.
import { fromB64u, isRequestId, newRequestId, type Bytes } from './bytes';
import { envelopeJson, openMsg, padJson, parseEnvelope, sealMsg, unpadJson } from './crypto';
import { fromTo, KIND, messageEvent, tag, type NostrEvent } from './nostr';
import { RelayPool, type Published, type RelayInfo, type WsFactory } from './relaypool';
import { cleanText } from './text';

/** What the phone holds to talk to its desktop. */
export interface LinkKeys {
  /** The phone's device key P (non-extractable) and its public bytes. */
  pPriv: CryptoKey;
  pPub: Bytes;
  /** The desktop's static key D. */
  dPub: Bytes;
  /** The phone's Nostr secret nP and its x-only public key. */
  nsec: Bytes;
  npub: string;
  /** The desktop's Nostr key nD. */
  nd: string;
}

export interface Request {
  id: string;
  ts: number;
  m: string;
  a: Record<string, unknown>;
}

export type Reply =
  | { re: string; k: 'ok'; ok: unknown }
  /** `busy`: the desktop was over its request limit and didn't run it (asking again, same id, is fine). */
  | { re: string; k: 'err'; err: string; busy?: boolean }
  | { re: string; k: 'held'; text: string }
  | { re: string; k: 'unsure'; text: string }
  /** Pairing only: the desktop's 16-byte commitment nonce N for the comparison code. */
  | { re: string; k: 'nonce'; nonce: Bytes };

/** The link was stopped (forgetting the computer) while the request waited: nothing is known about it. */
export class LinkStoppedError extends Error {
  constructor() {
    super('Stopped.');
  }
}

/** The desktop answered no (or couldn't): the request did not happen. */
export class ReplyError extends Error {}

/**
 * The desktop doesn't know whether the trade reached the node (`unsure`): it may have gone through. Not final: the
 * id stays open, so asking again (same id) or a later answer can settle it.
 */
export class UnsureError extends Error {}

/** The desktop was over its request limit and said so, again after the phone's own retries. Nothing was run. */
export class BusyError extends ReplyError {}

/** The desktop's answer when it's over its request limit (`"busy": true`, or from a desktop before that flag, these
 * words): not stored for the id, so asking again is fine. */
const BUSY = /^your computer is busy/i;
/** How often, and after how long, a busy answer is asked again by itself (same id). */
const BUSY_RETRIES = 2;
const BUSY_WAIT_MS = 5_000;

/** No answer came in time: the request may or may not have happened. Ask again with the same id. */
export class NoAnswerError extends Error {
  constructor(readonly accepted: number) {
    super(accepted ? 'No answer from your computer.' : 'No relay took the request.');
  }
}

const UNSURE = 'Not confirmed: check Positions before trying again';

/** A reply as the desktop sends it, or null for anything else. */
export function parseReply(v: unknown): Reply | null {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return null;
  const o = v as Record<string, unknown>;
  if (!isRequestId(o.re)) return null;
  const has = ['ok', 'err', 'held', 'unsure', 'nonce'].filter((k) => k in o);
  if (has.length !== 1) return null;
  if (has[0] === 'ok') return { re: o.re, k: 'ok', ok: o.ok };
  if (has[0] === 'err') {
    if (typeof o.err !== 'string') return null;
    const err = cleanText(o.err, 300) || 'Your computer said no.';
    return o.busy === true ? { re: o.re, k: 'err', err, busy: true } : { re: o.re, k: 'err', err };
  }
  if (has[0] === 'unsure') {
    if (typeof o.unsure !== 'string') return null;
    return { re: o.re, k: 'unsure', text: cleanText(o.unsure, 300) || UNSURE };
  }
  if (has[0] === 'nonce') {
    if (typeof o.nonce !== 'string') return null;
    try {
      const n = fromB64u(o.nonce);
      return n.length === 16 ? { re: o.re, k: 'nonce', nonce: n } : null;
    } catch {
      return null;
    }
  }
  const h = o.held as Record<string, unknown> | null;
  if (!h || typeof h !== 'object' || typeof h.text !== 'string') return null;
  return { re: o.re, k: 'held', text: cleanText(h.text, 300) };
}

export interface RequestOptions {
  /** Ask again under an earlier id (and its first `ts`): always safe. */
  id?: string;
  ts?: number;
  /** The desktop holds the request for its owner's yes. Called once per distinct text. */
  onHeld?: (text: string) => void;
  /** After the first send: how many relays took it. */
  onSent?: (p: Published) => void;
  /** Give up (NoAnswerError) after this long without any answer. A held request waits for its final answer. */
  timeoutMs?: number;
  /**
   * Send again (sealed afresh) at these times after the first send, while no answer has come, and only if nothing at
   * all came from the desktop since: a desktop answering other requests is alive, and this one is just slow.
   */
  resendAt?: number[];
  /** While held, ask again this often (the final answer may have been missed). */
  heldPollMs?: number;
  /** The desktop said it's busy; the request is asked again by itself in a few seconds. */
  onBusy?: () => void;
}

interface Listener {
  o: RequestOptions;
  resolve: (v: unknown) => void;
  reject: (e: Error) => void;
}

interface Waiter {
  req: Request;
  held: string | null;
  /** When it was last sent, ms. */
  sentAt: number;
  busy: number;
  listeners: Listener[];
  timers: ReturnType<typeof setTimeout>[];
  poll?: ReturnType<typeof setInterval>;
}

export interface LinkOptions {
  ws?: WsFactory;
  now?: () => number;
  /** A reply under an id nobody is waiting for (a late answer, or one to a request from before a reload). */
  onLateReply?: (r: Reply) => void;
  onRelays?: (relays: RelayInfo[]) => void;
  log?: (msg: string) => void;
  backoffMin?: number;
  backoffMax?: number;
  /** How long a busy answer waits before it's asked again (5 s). */
  busyWaitMs?: number;
  /** Defaults for requests. */
  timeoutMs?: number;
  resendAt?: number[];
  heldPollMs?: number;
}

const DONE_MAX = 512;

export class PhoneLink {
  readonly pool: RelayPool;
  private waiting = new Map<string, Waiter>();
  private done = new Set<string>();
  private now: () => number;

  constructor(
    private readonly keys: LinkKeys,
    relays: string[],
    private readonly opts: LinkOptions = {},
  ) {
    this.now = opts.now ?? (() => Math.floor(Date.now() / 1000));
    this.pool = new RelayPool({
      relays,
      ws: opts.ws,
      filter: () => ({ kinds: [KIND], '#p': [keys.npub], since: this.now() - 120 }),
      accept: fromTo(keys.nd, keys.npub),
      onEvent: (e) => void this.onEvent(e),
      onChange: opts.onRelays,
      log: opts.log,
      backoffMin: opts.backoffMin,
      // A relay that was down is tried again at least every 15 s, so the link comes back soon after it does.
      backoffMax: opts.backoffMax ?? 15_000,
    });
  }

  start() {
    this.pool.start();
  }

  stop() {
    this.pool.stop();
    for (const w of [...this.waiting.values()]) this.finish(w, new LinkStoppedError());
  }

  /** Back in front (or online again): reconnect now, and ask again about everything still waiting. */
  kick() {
    this.pool.kick();
    for (const w of this.waiting.values()) void this.send(w.req);
  }

  setRelays(urls: string[]) {
    this.pool.setRelays(urls);
  }

  /** A new request: its id and `ts` are made here unless given (asking again). */
  makeRequest(m: string, a: Record<string, unknown> = {}, id?: string, ts?: number): Request {
    return { id: id ?? newRequestId(), ts: ts ?? this.now(), m, a };
  }

  /**
   * Send a request and wait for its final answer: resolves with the result, rejects with ReplyError (the desktop said
   * no) or NoAnswerError (nothing came in time; it may have happened).
   */
  request(m: string, a: Record<string, unknown> = {}, o: RequestOptions = {}): Promise<unknown> {
    return this.ask(this.makeRequest(m, a, o.id, o.ts), o);
  }

  /**
   * Send a request made earlier (with `makeRequest`, maybe kept across a reload) and wait for its answer. Asking
   * about an id that is already waiting sends it again and waits on the same answer.
   */
  ask(req: Request, o: RequestOptions = {}): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const listener: Listener = { o, resolve, reject };
      const w = this.waiting.get(req.id);
      if (w) {
        w.listeners.push(listener);
        if (w.held !== null) o.onHeld?.(w.held);
        else this.arm(w, o);
        void this.send(w.req).then((p) => o.onSent?.(p));
        return;
      }
      this.done.delete(req.id); // asking again after a final answer: the desktop's repeat of it is wanted
      const fresh: Waiter = { req, held: null, sentAt: Date.now(), busy: 0, listeners: [listener], timers: [] };
      this.waiting.set(req.id, fresh);
      this.arm(fresh, o);
      void this.send(req).then((p) => o.onSent?.(p));
    });
  }

  /** (Re)start the resend and give-up timers of a request that has had no answer. */
  private arm(w: Waiter, o: RequestOptions) {
    for (const t of w.timers) clearTimeout(t);
    w.timers = [];
    const timeout = o.timeoutMs ?? this.opts.timeoutMs ?? 25_000;
    for (const at of o.resendAt ?? this.opts.resendAt ?? [15_000]) {
      if (at < timeout) {
        w.timers.push(
          setTimeout(() => {
            if (w.held === null && this.lastHeard < w.sentAt) void this.send(w.req);
          }, at),
        );
      }
    }
    w.timers.push(
      setTimeout(() => w.held === null && this.finish(w, new NoAnswerError(this.pool.openCount())), timeout),
    );
  }

  /** When the desktop was last heard from (a reply that opened and wasn't a repeat), ms. */
  private lastHeard = 0;

  /** Seal the request afresh and publish it to every relay. */
  private async send(req: Request): Promise<Published> {
    const w = this.waiting.get(req.id);
    if (w) w.sentAt = Date.now();
    const env = await sealMsg({ sPriv: this.keys.pPriv, sPub: this.keys.pPub, rPub: this.keys.dPub }, padJson(req));
    const ev = messageEvent(this.keys.nsec, this.keys.nd, envelopeJson(env), this.now());
    return this.pool.publish(ev);
  }

  private finish(w: Waiter, outcome: Error | { ok: unknown }) {
    for (const t of w.timers) clearTimeout(t);
    clearInterval(w.poll);
    if (this.waiting.get(w.req.id) === w) this.waiting.delete(w.req.id);
    for (const l of w.listeners) {
      if (outcome instanceof Error) l.reject(outcome);
      else l.resolve(outcome.ok);
    }
  }

  private remember(id: string) {
    this.done.add(id);
    if (this.done.size > DONE_MAX) this.done.delete(this.done.values().next().value as string);
  }

  /** An event from a relay, already checked (id, signature) and deduplicated by the pool. */
  private async onEvent(ev: NostrEvent) {
    if (ev.pubkey !== this.keys.nd || ev.kind !== KIND || tag(ev, 'p') !== this.keys.npub) return;
    const env = parseEnvelope(ev.content);
    if (!env || env.k !== 'msg') return;
    let reply: Reply | null;
    try {
      const pt = await openMsg({ rPriv: this.keys.pPriv, rPub: this.keys.pPub, sPub: this.keys.dPub }, env);
      reply = parseReply(unpadJson(pt));
    } catch {
      return; // doesn't open: dropped without an answer
    }
    if (reply) this.dispatch(reply);
  }

  /** A reply that opened: to its waiting request, unless that id already had its final answer (a repeat). */
  dispatch(r: Reply) {
    if (r.k === 'nonce') return; // pairing's, not a request's
    if (this.done.has(r.re)) return;
    const w = this.waiting.get(r.re);
    // `held` and `unsure` leave the id open: a final answer may still follow.
    const final = r.k === 'ok' || r.k === 'err';
    this.lastHeard = Date.now();
    if (!w) {
      if (final) this.remember(r.re);
      this.opts.onLateReply?.(r);
      return;
    }
    if (r.k === 'err' && (r.busy || BUSY.test(r.err))) {
      // Over the desktop's limit: nothing ran, and the answer isn't kept for the id. Ask again (same id) in a moment.
      if (w.busy < BUSY_RETRIES) {
        w.busy++;
        for (const t of w.timers) clearTimeout(t);
        w.timers = [
          setTimeout(() => {
            if (this.waiting.get(w.req.id) !== w) return;
            void this.send(w.req);
            this.arm(w, w.listeners[0]?.o ?? {});
          }, this.opts.busyWaitMs ?? BUSY_WAIT_MS),
        ];
        for (const l of w.listeners) l.o.onBusy?.();
        return;
      }
      this.remember(r.re);
      this.finish(w, new BusyError(r.err));
      return;
    }
    if (r.k === 'unsure') {
      this.finish(w, new UnsureError(r.text));
      return;
    }
    if (r.k === 'held') {
      if (w.held === r.text) return;
      const first = w.held === null;
      w.held = r.text;
      if (first) {
        // No more giving up: a held request waits for the owner. Ask again now and then, in case the final answer
        // went past while this page wasn't listening.
        for (const t of w.timers) clearTimeout(t);
        w.timers = [];
        const every = w.listeners[0]?.o.heldPollMs ?? this.opts.heldPollMs ?? 60_000;
        w.poll = setInterval(() => void this.send(w.req), every);
      }
      for (const l of w.listeners) l.o.onHeld?.(r.text);
      return;
    }
    this.remember(r.re);
    this.finish(w, r.k === 'ok' ? { ok: r.ok } : new ReplyError(r.err));
  }

  /** Is anything still waiting for an answer? */
  busy(): boolean {
    return this.waiting.size > 0;
  }
}
