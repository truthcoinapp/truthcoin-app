import { describe, expect, it } from 'vitest';
import { b64u, utf8 } from './bytes';
import { attemptFor, newAttempt, parsePairValue } from './pairing';
import { relayList, validRelay } from './relays';

const D = 'BAIX5hfwtkQ5KCePlpmeaaI6TywVK99tbN9m5bgCgtTtGUp968uXcS0t2jyoWqh2Wlb0X8dYWZZS8ol8ZTBuV5Q';
const good = {
  v: 1,
  r: ['wss://relay.damus.io', 'wss://nos.lol', 'wss://relay.primal.net'],
  n: 'ab'.repeat(32),
  d: D,
  c: 'DAwMDAwMDAwMDAwMDAwMDA',
  x: 1791200300,
};
const enc = (o: unknown) => b64u(utf8(JSON.stringify(o)));

describe('the pairing code', () => {
  it('reads a good one', async () => {
    expect(await parsePairValue(enc(good), false)).toEqual(good);
  });

  it('refuses relays that are not wss://, too many, or none', async () => {
    await expect(parsePairValue(enc({ ...good, r: ['ws://relay.damus.io'] }), false)).rejects.toThrow(/wss/);
    await expect(parsePairValue(enc({ ...good, r: ['https://relay.damus.io'] }), false)).rejects.toThrow();
    await expect(parsePairValue(enc({ ...good, r: [] }), false)).rejects.toThrow();
    await expect(parsePairValue(enc({ ...good, r: Array(6).fill('wss://a.example') }), false)).rejects.toThrow();
    await expect(parsePairValue(enc({ ...good, r: 'wss://nos.lol' }), false)).rejects.toThrow();
  });

  it('takes local test relays only when allowed', async () => {
    const local = { ...good, r: ['ws://127.0.0.1:7447'] };
    await expect(parsePairValue(enc(local), false)).rejects.toThrow();
    expect((await parsePairValue(enc(local), true)).r).toEqual(['ws://127.0.0.1:7447']);
  });

  it('refuses a bad desktop key, Nostr key, code or expiry', async () => {
    const compressed = b64u(new Uint8Array([2, ...new Uint8Array(32)]));
    const offCurve = b64u(new Uint8Array([4, ...new Uint8Array(64).fill(1)]));
    for (const bad of [
      { ...good, d: compressed },
      { ...good, d: offCurve },
      { ...good, n: 'AB'.repeat(32) },
      { ...good, n: 'ab'.repeat(31) },
      { ...good, c: b64u(new Uint8Array(15)) },
      { ...good, c: 'not base64!' },
      { ...good, x: '1791200300' },
      { ...good, x: -5 },
    ]) {
      await expect(parsePairValue(enc(bad), false)).rejects.toThrow(/damaged/);
    }
  });

  it('refuses other versions and junk', async () => {
    await expect(parsePairValue(enc({ ...good, v: 2 }), false)).rejects.toThrow(/version/);
    await expect(parsePairValue('!!!', false)).rejects.toThrow(/damaged/);
    await expect(parsePairValue(enc([1, 2]), false)).rejects.toThrow(/damaged/);
    await expect(parsePairValue(b64u(new Uint8Array([0xff, 0xfe])), false)).rejects.toThrow(/damaged/);
  });
});

describe('a kept pairing attempt', () => {
  it('answers its own code only, until the code expires (plus the grace)', async () => {
    const link = await parsePairValue(enc(good), false);
    const a = await newAttempt(link, 'My\u202E phone');
    expect(a.name).toBe('My phone');
    expect(a.code).toMatch(/^\d{3} \d{3}$/);
    expect(a.pPriv.extractable).toBe(false);
    expect(a.ePriv.extractable).toBe(false);
    expect(attemptFor(a, link, good.x - 10)).toBe(a);
    expect(attemptFor(a, link, good.x + 10)).toBe(a); // within the grace: a late answer may still come
    expect(attemptFor(a, link, good.x + 31)).toBeNull();
    expect(attemptFor(a, { ...link, c: b64u(new Uint8Array(16).fill(1)) }, good.x - 10)).toBeNull();
    expect(attemptFor(a, { ...link, n: 'cd'.repeat(32) }, good.x - 10)).toBeNull();
    expect(attemptFor(null, link, good.x - 10)).toBeNull();
  });
});

describe('relay addresses', () => {
  it('are wss:// with a host, no credentials or fragments', () => {
    expect(validRelay('wss://nos.lol', false)).toBe('wss://nos.lol');
    expect(validRelay('wss://user:pw@nos.lol', false)).toBeNull();
    expect(validRelay('wss://nos.lol/#x', false)).toBeNull();
    expect(validRelay('wss://nos.lol/ path', false)).toBeNull();
    expect(validRelay('ws://localhost:7447', true)).toBe('ws://localhost:7447');
    expect(validRelay('ws://10.0.0.1:7447', true)).toBeNull();
  });
  it('lose their duplicates', () => {
    expect(relayList(['wss://nos.lol', 'wss://nos.lol'], false)).toEqual(['wss://nos.lol']);
  });
});
