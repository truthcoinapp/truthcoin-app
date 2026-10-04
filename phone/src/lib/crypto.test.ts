import { describe, expect, it } from 'vitest';
import { b64u, fromB64u, fromUtf8, utf8 } from './bytes';
import {
  exportPub,
  generateKey,
  newEphemeral,
  openMsg,
  pad,
  padJson,
  pubBytes,
  sealMsg,
  unpadJson,
  parseEnvelope,
  envelopeJson,
} from './crypto';

describe('padding', () => {
  it('pads to 1024, 4096 or 16384 bytes, whichever fits first', () => {
    expect(pad(utf8('{}')).length).toBe(1024);
    expect(pad(new Uint8Array(1024).fill(0x78)).length).toBe(1024);
    expect(pad(new Uint8Array(1025).fill(0x78)).length).toBe(4096);
    expect(pad(new Uint8Array(4097).fill(0x78)).length).toBe(16384);
    expect(pad(new Uint8Array(16384).fill(0x78)).length).toBe(16384);
  });

  it('refuses anything bigger than 16 KiB', () => {
    expect(() => pad(new Uint8Array(16385))).toThrow();
  });

  it('pads with spaces, which JSON ignores', () => {
    const p = padJson({ a: 1, s: 'é' });
    expect(p.length).toBe(1024);
    expect(p[p.length - 1]).toBe(0x20);
    expect(unpadJson(p)).toEqual({ a: 1, s: 'é' });
  });

  it('refuses a plaintext that is not UTF-8 JSON', () => {
    expect(() => unpadJson(new Uint8Array([0xff, 0xfe]))).toThrow();
    expect(() => unpadJson(utf8('{"a":'))).toThrow();
  });
});

describe('sealed messages', () => {
  it('open only for their receiver, from their sender, unchanged', async () => {
    const s = await generateKey();
    const r = await generateKey();
    const x = await generateKey();
    const [sPub, rPub, xPub] = await Promise.all([s, r, x].map((k) => exportPub(k.publicKey)));
    const pt = padJson({ id: '1' });
    const env = await sealMsg({ sPriv: s.privateKey, sPub, rPub }, pt);
    expect(await openMsg({ rPriv: r.privateKey, rPub, sPub }, env)).toEqual(pt);
    await expect(openMsg({ rPriv: x.privateKey, rPub: xPub, sPub }, env)).rejects.toThrow();
    await expect(openMsg({ rPriv: r.privateKey, rPub, sPub: xPub }, env)).rejects.toThrow();
    const ct = fromB64u(env.ct);
    ct[0] ^= 1;
    await expect(openMsg({ rPriv: r.privateKey, rPub, sPub }, { ...env, ct: b64u(ct) })).rejects.toThrow();
    const e2 = await newEphemeral();
    await expect(openMsg({ rPriv: r.privateKey, rPub, sPub }, { ...env, e: b64u(e2.pub) })).rejects.toThrow();
  });

  it('uses a fresh ephemeral key and nonce each time', async () => {
    const s = await generateKey();
    const r = await generateKey();
    const sPub = await exportPub(s.publicKey);
    const rPub = await exportPub(r.publicKey);
    const a = await sealMsg({ sPriv: s.privateKey, sPub, rPub }, padJson({}));
    const b = await sealMsg({ sPriv: s.privateKey, sPub, rPub }, padJson({}));
    expect(a.e).not.toBe(b.e);
    expect(a.n).not.toBe(b.n);
    expect(a.ct).not.toBe(b.ct);
  });

  it('refuses public keys that are not 65-byte uncompressed points', async () => {
    const kp = await generateKey();
    const pub = await exportPub(kp.publicKey);
    expect(pubBytes(b64u(pub))).toEqual(pub);
    const compressed = new Uint8Array(33);
    compressed[0] = 2;
    expect(() => pubBytes(b64u(compressed))).toThrow();
    expect(() => pubBytes(b64u(pub) + '=')).toThrow();
    expect(() => pubBytes('not base64!')).toThrow();
  });
});

describe('envelopes', () => {
  const env = { v: 1 as const, k: 'msg' as const, e: 'E', n: 'N', ct: 'CT' };
  it('round-trip as compact JSON', () => {
    expect(envelopeJson(env)).toBe('{"v":1,"k":"msg","e":"E","n":"N","ct":"CT"}');
    expect(parseEnvelope(envelopeJson(env))).toEqual(env);
  });
  it('refuse anything else', () => {
    expect(parseEnvelope('nope')).toBeNull();
    expect(parseEnvelope('[]')).toBeNull();
    expect(parseEnvelope(JSON.stringify({ ...env, v: 2 }))).toBeNull();
    expect(parseEnvelope(JSON.stringify({ ...env, k: 'other' }))).toBeNull();
    expect(parseEnvelope(JSON.stringify({ ...env, ct: 5 }))).toBeNull();
    expect(fromUtf8(utf8(envelopeJson(env)))).toContain('"k":"msg"');
  });
});
