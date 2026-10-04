// The link with a hand-played relay: requests sealed to D, replies opened from D, repeats and reordering handled by
// request id, and nothing from anyone else accepted.
import { describe, expect, it } from 'vitest';
import { fromHex } from './bytes';
import { envelopeJson, generateKey, exportPub, importPrivate, openMsg, padJson, parseEnvelope, pubBytes, sealMsg, unpadJson } from './crypto';
import { NoAnswerError, PhoneLink, ReplyError, UnsureError, parseReply, type Reply, type Request } from './link';
import { messageEvent, newNostrSecret, nostrPub, tag, type NostrEvent } from './nostr';
import { fakeWsFactory, tick, waitFor, type FakeWs } from './testing/fakews';

const V = {
  D: {
    secret: '1111111111111111111111111111111111111111111111111111111111111111',
    public: 'BAIX5hfwtkQ5KCePlpmeaaI6TywVK99tbN9m5bgCgtTtGUp968uXcS0t2jyoWqh2Wlb0X8dYWZZS8ol8ZTBuV5Q',
  },
  P: {
    secret: '2222222222222222222222222222222222222222222222222222222222222222',
    public: 'BNZak5d8qj0bCBhS_1ennkZfFmBXcwS66tUF3TpIWJzzUBheiVNy32Ih6joTdVfkc_3bZ1XwW9UHw8Uz_OnJEoU',
  },
};
const RELAY = 'wss://relay.example';

async function setup(opts: ConstructorParameters<typeof PhoneLink>[2] = {}) {
  const dPub = pubBytes(V.D.public);
  const pPub = pubBytes(V.P.public);
  const dPriv = await importPrivate(fromHex(V.D.secret), dPub);
  const pPriv = await importPrivate(fromHex(V.P.secret), pPub);
  const nsec = newNostrSecret();
  const ndSec = newNostrSecret();
  const nd = nostrPub(ndSec);
  const f = fakeWsFactory();
  const late: Reply[] = [];
  const link = new PhoneLink({ pPriv, pPub, dPub, nsec, npub: nostrPub(nsec), nd }, [RELAY], {
    ws: f.ws,
    onLateReply: (r) => late.push(r),
    ...opts,
  });
  link.start();
  const relay = f.last(RELAY);
  relay.open();
  const sub = relay.of('REQ')[0][1] as string;

  /** What the phone sent, as the desktop opens it. */
  async function sentRequests(atLeast = 1): Promise<{ ev: NostrEvent; req: Request }[]> {
    await waitFor(() => relay.of('EVENT').length >= atLeast);
    const out = [];
    for (const [, ev] of relay.of('EVENT') as [string, NostrEvent][]) {
      const env = parseEnvelope(ev.content)!;
      const req = unpadJson(await openMsg({ rPriv: dPriv, rPub: dPub, sPub: pPub }, env)) as Request;
      out.push({ ev, req });
    }
    return out;
  }
  /** The desktop answers (sealed from D, signed by nD unless told otherwise). */
  async function reply(obj: unknown, o: { signer?: Uint8Array; sealer?: CryptoKey; sealerPub?: Uint8Array } = {}) {
    const env = await sealMsg({ sPriv: o.sealer ?? dPriv, sPub: o.sealerPub ?? dPub, rPub: pPub }, padJson(obj));
    const ev = messageEvent(o.signer ?? ndSec, nostrPub(nsec), envelopeJson(env), Math.floor(Date.now() / 1000));
    relay.push(['EVENT', sub, ev]);
    await tick(30); // opening is async
  }
  return { link, relay, sentRequests, reply, late, nd, npub: nostrPub(nsec) };
}

