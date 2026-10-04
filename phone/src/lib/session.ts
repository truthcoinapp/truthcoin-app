// The page's state once paired: the link to the desktop, what it last said, the trades in flight, and the line that
// always shows the last request. Screens read the stores and call the functions here.
import { get, writable } from 'svelte/store';
import { Api, failureText, type Tracker } from './api';
import { TradeFlows } from './flows';
import { BusyError, NoAnswerError, PhoneLink, ReplyError, UnsureError } from './link';
import { linkKeys, type Pairing } from './pairing';
import type { RelayInfo, WsFactory } from './relaypool';
import { forgetAll, idbKv, loadPairing, savePairing, type Kv } from './store';
import { cleanName } from './text';
import type { Balance, MarketsPage, Positions, Receive, Status, TradeRecord } from './validate';
import type { TradeFlow } from './flows';

/** `waiting`: still under way, with something to say (busy, or asking again). */
export type LastState = 'asking' | 'waiting' | 'answered' | 'held' | 'no-answer' | 'refused' | 'failed';

/** The last request and how it went, always on screen. */
export interface LastRequest {
  what: string;
  state: LastState;
  text: string;
  at: number;
  retry: (() => void) | null;
}

/** "Asking your computer …" for each request. */
const WHAT: Record<string, string> = {
  status: 'for its status',
  markets: 'for the markets',
  market: 'for the market',
  positions: 'for your positions',
  balance: 'for your balance',
  quote: 'for a price',
  'trade:buy': 'to buy',
  'trade:sell': 'to sell',
  trades: 'for recent trades',
  receive: 'for an address',
  unpair: 'to forget this phone',
};

// Other tabs or the installed app on the same storage: told when this one pairs or forgets, so they reload rather
// than keep writing under a pairing that's gone.
const channel: BroadcastChannel | null = (() => {
  try {
    return typeof BroadcastChannel === 'undefined' ? null : new BroadcastChannel('truthcoin-phone');
  } catch {
    return null;
  }
})();

export function announce(what: 'paired' | 'forgotten') {
  try {
    channel?.postMessage(what);
  } catch {
    // no other tabs to tell
  }
}

/** Called when another tab paired or forgot. */
export function onOtherTab(fn: () => void): () => void {
  if (!channel) return () => undefined;
  const h = () => fn();
  channel.addEventListener('message', h);
  return () => channel.removeEventListener('message', h);
}

export const pairing = writable<Pairing | null>(null);
export const relays = writable<RelayInfo[]>([]);
export const last = writable<LastRequest | null>(null);
export const status = writable<Status | null>(null);
export const balance = writable<Balance | null>(null);
export const positions = writable<Positions | null>(null);
export const trades = writable<TradeRecord[] | null>(null);
export const markets = writable<MarketsPage | null>(null);
export const address = writable<Receive | null>(null);
export const flows = writable<TradeFlow[]>([]);

let kv: Kv | null = null;
let link: PhoneLink | null = null;
let api: Api | null = null;
let tradeFlows: TradeFlows | null = null;
let unsubFlows: (() => void) | null = null;
/** Home is on screen (it asks for fresh data when shown, every 30 s, and when the page comes back to the front). */
let homeShown = false;
let homeBusy: Promise<void> | null = null;
/** When Home's data last came in (ms), and whether a trade settled since (then Home asks whatever its age). */
let homeAt = 0;
let homeDirty = false;
/** Showing Home again within this long reuses what it has: tab-flipping mustn't eat the desktop's request budget. */
export const HOME_FRESH_MS = 10_000;

export function storage(): Kv {
  kv ??= idbKv();
  return kv;
}

// Requests that overlap form one batch on the line: while any is asking, the line says so; when all have finished,
// the one that went worst speaks (a failure isn't hidden by a success that finished later).
interface Entry {
  what: string;
  state: LastState;
  text: string;
  retry: (() => void) | null;
}
const RANK: Record<LastState, number> = { asking: 0, waiting: 0, answered: 1, held: 2, refused: 3, failed: 4, 'no-answer': 5 };
const going = (s: LastState) => s === 'asking' || s === 'waiting';
let batch: Entry[] = [];

