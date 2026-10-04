// The desktop's methods (PROTOCOL.md, "Methods"), each answer checked before anyone sees it.
import { BusyError, NoAnswerError, ReplyError, UnsureError, type PhoneLink, type RequestOptions } from './link';
import * as v from './validate';

/** What the page tells people about a request while it runs (the "last request" line). */
export interface Tracking {
  ok(): void;
  fail(e: unknown): void;
  held?(text: string): void;
  /** The desktop is busy; it's asked again by itself in a moment. */
  busy?(): void;
  /** No answer yet; asking again by itself. */
  again?(): void;
}
export type Tracker = (method: string, retry: () => void) => Tracking;

export interface QuoteArgs {
  id: string;
  outcome: number;
  shares: number;
  side: v.Side;
}

export class Api {
  constructor(
    readonly link: PhoneLink,
    private readonly track?: Tracker,
    /** How long before a read with no answer is asked once more by itself. */
    private readonly againAfterMs = 3000,
  ) {}

  private async call<T>(
    m: string,
    a: Record<string, unknown>,
    check: (x: unknown) => T,
    o?: RequestOptions,
    again?: Tracking,
  ): Promise<T> {
    const t = again ?? this.track?.(m, () => void this.call(m, a, check, o).catch(() => undefined));
    try {
      const r = check(await this.link.request(m, a, { ...o, onBusy: () => t?.busy?.() }));
      t?.ok();
      return r;
    } catch (e) {
      // A relay took it but no answer came (the computer may be reconnecting): ask once more by itself, afresh.
      if (!again && m !== 'unpair' && e instanceof NoAnswerError && e.accepted > 0) {
        t?.again?.();
        await new Promise((r) => setTimeout(r, this.againAfterMs));
        return this.call(m, a, check, o, t ?? { ok: () => undefined, fail: () => undefined });
      }
      t?.fail(e);
      throw e;
    }
  }

  status() {
    return this.call('status', {}, v.status);
  }
  markets(page: number) {
    return this.call('markets', { page }, v.marketsPage);
  }
  market(id: string) {
    return this.call('market', { id }, v.market);
  }
  positions() {
    return this.call('positions', {}, v.positions);
  }
  balance() {
    return this.call('balance', {}, v.balance);
  }
  quote(a: QuoteArgs) {
    return this.call('quote', { id: a.id, outcome: a.outcome, shares: a.shares, side: a.side }, (x) => v.quote(x, a.side));
  }
  trades() {
    return this.call('trades', {}, v.trades);
  }
  receive() {
    return this.call('receive', {}, v.receive);
  }
  /** Ask the desktop to forget this phone: best effort, a short wait. */
  unpair() {
    return this.call('unpair', {}, v.unpaired, { timeoutMs: 6000, resendAt: [2000, 4000] });
  }
}

/** Words for people about a failed request. */
export function failureText(e: unknown): string {
  if (e instanceof BusyError) return 'Your computer is busy. Ask again in a moment.';
  if (e instanceof ReplyError) return e.message;
  if (e instanceof UnsureError) return 'Not confirmed: check Positions before trying again.';
  if (e instanceof NoAnswerError) {
    return e.accepted
      ? 'No answer from your computer. Is the Truthcoin App open there?'
      : "Couldn't reach any relay (the relays or this phone's connection). Trying again…";
  }
  if (e instanceof v.BadAnswerError) return e.message;
  return (e as Error)?.message || 'Something went wrong.';
}
