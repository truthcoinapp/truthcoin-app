// Trades from the phone: kept before they are sent, and the desktop's answers read for what they mean.
import { describe, expect, it } from 'vitest';
import { fromHex } from './bytes';
import { envelopeJson, importPrivate, openMsg, padJson, parseEnvelope, pubBytes, sealMsg, unpadJson } from './crypto';
import { TradeFlows } from './flows';
import { PhoneLink, type Request } from './link';
import { messageEvent, newNostrSecret, nostrPub, type NostrEvent } from './nostr';
import { loadPending, memoryKv, type Kv } from './store';
import { fakeWsFactory, waitFor } from './testing/fakews';

const D = { s: '1111111111111111111111111111111111111111111111111111111111111111', p: 'BAIX5hfwtkQ5KCePlpmeaaI6TywVK99tbN9m5bgCgtTtGUp968uXcS0t2jyoWqh2Wlb0X8dYWZZS8ol8ZTBuV5Q' };
const P = { s: '2222222222222222222222222222222222222222222222222222222222222222', p: 'BNZak5d8qj0bCBhS_1ennkZfFmBXcwS66tUF3TpIWJzzUBheiVNy32Ih6joTdVfkc_3bZ1XwW9UHw8Uz_OnJEoU' };
const RELAY = 'wss://relay.example';
const args = { marketId: 'a1b2c3d4e5f6', outcome: 1, shares: 100, side: 'buy' as const, limit: 60 };

async function setup(kv: Kv = memoryKv()) {
  const dPub = pubBytes(D.p);
  const pPub = pubBytes(P.p);
  const dPriv = await importPrivate(fromHex(D.s), dPub);
  const pPriv = await importPrivate(fromHex(P.s), pPub);
  const nsec = newNostrSecret();
  const ndSec = newNostrSecret();
  const f = fakeWsFactory();
  const order: string[] = [];
  const spy: Kv = {
    get: kv.get,
    del: kv.del,
    set: async (k, v) => {
      order.push(`kept:${k}`);
      await kv.set(k, v);
    },
  };
  // Late answers go to the flows, as session.ts wires them.
  let flowsRef: TradeFlows | null = null;
  const link = new PhoneLink({ pPriv, pPub, dPub, nsec, npub: nostrPub(nsec), nd: nostrPub(ndSec) }, [RELAY], {
    ws: f.ws,
    onLateReply: (r) => void flowsRef?.late(r),
  });
  link.start();
  const relay = f.last(RELAY);
  relay.open();
  const send = relay.send.bind(relay);
  relay.send = (d: string) => {
    if (d.startsWith('["EVENT"')) order.push('sent');
    send(d);
  };
  const sub = relay.of('REQ')[0][1] as string;
  const flows = new TradeFlows(link, spy, undefined, 2000);
  flowsRef = flows;
  async function requests(): Promise<Request[]> {
    const out: Request[] = [];
    for (const [, ev] of relay.of('EVENT') as [string, NostrEvent][]) {
      out.push(unpadJson(await openMsg({ rPriv: dPriv, rPub: dPub, sPub: pPub }, parseEnvelope(ev.content)!)) as Request);
    }
    return out;
  }
  async function reply(obj: unknown) {
    const env = await sealMsg({ sPriv: dPriv, sPub: dPub, rPub: pPub }, padJson(obj));
    relay.push(['EVENT', sub, messageEvent(ndSec, nostrPub(nsec), envelopeJson(env), Math.floor(Date.now() / 1000))]);
  }
  return { flows, order, relay, requests, reply, kv, link };
}