function render() {
  if (!batch.length) return;
  const asking = batch.filter((e) => going(e.state));
  const e = asking.length
    ? asking[asking.length - 1]
    : batch.reduce((a, b) => (RANK[b.state] >= RANK[a.state] ? b : a));
  // Several at once: "Asking your computer…" without naming one.
  const what = asking.length > 1 ? '' : e.what;
  last.set({ what, state: e.state, text: e.text, retry: e.retry, at: Date.now() });
}

const tracker: Tracker = (m, retry) => {
  const e: Entry = { what: WHAT[m] ?? '', state: 'asking', text: '', retry: null };
  if (!batch.some((x) => going(x.state))) batch = [];
  batch.push(e);
  render();
  const update = (s: LastState, text: string, r: (() => void) | null) => {
    e.state = s;
    e.text = text;
    e.retry = r;
    if (!batch.includes(e) && !batch.some((x) => going(x.state))) batch = [e];
    render();
  };
  return {
    ok: () => update('answered', '', null),
    held: (text) => update('held', text, null),
    busy: () => update('waiting', 'Your computer is busy; asking again in a moment.', null),
    again: () => update('waiting', "Your computer hasn't answered yet (it may be reconnecting). Asking again…", null),
    fail: (err) => {
      const s: LastState =
        err instanceof NoAnswerError || err instanceof UnsureError
          ? 'no-answer'
          : err instanceof BusyError
            ? 'failed'
            : err instanceof ReplyError
              ? 'refused'
              : 'failed';
      update(s, failureText(err), s === 'refused' ? null : retry);
    },
  };
};

/** Start talking to the paired desktop: ask for its status, and again about trades still waiting. */
export function startSession(p: Pairing, o: { ws?: WsFactory } = {}) {
  stopSession();
  pairing.set(p);
  let wasOpen = true;
  link = new PhoneLink(linkKeys(p), p.relays, {
    ws: o.ws,
    onRelays: (r) => {
      relays.set(r);
      // A relay back after none was open: ask again for whatever failed for want of one.
      const open = r.some((x) => x.state === 'open');
      if (open && !wasOpen) retryFailed();
      wasOpen = open;
    },
    onLateReply: (r) => void tradeFlows?.late(r),
  });
  api = new Api(link, tracker);
  tradeFlows = new TradeFlows(link, storage(), p.npub, tracker);
  // A trade that gets its final answer changes the balance and positions: ask for them again.
  const seen = new Map<string, string>();
  unsubFlows = tradeFlows.store.subscribe((list) => {
    flows.set(list);
    let settled = false;
    for (const f of list) {
      const before = seen.get(f.id);
      if (before && before !== f.state.k && (f.state.k === 'pending' || f.state.k === 'refused')) settled = true;
      seen.set(f.id, f.state.k);
    }
    if (settled) {
      if (homeShown) void refreshHome();
      else homeDirty = true; // Home asks when it's shown next
    }
  });
  link.start();
  relays.set(link.pool.info());
  void refreshStatus().catch(() => undefined);
  void tradeFlows.resume();
}

export function stopSession() {
  unsubFlows?.();
  unsubFlows = null;
  tradeFlows?.stop(); // first: nothing it does from here on is written
  link?.stop();
  link = null;
  api = null;
  tradeFlows = null;
  homeBusy = null;
  homeAt = 0;
  homeDirty = false;
  batch = [];
  status.set(null);
  balance.set(null);
  positions.set(null);
  trades.set(null);
  markets.set(null);
  address.set(null);
  last.set(null);
  flows.set([]);
  relays.set([]);
  pairing.set(null);
}

export function currentApi(): Api {
  if (!api) throw new Error('Not paired.');
  return api;
}

