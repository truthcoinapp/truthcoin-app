// Trades sent from this phone, from the tap to the desktop's final answer. Each trade's request (id, `ts`, args) is
// kept before it is first sent, so a trade without an answer (no reply, held on the desktop, or the page closed) can
// be asked about again under the same id, which is always safe: the desktop answers a repeat from its record and
// never trades twice. Never "Not sent" when it may have gone: "Not confirmed: check Positions before trying again".
import { writable, type Readable } from 'svelte/store';
import type { Tracker } from './api';
import { LinkStoppedError, NoAnswerError, ReplyError, UnsureError, type PhoneLink, type Reply, type Request } from './link';
import { loadPending, savePending, type Kv, type PendingTrade } from './store';
import { BadAnswerError, tradeDone, type Side } from './validate';

export type FlowState =
  /** Sent; no answer yet. */
  | { k: 'sending' }
  /** Over this phone's limit: waiting for its owner's yes on the computer. */
  | { k: 'held'; text: string }
  /** The computer passed it to the node: it goes through with the next Truthcoin block. */
  | { k: 'pending'; txid: string | null }
  /** The computer said no (`err`): nothing was traded. */
  | { k: 'refused'; msg: string }
  /** No answer, or the computer's `unsure`: it may or may not have gone. `why` says what went wrong. */
  | { k: 'unconfirmed'; why: string };

export interface TradeFlow {
  id: string;
  req: Request;
  title: string;
  label: string;
  side: Side;
  shares: number;
  limit: number;
  /** When it was made, ms. */
  at: number;
  state: FlowState;
}

export interface TradeArgs {
  marketId: string;
  outcome: number;
  shares: number;
  side: Side;
  /** At most this (buy, miner fee included), at least this (sell). */
  limit: number;
}

const OPEN = new Set(['sending', 'held', 'unconfirmed']);

export class TradeFlows {
  private list: TradeFlow[] = [];
  private readonly w = writable<TradeFlow[]>([]);
  readonly store: Readable<TradeFlow[]> = { subscribe: this.w.subscribe };

  constructor(
    private readonly link: PhoneLink,
    private readonly kv: Kv,
    private readonly track?: Tracker,
    private readonly timeoutMs = 30_000,
  ) {}

  all(): TradeFlow[] {
    return this.list;
  }

  get(id: string): TradeFlow | undefined {
    return this.list.find((f) => f.id === id);
  }

  /** Trades kept from before (the page was closed or reloaded): ask about each again, under its own id. */
  async resume(): Promise<void> {
    for (const p of await loadPending(this.kv)) {
      if (this.get(p.req.id)) continue;
      const a = p.req.a;
      this.list.push({
        id: p.req.id,
        req: p.req,
        title: p.title,
        label: p.label,
        side: a.side === 'sell' ? 'sell' : 'buy',
        shares: Number(a.shares) || 0,
        limit: Number(a.limit) || 0,
        at: p.at,
        state: p.state === 'held' ? { k: 'held', text: p.heldText } : { k: 'sending' },
      });
    }
    this.publish();
    await this.persist();
    for (const f of this.list) if (OPEN.has(f.state.k)) void this.run(f);
  }

  /** A new trade: kept first, then sent. */
  async start(t: TradeArgs, show: { title: string; label: string }): Promise<TradeFlow> {
    const req = this.link.makeRequest('trade', {
      id: t.marketId,
      outcome: t.outcome,
      shares: t.shares,
      side: t.side,
      limit: t.limit,
    });
    const f: TradeFlow = {
      id: req.id,
      req,
      title: show.title,
      label: show.label,
      side: t.side,
      shares: t.shares,
      limit: t.limit,
      at: Date.now(),
      state: { k: 'sending' },
    };
    this.list.unshift(f);
    this.publish();
    await this.persist();
    void this.run(f);
    return f;
  }

  /** Ask again about a trade with no final answer: the same request, the same id. */
  askAgain(id: string) {
    const f = this.get(id);
    if (!f || !OPEN.has(f.state.k)) return;
    if (f.state.k === 'unconfirmed') this.set(f, { k: 'sending' });
    void this.run(f);
  }

  /** Ask again about every trade that has no answer (the page came back to the front). */
  askAgainAll() {
    for (const f of this.list) if (f.state.k === 'unconfirmed') this.askAgain(f.id);
  }

  /** Take a finished trade off the list. */
  dismiss(id: string) {
    const f = this.get(id);
    if (!f || OPEN.has(f.state.k)) return;
    this.list = this.list.filter((x) => x !== f);
    this.publish();
  }

  /** A reply nobody was waiting for: maybe a late answer to one of these trades. */
  late(r: Reply): boolean {
    const f = this.get(r.re);
    if (!f) return false;
    if (r.k === 'held') this.set(f, { k: 'held', text: r.text });
    else if (r.k === 'unsure') this.set(f, { k: 'unconfirmed', why: r.text });
    else if (r.k === 'err') this.set(f, { k: 'refused', msg: r.err });
    else {
      try {
        this.set(f, { k: 'pending', txid: tradeDone(r.ok).txid });
      } catch (e) {
        this.set(f, { k: 'unconfirmed', why: (e as Error).message });
      }
    }
    return true;
  }

  private async run(f: TradeFlow) {
    const t = this.track?.('trade', () => this.askAgain(f.id));
    try {
      const r = await this.link.ask(f.req, {
        timeoutMs: this.timeoutMs,
        onHeld: (text) => {
          t?.held?.(text);
          this.set(f, { k: 'held', text });
        },
      });
      const done = tradeDone(r);
      t?.ok();
      this.set(f, { k: 'pending', txid: done.txid });
    } catch (e) {
      if (e instanceof LinkStoppedError) return; // the page is closing: what was kept stays as it was
      t?.fail(e);
      if (e instanceof ReplyError) this.set(f, { k: 'refused', msg: e.message });
      else if (e instanceof UnsureError) this.set(f, { k: 'unconfirmed', why: e.message });
      else if (e instanceof NoAnswerError)
        this.set(f, {
          k: 'unconfirmed',
          why: e.accepted ? 'No answer from your computer.' : "Couldn't reach any relay.",
        });
      else if (e instanceof BadAnswerError) this.set(f, { k: 'unconfirmed', why: e.message });
      else this.set(f, { k: 'unconfirmed', why: (e as Error)?.message || 'Something went wrong.' });
    }
  }

  private set(f: TradeFlow, s: FlowState) {
    // A final answer stands: a late repeat or a give-up timer can't turn it back.
    if (!OPEN.has(f.state.k)) return;
    f.state = s;
    this.publish();
    void this.persist();
  }

  private publish() {
    this.w.set([...this.list]);
  }

  private async persist() {
    const keep: PendingTrade[] = this.list
      .filter((f) => OPEN.has(f.state.k))
      .map((f) => ({
        req: f.req,
        title: f.title,
        label: f.label,
        state: f.state.k as PendingTrade['state'],
        heldText: f.state.k === 'held' ? f.state.text : '',
        at: f.at,
      }));
    try {
      await savePending(this.kv, keep);
    } catch {
      // Storage refused (private browsing): the trades still run; they just won't survive a reload.
    }
  }
}
