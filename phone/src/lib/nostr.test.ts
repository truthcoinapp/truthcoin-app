import { schnorr } from '@noble/curves/secp256k1';
import { describe, expect, it } from 'vitest';
import { fromHex, hex } from './bytes';
import { KIND, messageEvent, newNostrSecret, nostrPub, signEvent, tag, verifyEvent } from './nostr';

describe('Nostr events', () => {
  const sk = newNostrSecret();
  const to = 'ab'.repeat(32);
  const ev = messageEvent(sk, to, '{"v":1}', 1_791_200_000);

  it('are of the link kind, to their recipient, expiring in 5 minutes', () => {
    expect(ev.kind).toBe(KIND);
    expect(ev.pubkey).toBe(nostrPub(sk));
    expect(tag(ev, 'p')).toBe(to);
    expect(tag(ev, 'expiration')).toBe(String(1_791_200_000 + 300));
    expect(verifyEvent(ev)).toBe(true);
  });

  it('are refused with an id that is not theirs', () => {
    const id = (ev.id[0] === '0' ? '1' : '0') + ev.id.slice(1);
    expect(verifyEvent({ ...ev, id })).toBe(false);
    expect(verifyEvent({ ...ev, content: '{"v":2}' })).toBe(false);
    expect(verifyEvent({ ...ev, tags: [['p', 'cd'.repeat(32)]] })).toBe(false);
  });

  it('are refused with a bad signature', () => {
    const sig = ev.sig.slice(0, -1) + (ev.sig.endsWith('0') ? '1' : '0');
    expect(verifyEvent({ ...ev, sig })).toBe(false);
    // Signed by someone else, claiming our pubkey: id is right, signature isn't.
    const other = signEvent(newNostrSecret(), ev.created_at, ev.kind, ev.tags, ev.content);
    expect(verifyEvent({ ...ev, sig: other.sig })).toBe(false);
  });

  it('are refused when malformed', () => {
    for (const bad of [
      null,
      'x',
      { ...ev, pubkey: ev.pubkey.toUpperCase() },
      { ...ev, created_at: 1.5 },
      { ...ev, kind: '21913' },
      { ...ev, tags: [[1]] },
      { ...ev, content: 5 },
      { ...ev, sig: 'zz' },
    ]) {
      expect(verifyEvent(bad)).toBe(false);
    }
  });

  it('sign as BIP340 says (test vector 0: secret 3, aux 0, message 0)', () => {
    const sk3 = new Uint8Array(32);
    sk3[31] = 3;
    expect(nostrPub(sk3).toUpperCase()).toBe('F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9');
    expect(hex(schnorr.sign(new Uint8Array(32), sk3, new Uint8Array(32))).toUpperCase()).toBe(
      'E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DCA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0',
    );
    expect(fromHex(nostrPub(sk3)).length).toBe(32);
  });
});