export function tradeFlowsNow(): TradeFlows {
  if (!tradeFlows) throw new Error('Not paired.');
  return tradeFlows;
}

/** The page came back to the front, or the network returned: reconnect, ask again, and ask for fresh data. */
export function kick() {
  if (!link) return;
  link.kick();
  tradeFlows?.askAgainAll();
  if (homeShown) void refreshHome({ maxAgeMs: HOME_FRESH_MS });
  else void refreshStatus().catch(() => undefined);
}

/** Home tells the session when it is on screen. */
export function homeOnScreen(shown: boolean) {
  homeShown = shown;
}

/** The last request that failed for want of an answer, asked again (a relay came back). */
function retryFailed() {
  const e = batch.find((x) => (x.state === 'no-answer' || x.state === 'failed') && x.retry);
  if (e && !batch.some((x) => going(x.state))) e.retry?.();
}

/** Ask for the desktop's status, and follow its relay list and this phone's name. On a new block, while trades are
 * on their way, the recent trades too (Home shows how they went). */
export async function refreshStatus(): Promise<Status> {
  const before = get(status)?.height ?? null;
  const s = await currentApi().status();
  status.set(s);
  const waiting = get(flows).some((f) => f.state.k === 'pending') || (get(trades) ?? []).some((t) => t.status === 'pending' || t.status === 'sending');
  if (homeShown && before !== null && s.height !== null && s.height !== before && waiting) {
    void currentApi()
      .trades()
      .then((t) => trades.set(t))
      .catch(() => undefined);
  }
  const p = get(pairing);
  if (p) {
    const changedRelays = s.relays && s.relays.join(' ') !== p.relays.join(' ');
    const name = cleanName(s.name);
    if (changedRelays || name !== p.name || s.limitSats !== p.limitSats) {
      const next: Pairing = { ...p, relays: s.relays ?? p.relays, name, limitSats: s.limitSats };
      pairing.set(next);
      if (changedRelays) link?.setRelays(next.relays);
      try {
        // Only over the same pairing: another tab may have paired again or forgotten it meanwhile.
        if ((await loadPairing(storage()))?.npub === next.npub) await savePairing(storage(), next);
      } catch {
        // kept for this visit only
      }
    }
  }
  return s;
}

/**
 * Home: status, balance and positions, and recent trades unless `trades` is false. While one refresh is under way,
 * another call waits for it rather than asking again.
 */
export function refreshHome(o: { trades?: boolean; maxAgeMs?: number } = {}): Promise<void> {
  if (!api) return Promise.resolve();
  if (homeBusy) return homeBusy;
  if (o.maxAgeMs && !homeDirty && Date.now() - homeAt < o.maxAgeMs) return Promise.resolve();
  const a = api;
  const asks: Promise<unknown>[] = [
    refreshStatus(),
    a.balance().then((b) => balance.set(b)),
    a.positions().then((p) => positions.set(p)),
  ];
  if (o.trades !== false) asks.push(a.trades().then((t) => trades.set(t)));
  const run: Promise<void> = Promise.allSettled(asks).then(() => {
    if (homeBusy === run) homeBusy = null;
    homeAt = Date.now();
    homeDirty = false;
  });
  homeBusy = run;
  return run;
}

/**
 * Forget this computer: ask the desktop to forget this phone too (best effort, a few seconds), stop, then delete the
 * page's whole database. `told` says whether the desktop's answer came back (without it, it has most likely still
 * forgotten the phone: it keeps its relays up a few seconds after `unpair` so the answer can go out). Rejects, with words for people, when the database
 * couldn't be deleted (the session is stopped either way).
 */
export async function forget(): Promise<{ told: boolean }> {
  let told = false;
  if (api) {
    try {
      await api.unpair();
      told = true;
    } catch {
      // the desktop may be closed: the person removes the phone there
    }
  }
  stopSession();
  await forgetAll(storage());
  announce('forgotten');
  return { told };
}
