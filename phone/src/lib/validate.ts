// Every answer from the desktop is checked before the page shows it (PROTOCOL.md, "Methods"): numbers must be finite
// and in range, ids and addresses in their form, text cleaned and capped, lists capped. An answer that fails is
// treated as no answer at all, never shown in part.
import { MAX_RELAYS, relayList, validRelay } from './relays';
import { cleanText } from './text';

/** What to do when the page and the desktop don't speak the same version. */
export const UPDATE_THE_APP = 'update the app (Settings › About › Check for a newer version)';

export class BadAnswerError extends Error {
  constructor(readonly what: string) {
    super(`This page and your Truthcoin App don't speak the same version: ${UPDATE_THE_APP}. (It sent a ${what} this page can't read.)`);
  }
}

/** 21 million coins in sats: no amount can be more. */
export const MAX_SATS = 2_100_000_000_000_000;

type O = Record<string, unknown>;

function obj(v: unknown, what: string): O {
  if (!v || typeof v !== 'object' || Array.isArray(v)) throw new BadAnswerError(what);
  return v as O;
}

/** A whole number from 0 to `max`. */
function int(v: unknown, what: string, max = MAX_SATS): number {
  if (typeof v !== 'number' || !Number.isSafeInteger(v) || v < 0 || v > max) throw new BadAnswerError(what);
  return v;
}

/** A finite number from `min` to `max` (amounts that may have a fraction: shares, values). */
function num(v: unknown, what: string, min = 0, max = MAX_SATS): number {
  if (typeof v !== 'number' || !Number.isFinite(v) || v < min || v > max) throw new BadAnswerError(what);
  return v;
}

/** A probability. */
const price = (v: unknown, what: string) => num(v, what, 0, 1);

function bool(v: unknown, what: string): boolean {
  if (typeof v !== 'boolean') throw new BadAnswerError(what);
  return v;
}

function str(v: unknown, what: string, max: number, multiline = false): string {
  if (typeof v !== 'string') throw new BadAnswerError(what);
  return cleanText(v, max, multiline);
}

function arr(v: unknown, what: string, max: number): unknown[] {
  if (!Array.isArray(v) || v.length > max) throw new BadAnswerError(what);
  return v;
}

function matching(v: unknown, what: string, re: RegExp): string {
  if (typeof v !== 'string' || !re.test(v)) throw new BadAnswerError(what);
  return v;
}

const MARKET_ID = /^[0-9A-Za-z_-]{1,64}$/;
const TXID = /^[0-9a-fA-F]{64}$/;
const ADDRESS = /^[0-9A-Za-z_]{8,128}$/;
const SHORT_WORD = /^[A-Za-z_]{1,24}$/;

/** A state as a word ("trading", "settled", "pending", …): letters only, lowercased. */
const state = (v: unknown, what: string) => matching(v, what, SHORT_WORD).toLowerCase();

// ---------- status ----------

export type NodeState = 'running' | 'starting' | 'stopped' | 'failed';
const NODE_STATES: NodeState[] = ['running', 'starting', 'stopped', 'failed'];

export interface Status {
  app: string;
  node: NodeState;
  /** null when the desktop can't tell (its node isn't running). */
  height: number | null;
  synced: boolean;
  network: string;
  name: string;
  limitSats: number;
  leftSats: number;
  /** The desktop's relays, when its list is one the page can use (else null: keep the current list). */
  relays: string[] | null;
}

export function status(v: unknown): Status {
  const o = obj(v, 'status');
  if (!NODE_STATES.includes(o.node as NodeState)) throw new BadAnswerError('node state');
  return {
    app: str(o.app, 'app version', 20),
    node: o.node as NodeState,
    height: o.height === null || o.height === undefined ? null : int(o.height, 'height', 1e9),
    synced: bool(o.synced, 'synced'),
    network: str(o.network, 'network', 30),
    name: str(o.name, 'phone name', 40),
    limitSats: int(o.limit_sats, 'limit'),
    leftSats: int(o.left_sats, 'limit left'),
    relays: followRelays(o.relays),
  };
}

/** The desktop's relays as the phone follows them: the usable ones, at most 5; null when none is usable. */
function followRelays(v: unknown): string[] | null {
  if (!Array.isArray(v)) return null;
  const ok = v.map((u) => validRelay(u)).filter((u): u is string => !!u);
  return relayList([...new Set(ok)].slice(0, MAX_RELAYS));
}

// ---------- markets ----------