describe('trade flows', () => {
  it('keep a trade before it is first sent', async () => {
    const { flows, order, relay, kv } = await setup();
    const f = await flows.start(args, { title: 'T', label: 'Yes' });
    await waitFor(() => relay.of('EVENT').length > 0);
    expect(order.indexOf('kept:pending')).toBeGreaterThanOrEqual(0);
    expect(order.indexOf('kept:pending')).toBeLessThan(order.indexOf('sent'));
    expect((await loadPending(kv)).map((p) => p.req.id)).toEqual([f.id]);
  });

  it('send the trade args the protocol names, and end pending with the txid', async () => {
    const { flows, requests, reply, kv } = await setup();
    const f = await flows.start(args, { title: 'T', label: 'Yes' });
    const [req] = await (async () => {
      await waitFor(() => flows.get(f.id) !== undefined);
      let r: Request[] = [];
      await waitFor(() => r.length > 0 || (void requests().then((x) => (r = x)), false));
      return r;
    })();
    expect(req.m).toBe('trade');
    expect(req.a).toEqual({ id: 'a1b2c3d4e5f6', outcome: 1, shares: 100, side: 'buy', limit: 60 });
    await reply({ re: f.id, ok: { status: 'pending', txid: 'ab'.repeat(32) } });
    await waitFor(() => flows.get(f.id)!.state.k === 'pending');
    expect(await loadPending(kv)).toEqual([]); // final: no longer kept
  });

  it('keep an "unsure" trade open (it may have gone), and let it be asked about again', async () => {
    const { flows, reply, kv } = await setup();
    const f = await flows.start(args, { title: 'T', label: 'Yes' });
    await reply({ re: f.id, unsure: 'Not confirmed: check Positions before trying again' });
    await waitFor(() => flows.get(f.id)!.state.k === 'unconfirmed');
    expect((await loadPending(kv)).map((p) => p.state)).toEqual(['unconfirmed']);
    flows.askAgain(f.id);
    expect(flows.get(f.id)!.state.k).toBe('sending');
    await reply({ re: f.id, ok: { status: 'pending', txid: null } });
    await waitFor(() => flows.get(f.id)!.state.k === 'pending');
  });

  it('settle an "unsure" trade when the answer comes later, without asking', async () => {
    const { flows, reply } = await setup();
    const f = await flows.start(args, { title: 'T', label: 'Yes' });
    await reply({ re: f.id, unsure: 'The node did not answer' });
    await waitFor(() => flows.get(f.id)!.state.k === 'unconfirmed');
    expect(flows.get(f.id)!.state).toEqual({ k: 'unconfirmed', why: 'The node did not answer' });
    await reply({ re: f.id, ok: { status: 'pending', txid: 'ab'.repeat(32) } });
    await waitFor(() => flows.get(f.id)!.state.k === 'pending');
  });

  it("show any err as not done, whatever its words", async () => {
    const { flows, reply, link } = await setup();
    const f = await flows.start(args, { title: 'T', label: 'Yes' });
    await reply({ re: f.id, err: 'Refused on your computer' });
    await waitFor(() => flows.get(f.id)!.state.k === 'refused');
    expect(flows.get(f.id)!.state).toEqual({ k: 'refused', msg: 'Refused on your computer' });
    const g = await flows.start(args, { title: 'T', label: 'Yes' });
    await reply({ re: g.id, err: 'Not confirmed: but err means not done' });
    await waitFor(() => flows.get(g.id)!.state.k === 'refused');
    const h = await flows.start(args, { title: 'T', label: 'Yes' });
    const records = "The computer couldn't read some of its trade records, so phones can't trade until you look";
    await reply({ re: h.id, err: records });
    await waitFor(() => flows.get(h.id)!.state.k === 'refused');
    expect(flows.get(h.id)!.state).toEqual({ k: 'refused', msg: records });
    link.stop();
  });

  it('come back after a reload, still held, and are asked about under the same id', async () => {
    const kv = memoryKv();
    const a = await setup(kv);
    const f = await a.flows.start(args, { title: 'T', label: 'Yes' });
    await a.reply({ re: f.id, held: { text: 'Over the limit' } });
    await waitFor(() => a.flows.get(f.id)!.state.k === 'held');
    a.link.stop();
    const b = await setup(kv);
    await b.flows.resume();
    expect(b.flows.get(f.id)!.state).toEqual({ k: 'held', text: 'Over the limit' });
    let reqs: Request[] = [];
    await waitFor(() => reqs.length > 0 || (void b.requests().then((x) => (reqs = x)), false));
    expect(reqs[0]).toEqual(f.req);
  });
});
