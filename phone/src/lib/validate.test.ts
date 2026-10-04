import { describe, expect, it } from 'vitest';
import * as v from './validate';

const goodStatus = {
  app: '0.1.0',
  node: 'running',
  height: 1234,
  synced: true,
  network: 'betanet',
  name: 'iPhone',
  limit_sats: 100000,
  left_sats: 50000,
  relays: ['wss://relay.damus.io', 'wss://nos.lol'],
};

const goodMarket = {
  id: 'a1b2c3d4e5f6',
  title: 'Will it rain?',
  description: 'Line one\nLine two',
  state: 'trading',
  fee_rate: 0.01,
  volume: 52250,
  outcomes: [
    { i: 0, label: 'No', price: 0.475, volume: 0 },
    { i: 1, label: 'Yes', price: 0.525, volume: 52250 },
  ],
  resolution: null,
  holdings: [{ outcome: 1, shares: 100000, value: 52500 }],
};

describe('answers from the desktop', () => {
  it('status: good values pass, with the relay list', () => {
    const s = v.status(goodStatus);
    expect(s).toMatchObject({ height: 1234, synced: true, limitSats: 100000, leftSats: 50000 });
    expect(s.relays).toEqual(goodStatus.relays);
  });

  it('status: a relay list the page cannot use is ignored, not followed', () => {
    expect(v.status({ ...goodStatus, relays: ['ws://evil.example'] }).relays).toBeNull();
    expect(v.status({ ...goodStatus, relays: [] }).relays).toBeNull();
  });

  it("status: no height when the computer's node isn't running; at most 5 usable relays followed", () => {
    expect(v.status({ ...goodStatus, node: 'stopped', height: null }).height).toBeNull();
    const many = ['wss://a.example', 'ws://bad.example', 'wss://b.example', 'wss://c.example', 'wss://d.example', 'wss://e.example', 'wss://f.example'];
    expect(v.status({ ...goodStatus, relays: many }).relays).toEqual(['wss://a.example', 'wss://b.example', 'wss://c.example', 'wss://d.example', 'wss://e.example']);
  });

  it("market: a missing state or volume (the node's record may lack them) doesn't sink the answer", () => {
    const m = v.market({ ...goodMarket, state: null, volume: null, outcomes: [{ i: 0, label: 'No', price: 0.5, volume: 10 }, { i: 1, label: 'Yes', price: 0.5, volume: null }] });
    expect(m.state).toBe('');
    expect(m.volume).toBe(10);
  });

  it('numbers must be finite, whole where counted, and in range', () => {
    for (const bad of [-1, 1.5, NaN, Infinity, '5', null, 3e15]) {
      expect(() => v.status({ ...goodStatus, limit_sats: bad })).toThrow(v.BadAnswerError);
    }
    expect(() => v.status({ ...goodStatus, node: 'exploded' })).toThrow(v.BadAnswerError);
    expect(() => v.status({ ...goodStatus, synced: 'yes' })).toThrow(v.BadAnswerError);
  });

  it('market: chances are probabilities, indexes are known, winners are outcomes', () => {
    const m = v.market(goodMarket);
    expect(m.outcomes[1]).toEqual({ i: 1, label: 'Yes', price: 0.525, volume: 52250 });
    expect(m.description).toBe('Line one\nLine two');
    expect(() => v.market({ ...goodMarket, outcomes: [{ i: 0, label: 'x', price: 1.2, volume: 0 }] })).toThrow();
    expect(() => v.market({ ...goodMarket, outcomes: [{ i: 0, label: 'x', price: -0.1, volume: 0 }] })).toThrow();
    expect(() => v.market({ ...goodMarket, fee_rate: 2 })).toThrow();
    expect(() => v.market({ ...goodMarket, resolution: { summary: 'Yes', winners: [7] } })).toThrow();
    expect(() => v.market({ ...goodMarket, holdings: [{ outcome: 9, shares: 1, value: 1 }] })).toThrow();
    expect(() => v.market({ ...goodMarket, id: '../etc' })).toThrow();
    const settled = v.market({ ...goodMarket, state: 'Settled', resolution: { summary: 'Yes', winners: [1] } });
    expect(settled.state).toBe('settled');
    expect(settled.resolution).toEqual({ summary: 'Yes', winners: [1] });
  });

  it('market: how each question is decided and when; positions: what settled markets paid', () => {
    const m = v.market({
      ...goodMarket,
      decisions: [{ question: 'Rain?', rules: 'IPMA records.\nNothing else.', period: 3 }],
      current_period: 1,
      blocks_per_period: null,
    });
    expect(m.decisions).toEqual([{ question: 'Rain?', rules: 'IPMA records.\nNothing else.', period: 3 }]);
    expect(m.currentPeriod).toBe(1);
    expect(m.blocksPerPeriod).toBeNull();
    expect(v.market(goodMarket).decisions).toEqual([]); // a desktop that doesn't say
    expect(() => v.market({ ...goodMarket, decisions: [{ question: 'Q', rules: '', period: -1 }] })).toThrow();
    const p = v.positions({
      positions: [],
      total_value: 0,
      settled: [{ market_id: 'b2c3d4e5f6a1', title: 'T', winners: ['Yes'], paid: 50000, outcomes: [{ label: 'Yes', shares: 50000, per_share: 1 }] }],
    });
    expect(p.settled[0]).toEqual({ marketId: 'b2c3d4e5f6a1', title: 'T', winners: ['Yes'], paid: 50000, outcomes: [{ label: 'Yes', shares: 50000, perShare: 1 }] });
    expect(v.positions({ positions: [], total_value: 0 }).settled).toEqual([]);
    expect(() =>
      v.positions({ positions: [], total_value: 0, settled: [{ market_id: 'x', title: 'T', winners: [], paid: 0, outcomes: [{ label: 'Y', shares: 1, per_share: 2 }] }] }),
    ).toThrow();
  });

  it('markets: a page of summaries', () => {
    const p = v.marketsPage({
      markets: [
        { id: 'a1b2c3d4e5f6', title: 'T', state: 'trading', outcomes: 2, volume: 0, created: 10, leading: { label: 'Yes', price: 0.63 } },
        { id: 'b1b2c3d4e5f6', title: 'S', state: 'settled', outcomes: 2, volume: 0, created: 9, leading: null },
        { id: 'c1b2c3d4e5f6', title: 'Old', state: 'trading', outcomes: 2, volume: 0, created: 8 },
      ],
      page: 0,
      pages: 1,
    });
    expect(p.markets[0].title).toBe('T');
    expect(p.markets[0].leading).toEqual({ label: 'Yes', price: 0.63 });
    expect(p.markets[1].leading).toBeNull();
    expect(p.markets[2].leading).toBeNull(); // a desktop that doesn't say
    expect(() =>
      v.marketsPage({ markets: [{ id: 'a1', title: 'T', state: 'trading', outcomes: 2, volume: 0, created: 1, leading: { label: 'Yes', price: 1.5 } }], page: 0, pages: 1 }),
    ).toThrow();
    expect(() => v.marketsPage({ markets: 'x', page: 0, pages: 1 })).toThrow();
  });

  it('quote: the side must match, and the cap must cover the price', () => {
    const q = { side: 'buy', sats: 52250, fee: 1000, miner_fee: 1000, price_now: 0.5, price_after: 0.525, limit: 55000 };
    expect(v.quote(q, 'buy')).toMatchObject({ sats: 52250, minerFee: 1000, limit: 55000 });
    expect(() => v.quote(q, 'sell')).toThrow();
    expect(() => v.quote({ ...q, limit: 50000 }, 'buy')).toThrow();
    expect(() => v.quote({ ...q, limit: 53000 }, 'buy')).toThrow(); // covers the price but not the miner fee
    const s = { ...q, side: 'sell', sats: 24937, limit: 23000 };
    expect(v.quote(s, 'sell').limit).toBe(23000);
    expect(() => v.quote({ ...s, limit: 30000 }, 'sell')).toThrow();
    expect(() => v.quote({ ...s, limit: 24500 }, 'sell')).toThrow(); // above the proceeds less the miner fee
  });

  it('trade, trades, receive, balance, positions', () => {
    const txid = 'ab'.repeat(32);
    expect(v.tradeDone({ status: 'pending', txid })).toEqual({ status: 'pending', txid });
    expect(() => v.tradeDone({ status: 'pending', txid: 'zz' })).toThrow();
    expect(v.tradeDone({ status: 'pending', txid: null }).txid).toBeNull();
    expect(() => v.tradeDone({ status: 'done', txid })).toThrow();
    expect(
      v.trades({
        trades: [
          { id: 'x1', time: 1791200000, title: 'T', label: 'Yes', side: 'buy', shares: 10, sats: null, limit: 20, status: 'held', txid: null },
        ],
      })[0],
    ).toMatchObject({ sats: null, txid: null, status: 'held' });
    expect(v.receive({ address: '1AbCdEfGhJkLmN', deposit_address: 's13_1AbCdEfGhJkLmN_abc123' }).depositAddress).toBe(
      's13_1AbCdEfGhJkLmN_abc123',
    );
    expect(() => v.receive({ address: '<img src=x>', deposit_address: 'x' })).toThrow();
    expect(v.balance({ total: 10, available: 5, in_pending_trades: 5, pending_trades: 1 })).toEqual({
      total: 10,
      available: 5,
      inPendingTrades: 5,
      pendingTrades: 1,
      withdrawing: 0,
    });
    expect(v.balance({ total: 10, available: 5, in_pending_trades: 5, pending_trades: 1, withdrawing: 7 }).withdrawing).toBe(7);
    expect(() => v.balance({ total: 10, available: 5, in_pending_trades: 5, pending_trades: 1, withdrawing: -1 })).toThrow(
      /same version/,
    );
    const pos = v.positions({
      positions: [
        { market_id: 'a1b2c3d4e5f6', title: 'T', state: 'trading', outcome: 1, label: 'Yes', shares: 5, price: 0.5, value: 2.5, paid: null },
      ],
      total_value: 2.5,
    });
    expect(pos.positions[0].paid).toBeNull();
    expect(() => v.positions({ positions: [], total_value: -1 })).toThrow();
  });

  it('pairing: only an explicit yes', () => {
    expect(v.paired({ paired: true, name: 'iPhone', limit_sats: 5 })).toEqual({ name: 'iPhone', limitSats: 5 });
    expect(() => v.paired({ paired: 'true', name: 'x', limit_sats: 5 })).toThrow();
  });
});
