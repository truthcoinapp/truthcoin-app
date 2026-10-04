import { describe, expect, it } from 'vitest';
import { messageEvent, newNostrSecret, type NostrEvent } from './nostr';
import { RelayPool } from './relaypool';
import { fakeWsFactory, tick, waitFor } from './testing/fakews';

const A = 'wss://a.example';
const B = 'wss://b.example';
const me = 'ab'.repeat(32);

function setup(relays = [A, B], extra: Partial<ConstructorParameters<typeof RelayPool>[0]> = {}) {
  const f = fakeWsFactory();
  const got: NostrEvent[] = [];
  const pool = new RelayPool({
    relays,
    ws: f.ws,
    filter: () => ({ kinds: [21913], '#p': [me], since: 1000 }),
    onEvent: (e) => got.push(e),
    backoffMin: 20,
    backoffMax: 40,
    rateBackoffMs: 400,
    ...extra,
  });
  pool.start();
  return { pool, f, got };
}

const subOf = (s: { of: (t: string) => unknown[][] }) => s.of('REQ')[0][1] as string;

describe('the relay pool', () => {
  it('subscribes on every relay with the filter', () => {
    const { f } = setup();
    for (const url of [A, B]) {
      const s = f.last(url);
      s.open();
      const req = s.of('REQ')[0];
      expect(req[2]).toEqual({ kinds: [21913], '#p': [me], since: 1000 });
    }
  });

  it('passes each good event on once, whichever relays repeat it and in whatever order', () => {
    const { f, got } = setup();
    const a = f.last(A);
    const b = f.last(B);
    a.open();
    b.open();
    const sk = newNostrSecret();
    const e1 = messageEvent(sk, me, 'one', 1000);
    const e2 = messageEvent(sk, me, 'two', 1001);
    const e3 = messageEvent(sk, me, 'three', 1002);
    for (const [s, e] of [[a, e2], [b, e1], [a, e1], [a, e2], [b, e3], [b, e2], [a, e3], [a, e1]] as const) {
      s.push(['EVENT', subOf(s), e]);
    }
    expect(got.map((e) => e.content)).toEqual(['two', 'one', 'three']);
  });

  it('drops events with a bad id or signature before deduplicating (a forgery cannot block the real one)', () => {
    const { f, got } = setup([A]);
    const a = f.last(A);
    a.open();
    const sk = newNostrSecret();
    const real = messageEvent(sk, me, 'real', 1000);
    const forged = { ...real, content: 'forged' }; // same id, other content
    const badSig = { ...real, sig: real.sig.slice(0, -1) + (real.sig.endsWith('0') ? '1' : '0') };
    a.push(['EVENT', subOf(a), forged]);
    a.push(['EVENT', subOf(a), badSig]);
    expect(got).toEqual([]);
    a.push(['EVENT', subOf(a), real]);
    expect(got.map((e) => e.content)).toEqual(['real']);
  });

  it('ignores events for other subscriptions and junk frames', () => {
    const { f, got } = setup([A]);
    const a = f.last(A);
    a.open();
    a.push(['EVENT', 'someone-else', messageEvent(newNostrSecret(), me, 'x', 1000)]);
    a.onmessage?.({ data: 'not json' });
    a.push({ not: 'an array' });
    a.push(['NOTICE', 'hello']);
    a.push(['EOSE', subOf(a)]);
    expect(got).toEqual([]);
  });

  it('publishes to every relay, queueing for one not yet connected, and counts the OKs', async () => {
    const { pool, f } = setup();
    const a = f.last(A);
    a.open();
    const ev = messageEvent(newNostrSecret(), me, 'hi', 1000);
    const done = pool.publish(ev, { waitMs: 2000 });
    expect(a.of('EVENT')).toEqual([['EVENT', ev]]);
    const b = f.last(B);
    expect(b.of('EVENT')).toEqual([]);
    b.open();
    expect(b.of('EVENT')).toEqual([['EVENT', ev]]);
    a.push(['OK', ev.id, true, '']);
    b.push(['OK', ev.id, false, 'blocked: no']);
    expect(await done).toEqual({ accepted: 1, refused: [`${B}: blocked: no`] });
  });

  it('backs off on "rate-limited", then sends the event again', async () => {
    const { pool, f } = setup([A]);
    const a = f.last(A);
    a.open();
    const ev = messageEvent(newNostrSecret(), me, 'hi', 1000);
    const done = pool.publish(ev, { waitMs: 3000 });
    a.push(['OK', ev.id, false, 'rate-limited: slow down']);
    expect(a.of('EVENT').length).toBe(1);
    expect(pool.info()[0].note).toBe('rate-limited');
    await tick(20);
    expect(a.of('EVENT').length).toBe(1); // still holding back
    await waitFor(() => a.of('EVENT').length === 2, 3000);
    a.push(['OK', ev.id, true, '']);
    expect(await done).toEqual({ accepted: 1, refused: [] });
  });

  it('reconnects after a dropped connection, subscribes again, and re-sends what had no answer', async () => {
    const { pool, f } = setup([A]);
    const a1 = f.last(A);
    a1.open();
    const ev = messageEvent(newNostrSecret(), me, 'hi', 1000);
    void pool.publish(ev, { waitMs: 3000 });
    a1.drop();
    expect(pool.info()[0].state).toBe('waiting');
    await waitFor(() => f.last(A) !== a1, 3000);
    const a2 = f.last(A);
    expect(a2).not.toBe(a1);
    a2.open();
    expect(a2.of('REQ').length).toBe(1);
    expect(a2.of('EVENT')).toEqual([['EVENT', ev]]);
    expect(pool.openCount()).toBe(1);
  });

  it('resubscribes after the relay closes the subscription', async () => {
    const { f } = setup([A]);
    const a = f.last(A);
    a.open();
    a.push(['CLOSED', subOf(a), 'rate-limited: too many']);
    await waitFor(() => a.of('REQ').length === 2, 3000);
  });

  it('follows a new relay list', () => {
    const { pool, f } = setup([A]);
    f.last(A).open();
    pool.setRelays([B]);
    expect(pool.urls()).toEqual([B]);
    expect(f.last(A).readyState).toBe(3);
    expect(f.last(B)).toBeTruthy();
    pool.stop();
  });

  it('drops a repeat before anything else, and runs the cheap filter before the signature', () => {
    const seenByFilter: string[] = [];
    const { f, got } = setup([A], {
      accept: (e) => {
        seenByFilter.push(String((e as { content?: string }).content));
        return (e as { pubkey?: string }).pubkey !== 'cd'.repeat(32);
      },
    });
    const a = f.last(A);
    a.open();
    const ev = messageEvent(newNostrSecret(), me, 'once', 1000);
    a.push(['EVENT', subOf(a), ev]);
    a.push(['EVENT', subOf(a), ev]);
    expect(got.map((e) => e.content)).toEqual(['once']);
    expect(seenByFilter).toEqual(['once']); // the repeat never reached the filter (nor the signature check)
    a.push(['EVENT', subOf(a), { ...ev, id: 'ff'.repeat(32), pubkey: 'cd'.repeat(32), content: 'stranger' }]);
    expect(got.length).toBe(1);
  });

  it('drops a relay that floods, for its penalty, and kick() does not cut that short', async () => {
    const { pool, f } = setup([A], { budgetEvents: 5, budgetWindowMs: 10_000, penaltyMs: 400 });
    const a = f.last(A);
    a.open();
    const sk = newNostrSecret();
    for (let i = 0; i < 6; i++) a.push(['EVENT', subOf(a), messageEvent(sk, me, `e${i}`, 1000 + i)]);
    expect(a.readyState).toBe(3);
    expect(pool.info()[0]).toMatchObject({ state: 'waiting', note: 'flooding' });
    pool.kick();
    expect(f.sockets.length).toBe(1);
    await waitFor(() => f.sockets.length === 2, 3000);
  });

  it('drops a relay that sends a frame over 256 KiB', () => {
    const { pool, f } = setup([A], { penaltyMs: 60_000 });
    const a = f.last(A);
    a.open();
    a.onmessage?.({ data: '["NOTICE","' + 'x'.repeat(300 * 1024) + '"]' });
    expect(a.readyState).toBe(3);
    expect(pool.info()[0]).toMatchObject({ state: 'waiting', note: 'frame too big' });
    pool.stop();
  });

  it('starts the backoff over only after a connection has stayed up', async () => {
    const { f } = setup([A], { backoffMin: 50, backoffMax: 5000, stableMs: 150 });
    // A relay that accepts and closes at once: each pause doubles (50, 100, 200, 400 ms, with jitter).
    for (let i = 0; i < 4; i++) {
      const s = f.last(A);
      s.open();
      s.drop();
      await waitFor(() => f.last(A) !== s, 3000);
    }
    let s = f.last(A);
    s.open();
    s.drop();
    let t = Date.now();
    await waitFor(() => f.last(A) !== s, 5000);
    expect(Date.now() - t).toBeGreaterThan(400); // 800 ms ± 25%
    // One that stays up past stableMs starts over.
    s = f.last(A);
    s.open();
    await tick(250);
    s.drop();
    t = Date.now();
    await waitFor(() => f.last(A) !== s, 5000);
    expect(Date.now() - t).toBeLessThan(400); // 50 ms ± 25%
  });
});