describe('replies', () => {
  it('are parsed strictly', () => {
    const re = '0123456789abcdef0123456789abcdef';
    expect(parseReply({ re, ok: { a: 1 } })).toEqual({ re, k: 'ok', ok: { a: 1 } });
    expect(parseReply({ re, err: 'no‮ way' })).toEqual({ re, k: 'err', err: 'no way' });
    expect(parseReply({ re, held: { text: 'wait' } })).toEqual({ re, k: 'held', text: 'wait' });
    expect(parseReply({ re, unsure: 'May have gone' })).toEqual({ re, k: 'unsure', text: 'May have gone' });
    expect(parseReply({ re, unsure: '' })).toEqual({ re, k: 'unsure', text: 'Not confirmed: check Positions before trying again' });
    expect(parseReply({ re, unsure: 5 })).toBeNull();
    for (const err of [
      'Two phones tried to pair with this code: refuse, and start again',
      "The computer couldn't read some of its trade records, so phones can't trade until you look",
    ]) {
      expect(parseReply({ re, err })).toEqual({ re, k: 'err', err });
    }
    expect(parseReply({ re, unsure: 'x', ok: 1 })).toBeNull();
    expect(parseReply({ re: 'short', ok: 1 })).toBeNull();
    expect(parseReply({ re, ok: 1, err: 'x' })).toBeNull();
    expect(parseReply({ re })).toBeNull();
    expect(parseReply({ re, err: 5 })).toBeNull();
    expect(parseReply({ re, held: 'x' })).toBeNull();
    expect(parseReply([re])).toBeNull();
  });
});