export interface MarketSummary {
  id: string;
  title: string;
  state: string;
  outcomes: number;
  volume: number;
  created: number;
  /** The outcome with the highest chance, while trading (null otherwise, or from a desktop that doesn't say). */
  leading: { label: string; price: number } | null;
}

export interface MarketsPage {
  markets: MarketSummary[];
  page: number;
  pages: number;
}

export function marketsPage(v: unknown): MarketsPage {
  const o = obj(v, 'markets');
  const page = int(o.page, 'page', 1e6);
  const pages = int(o.pages, 'pages', 1e6);
  const markets = arr(o.markets, 'markets', 500).map((m, i) => {
    const x = obj(m, `market ${i}`);
    return {
      id: matching(x.id, 'market id', MARKET_ID),
      title: str(x.title, 'market title', 200),
      state: state(x.state, 'market state'),
      outcomes: int(x.outcomes, 'outcome count', 4096),
      volume: num(x.volume, 'volume'),
      created: int(x.created, 'created at', 1e9),
      leading: leadingOf(x.leading),
    };
  });
  return { markets, page, pages };
}

function leadingOf(v: unknown): MarketSummary['leading'] {
  if (v === null || v === undefined) return null;
  const l = obj(v, 'leading outcome');
  return { label: str(l.label, 'leading outcome', 120), price: price(l.price, 'leading chance') };
}

export interface Outcome {
  i: number;
  label: string;
  price: number;
  volume: number;
}

export interface Holding {
  outcome: number;
  shares: number;
  value: number;
}

export interface Market {
  id: string;
  title: string;
  description: string;
  state: string;
  feeRate: number;
  volume: number;
  outcomes: Outcome[];
  resolution: { summary: string; winners: number[] } | null;
  holdings: Holding[];
}

export function market(v: unknown): Market {
  const o = obj(v, 'market');
  const outcomes = arr(o.outcomes, 'outcomes', 4096).map((x, n) => {
    const p = obj(x, `outcome ${n}`);
    return {
      i: int(p.i, 'outcome index', 4095),
      label: str(p.label, 'outcome label', 120),
      price: price(p.price, 'chance'),
      volume: p.volume === null || p.volume === undefined ? 0 : num(p.volume, 'outcome volume'),
    };
  });
  const known = new Set(outcomes.map((x) => x.i));
  if (known.size !== outcomes.length) throw new BadAnswerError('outcome index repeated');
  let resolution: Market['resolution'] = null;
  if (o.resolution !== null && o.resolution !== undefined) {
    const r = obj(o.resolution, 'resolution');
    const winners = arr(r.winners, 'winners', 4096).map((w) => int(w, 'winner', 4095));
    if (!winners.every((w) => known.has(w))) throw new BadAnswerError('winner');
    resolution = { summary: str(r.summary, 'resolution', 500), winners };
  }
  const holdings = arr(o.holdings ?? [], 'holdings', 4096).map((x, n) => {
    const h = obj(x, `holding ${n}`);
    const outcome = int(h.outcome, 'holding outcome', 4095);
    if (!known.has(outcome)) throw new BadAnswerError('holding outcome');
    return { outcome, shares: num(h.shares, 'shares'), value: num(h.value, 'value') };
  });
  return {
    id: matching(o.id, 'market id', MARKET_ID),
    title: str(o.title, 'market title', 200),
    description: str(o.description ?? '', 'description', 4000, true),
    // The node's market record may not carry these two: '' and the outcomes' sum stand in.
    state: o.state === null || o.state === undefined ? '' : state(o.state, 'market state'),
    feeRate: num(o.fee_rate, 'trading fee', 0, 1),
    volume:
      o.volume === null || o.volume === undefined ? outcomes.reduce((n, x) => n + x.volume, 0) : num(o.volume, 'volume'),
    outcomes,
    resolution,
    holdings,
  };
}

// ---------- the wallet ----------

export interface Position {
  marketId: string;
  title: string;
  state: string;
  outcome: number;
  label: string;
  shares: number;
  price: number;
  value: number;
  paid: number | null;
}

export interface Positions {
  positions: Position[];
  totalValue: number;
}

export function positions(v: unknown): Positions {
  const o = obj(v, 'positions');
  const list = arr(o.positions, 'positions', 2000).map((x, n) => {
    const p = obj(x, `position ${n}`);
    return {
      marketId: matching(p.market_id, 'market id', MARKET_ID),
      title: str(p.title, 'market title', 200),
      state: state(p.state, 'market state'),
      outcome: int(p.outcome, 'outcome', 4095),
      label: str(p.label, 'outcome label', 120),
      shares: num(p.shares, 'shares'),
      price: price(p.price, 'chance'),
      value: num(p.value, 'value'),
      paid: p.paid === null || p.paid === undefined ? null : num(p.paid, 'paid'),
    };
  });
  return { positions: list, totalValue: num(o.total_value, 'total value') };
}

