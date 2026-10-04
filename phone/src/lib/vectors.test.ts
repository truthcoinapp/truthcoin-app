// The shared test vectors (src-tauri/testdata/protocol-v1.json, made by the desktop's Rust code): this page's
// sealing, pairing, comparison code and Nostr events must match them byte for byte.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { fromB64u, fromHex, fromUtf8, utf8 } from './bytes';
import {
  ecdh,
  envelopeJson,
  exportPub,
  importPrivate,
  openMsg,
  openPair,
  pad,
  pairCode,
  parseEnvelope,
  pubBytes,
  sealMsg,
  sealPair,
  type Envelope,
} from './crypto';
import { keyFingerprint } from './format';
import { eventId, nostrPub, signEvent, verifyEvent, type NostrEvent } from './nostr';

const V = JSON.parse(
  readFileSync(new URL('../../../src-tauri/testdata/protocol-v1.json', import.meta.url), 'utf8'),
) as any;

type Name = 'D' | 'P' | 'E1' | 'E2' | 'E3';
const pub = (k: Name) => pubBytes(V.keys[k].public);
const priv = (k: Name) => importPrivate(fromHex(V.keys[k].secret), pub(k));
const eph = async (k: Name) => ({ priv: await priv(k), pub: pub(k) });

describe('protocol-v1.json', () => {
  it('imports every secret coherently with its public key (ECDH agrees both ways)', async () => {
    const names: Name[] = ['D', 'P', 'E1', 'E2', 'E3'];
    for (const a of names) {
      for (const b of names) {
        if (a >= b) continue;
        expect(await ecdh(await priv(a), pub(b))).toEqual(await ecdh(await priv(b), pub(a)));
      }
    }
  });

  it('seals the pairing request exactly as the vector', async () => {
    const pt = utf8(V.pair.plaintext);
    expect(pt.length).toBe(1024);
    const { env, ePub } = await sealPair(
      { dPub: pub('D'), c: fromB64u(V.pair.C), e: await eph('E1'), nonce: fromB64u(V.pair.nonce) },
      pt,
    );
    expect(env).toEqual(V.pair.envelope);
    expect(ePub).toEqual(pub('E1'));
  });

  it('opens the pairing request with D and C, and not with another C', async () => {
    const env = V.pair.envelope as Envelope;
    const { pt, ePub } = await openPair({ dPriv: await priv('D'), dPub: pub('D'), c: fromB64u(V.pair.C) }, env);
    expect(fromUtf8(pt)).toBe(V.pair.plaintext);
    expect(ePub).toEqual(pub('E1'));
    const other = new Uint8Array(16).fill(0x0d);
    await expect(openPair({ dPriv: await priv('D'), dPub: pub('D'), c: other }, env)).rejects.toThrow();
  });

  it("shows the desktop key's fingerprint as the vector (and the desktop) does", () => {
    expect(keyFingerprint(V.keys.D.public)).toBe(V.keys.D.fingerprint);
  });

  it('makes the comparison code of the vector', async () => {
    expect(await pairCode(pub('D'), pub('P'), pub('E1'), fromB64u(V.pair.C))).toBe(V.pair.comparison_code);
  });

  it('seals the request (P to D) exactly as the vector, and D opens it', async () => {
    const env = await sealMsg(
      { sPriv: await priv('P'), sPub: pub('P'), rPub: pub('D'), e: await eph('E2'), nonce: fromB64u(V.request.nonce) },
      utf8(V.request.plaintext),
    );
    expect(env).toEqual(V.request.envelope);
    const pt = await openMsg({ rPriv: await priv('D'), rPub: pub('D'), sPub: pub('P') }, V.request.envelope);
    expect(fromUtf8(pt)).toBe(V.request.plaintext);
  });

  it('opens the reply (D to P) and seals it exactly as the vector', async () => {
    const pt = await openMsg({ rPriv: await priv('P'), rPub: pub('P'), sPub: pub('D') }, V.reply.envelope);
    expect(fromUtf8(pt)).toBe(V.reply.plaintext);
    expect(JSON.parse(fromUtf8(pt))).toEqual({ re: '0123456789abcdef0123456789abcdef', ok: { height: 42 } });
    const env = await sealMsg(
      { sPriv: await priv('D'), sPub: pub('D'), rPub: pub('P'), e: await eph('E3'), nonce: fromB64u(V.reply.nonce) },
      utf8(V.reply.plaintext),
    );
    expect(env).toEqual(V.reply.envelope);
  });

  it('refuses the reply from a claimed sender other than D, and to a receiver other than P', async () => {
    await expect(openMsg({ rPriv: await priv('P'), rPub: pub('P'), sPub: pub('E1') }, V.reply.envelope)).rejects.toThrow();
    await expect(openMsg({ rPriv: await priv('D'), rPub: pub('D'), sPub: pub('D') }, V.reply.envelope)).rejects.toThrow();
    const bad = { ...V.reply.envelope, ct: V.reply.envelope.ct.slice(0, -2) + (V.reply.envelope.ct.endsWith('A') ? 'BA' : 'AA') };
    await expect(openMsg({ rPriv: await priv('P'), rPub: pub('P'), sPub: pub('D') }, bad)).rejects.toThrow();
  });

  it('pads the vector plaintexts the same way', () => {
    expect(fromUtf8(pad(utf8('{"re":"0123456789abcdef0123456789abcdef","ok":{"height":42}}')))).toBe(V.reply.plaintext);
    expect(fromUtf8(pad(utf8(V.request.plaintext.trimEnd())))).toBe(V.request.plaintext);
  });

  it('writes the envelope as the event content does (compact, fields in order)', () => {
    expect(envelopeJson(V.request.envelope)).toBe(V.nostr.event.content);
    expect(parseEnvelope(V.nostr.event.content)).toEqual(V.request.envelope);
  });

  it('signs the Nostr event exactly as the vector (fixed aux), and verifies it', () => {
    const sk = fromHex(V.nostr.secret);
    expect(nostrPub(sk)).toBe(V.nostr.pubkey);
    const e = V.nostr.event as NostrEvent;
    const signed = signEvent(sk, e.created_at, e.kind, e.tags, e.content, fromHex(V.nostr.aux));
    expect(signed).toEqual(e);
    expect(Buffer.from(eventId(e.pubkey, e.created_at, e.kind, e.tags, e.content)).toString('hex')).toBe(e.id);
    expect(verifyEvent(e)).toBe(true);
  });

  it('exports the public key of a generated non-extractable key', async () => {
    const kp = await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, false, ['deriveBits']);
    expect((await exportPub(kp.publicKey)).length).toBe(65);
    await expect(crypto.subtle.exportKey('jwk', kp.privateKey)).rejects.toThrow();
  });
});