describe('the phone link', () => {
  it('seals each request to D, from nP to nD, with the id, ts, method and args', async () => {
    const { link, sentRequests, reply, nd, npub } = await setup();
    const p = link.request('market', { id: 'abc123abc123' });
    const [{ ev, req }] = await sentRequests();
    expect(ev.pubkey).toBe(npub);
    expect(tag(ev, 'p')).toBe(nd);
    expect(ev.kind).toBe(21913);
    expect(req.m).toBe('market');
    expect(req.a).toEqual({ id: 'abc123abc123' });
    expect(req.id).toMatch(/^[0-9a-f]{32}$/);
    expect(Math.abs(req.ts - Date.now() / 1000)).toBeLessThan(5);
    await reply({ re: req.id, ok: { fine: true } });
    expect(await p).toEqual({ fine: true });
  });

  it('drops a repeated final answer, and a held that comes after the final one', async () => {
    const { link, sentRequests, reply, late } = await setup();
    const held: string[] = [];
    const p = link.request('trade', { side: 'buy' }, { onHeld: (t) => held.push(t) });
    const [{ req }] = await sentRequests();
    await reply({ re: req.id, ok: { status: 'pending', txid: 'aa' } });
    await reply({ re: req.id, ok: { status: 'pending', txid: 'bb' } });
    await reply({ re: req.id, held: { text: 'late held' } });
    expect(await p).toEqual({ status: 'pending', txid: 'aa' });
    expect(held).toEqual([]);
    expect(late).toEqual([]);
  });

  it('shows a held request once, waits past the timeout, then takes the final answer', async () => {
    const { link, sentRequests, reply } = await setup({ timeoutMs: 800, resendAt: [] });
    const held: string[] = [];
    const p = link.request('trade', { side: 'buy' }, { onHeld: (t) => held.push(t), heldPollMs: 10_000 });
    const [{ req }] = await sentRequests();
    await reply({ re: req.id, held: { text: 'Confirm on your computer' } });
    await reply({ re: req.id, held: { text: 'Confirm on your computer' } });
    await tick(1000); // past the timeout: a held request doesn't give up
    await reply({ re: req.id, ok: { status: 'pending', txid: 'cc' } });
    expect(await p).toEqual({ status: 'pending', txid: 'cc' });
    expect(held).toEqual(['Confirm on your computer']);
  });

  it('takes a final answer that overtakes its held one', async () => {
    const { link, sentRequests, reply } = await setup();
    const held: string[] = [];
    const p = link.request('trade', {}, { onHeld: (t) => held.push(t) });
    const [{ req }] = await sentRequests();
    await reply({ re: req.id, ok: 1 });
    await reply({ re: req.id, held: { text: 'h' } });
    expect(await p).toBe(1);
    expect(held).toEqual([]);
  });

  it("turns the desktop's no into ReplyError", async () => {
    const { link, sentRequests, reply } = await setup();
    const p = link.request('trade', {});
    p.catch(() => undefined); // checked below, after the reply
    const [{ req }] = await sentRequests();
    await reply({ re: req.id, err: 'Not enough coins.' });
    await expect(p).rejects.toThrow(ReplyError);
    await expect(p).rejects.toThrow('Not enough coins.');
  });

  it('turns "unsure" into UnsureError, and leaves the id open for a later answer', async () => {
    const { link, sentRequests, reply, late } = await setup();
    const p = link.request('trade', {});
    p.catch(() => undefined); // checked below
    const [{ req }] = await sentRequests();
    await reply({ re: req.id, unsure: 'The node did not answer' });
    await expect(p).rejects.toThrow(UnsureError);
    await expect(p).rejects.toThrow('The node did not answer');
    // Not final: the desktop's later answer under the same id is passed on, not dropped as a repeat.
    await reply({ re: req.id, ok: { status: 'pending', txid: null } });
    await waitFor(() => late.length > 0);
    expect(late[0]).toEqual({ re: req.id, k: 'ok', ok: { status: 'pending', txid: null } });
  });

  it('accepts nothing from another Nostr key, nor anything not sealed by D', async () => {
    const { link, sentRequests, reply } = await setup({ timeoutMs: 400, resendAt: [] });
    const p = link.request('status');
    const [{ req }] = await sentRequests();
    // D's sealing, but signed by a Nostr key that isn't the desktop's.
    await reply({ re: req.id, ok: 'from a stranger' }, { signer: newNostrSecret() });
    // The desktop's Nostr key (as if stolen), but sealed by a key that isn't D.
    const x = await generateKey();
    await reply({ re: req.id, ok: 'not D' }, { sealer: x.privateKey, sealerPub: await exportPub(x.publicKey) });
    await expect(p).rejects.toThrow(NoAnswerError);
  });

  it('sends a request again (same id and ts, sealed afresh) until it gives up', async () => {
    const { link, relay, sentRequests } = await setup({ timeoutMs: 300, resendAt: [80, 160] });
    const p = link.request('status');
    await expect(p).rejects.toThrow(NoAnswerError);
    const sent = await sentRequests(3);
    expect(sent.length).toBe(3);
    expect(new Set(sent.map((s) => s.req.id)).size).toBe(1);
    expect(new Set(sent.map((s) => s.req.ts)).size).toBe(1);
    expect(new Set(sent.map((s) => s.ev.id)).size).toBe(3);
    expect(new Set(sent.map((s) => parseEnvelope(s.ev.content)!.e)).size).toBe(3);
    expect((relay as FakeWs).of('EVENT').length).toBe(3);
  });

  it('asks again under the same id, and a late answer goes to whoever listens for it', async () => {
    const { link, sentRequests, reply, late } = await setup({ timeoutMs: 100, resendAt: [] });
    const first = link.makeRequest('trade', { side: 'buy', limit: 5 });
    await expect(link.ask(first)).rejects.toThrow(NoAnswerError);
    // Nobody waits now: the answer that arrives late is handed on.
    await reply({ re: first.id, ok: { status: 'pending', txid: 'dd' } });
    await waitFor(() => late.length > 0);
    expect(late).toEqual([{ re: first.id, k: 'ok', ok: { status: 'pending', txid: 'dd' } }]);
    // Asking again gets the desktop's repeat of the same answer.
    const again = link.ask(first, { timeoutMs: 1000 });
    const sent = await sentRequests(2);
    expect(sent.length).toBe(2);
    expect(sent[1].req).toEqual(first);
    await reply({ re: first.id, ok: { status: 'pending', txid: 'dd' } });
    expect(await again).toEqual({ status: 'pending', txid: 'dd' });
  });

  it('lets two asks of the same id share one answer', async () => {
    const { link, sentRequests, reply } = await setup();
    const req = link.makeRequest('trade', {});
    const a = link.ask(req);
    const b = link.ask(req);
    expect((await sentRequests(2)).length).toBe(2);
    await reply({ re: req.id, ok: 7 });
    expect(await a).toBe(7);
    expect(await b).toBe(7);
  });
});