export interface Balance {
  /** What the wallet holds once what's moving settles (withdrawals left out). */
  total: number;
  available: number;
  inPendingTrades: number;
  pendingTrades: number;
  /** On its way to eCash (withdrawals): pays out in days. 0 from a desktop that doesn't say. */
  withdrawing: number;
}

export function balance(v: unknown): Balance {
  const o = obj(v, 'balance');
  return {
    total: int(o.total, 'total'),
    available: int(o.available, 'available'),
    inPendingTrades: int(o.in_pending_trades, 'in pending trades'),
    pendingTrades: int(o.pending_trades, 'pending trades', 1e6),
    withdrawing: o.withdrawing === null || o.withdrawing === undefined ? 0 : int(o.withdrawing, 'withdrawing'),
  };
}

export type Side = 'buy' | 'sell';

export interface Quote {
  side: Side;
  sats: number;
  fee: number;
  minerFee: number;
  priceNow: number;
  priceAfter: number;
  limit: number;
}

export function quote(v: unknown, side: Side): Quote {
  const o = obj(v, 'quote');
  if (o.side !== side) throw new BadAnswerError('side');
  const q = {
    side,
    sats: int(o.sats, 'price in sats'),
    fee: int(o.fee, 'trading fee'),
    minerFee: int(o.miner_fee, 'miner fee'),
    priceNow: price(o.price_now, 'chance now'),
    priceAfter: price(o.price_after, 'chance after'),
    limit: int(o.limit, 'limit'),
  };
  // A buy's cap covers the cost and the miner fee (paid from the wallet's coin); a sell's floor is at most what it
  // brings after the miner fee. A suggestion outside these would get the trade refused or stuck.
  if (side === 'buy' && q.limit < q.sats + q.minerFee) throw new BadAnswerError('limit below the price');
  if (side === 'sell' && q.limit > q.sats - q.minerFee) throw new BadAnswerError('limit above the proceeds');
  return q;
}

export interface TradeDone {
  status: 'pending';
  txid: string | null;
}

export function tradeDone(v: unknown): TradeDone {
  const o = obj(v, 'trade');
  if (o.status !== 'pending') throw new BadAnswerError('trade status');
  return { status: 'pending', txid: o.txid === null || o.txid === undefined ? null : matching(o.txid, 'txid', TXID) };
}

export interface TradeRecord {
  id: string;
  time: number;
  title: string;
  label: string;
  side: Side;
  shares: number;
  sats: number | null;
  limit: number;
  status: string;
  txid: string | null;
}

export function trades(v: unknown): TradeRecord[] {
  const o = obj(v, 'trades');
  return arr(o.trades, 'trades', 100).map((x, n) => {
    const t = obj(x, `trade ${n}`);
    if (t.side !== 'buy' && t.side !== 'sell') throw new BadAnswerError('side');
    return {
      id: matching(t.id, 'trade id', /^[0-9A-Za-z_-]{1,64}$/),
      time: int(t.time, 'time', 1e11),
      title: str(t.title, 'market title', 200),
      label: str(t.label, 'outcome label', 120),
      side: t.side,
      shares: num(t.shares, 'shares'),
      sats: t.sats === null || t.sats === undefined ? null : num(t.sats, 'sats'),
      limit: int(t.limit, 'limit'),
      status: state(t.status, 'trade status'),
      txid: t.txid === null || t.txid === undefined ? null : matching(t.txid, 'txid', TXID),
    };
  });
}

export interface Receive {
  address: string;
  depositAddress: string;
}

export function receive(v: unknown): Receive {
  const o = obj(v, 'receive');
  return {
    address: matching(o.address, 'address', ADDRESS),
    depositAddress: matching(o.deposit_address, 'deposit address', ADDRESS),
  };
}

/** The desktop's yes to `unpair`: it has forgotten this phone. */
export function unpaired(v: unknown): true {
  const o = obj(v, 'unpair');
  if (o.unpaired !== true) throw new BadAnswerError('unpair');
  return true;
}

export interface Paired {
  name: string;
  limitSats: number;
}

/** The desktop's yes to a pairing request. */
export function paired(v: unknown): Paired {
  const o = obj(v, 'pairing');
  if (o.paired !== true) throw new BadAnswerError('pairing');
  return { name: str(o.name, 'phone name', 40), limitSats: int(o.limit_sats, 'limit') };
}
