// Byte helpers: base64url (no padding), hex, UTF-8. Strict on input: anything odd throws.

export type Bytes = Uint8Array<ArrayBuffer>;

const te = new TextEncoder();
const td = new TextDecoder('utf-8', { fatal: true });

export function b64u(data: Uint8Array | ArrayBuffer): string {
  const b = data instanceof Uint8Array ? data : new Uint8Array(data);
  let s = '';
  for (let i = 0; i < b.length; i++) s += String.fromCharCode(b[i]);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function fromB64u(s: string): Bytes {
  if (typeof s !== 'string' || !/^[A-Za-z0-9_-]*$/.test(s) || s.length % 4 === 1) throw new Error('bad base64url');
  const pad = s.length % 4 === 0 ? '' : '='.repeat(4 - (s.length % 4));
  const bin = atob(s.replace(/-/g, '+').replace(/_/g, '/') + pad);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export function hex(b: Uint8Array): string {
  let s = '';
  for (let i = 0; i < b.length; i++) s += b[i].toString(16).padStart(2, '0');
  return s;
}

export function fromHex(s: string): Bytes {
  if (typeof s !== 'string' || s.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(s)) throw new Error('bad hex');
  const out = new Uint8Array(s.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export function concat(...parts: Uint8Array[]): Bytes {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

export const utf8 = (s: string): Bytes => te.encode(s) as Bytes;

/** UTF-8 to text; invalid UTF-8 throws. */
export const fromUtf8 = (b: Uint8Array): string => td.decode(b);

export function randomBytes(n: number): Bytes {
  return globalThis.crypto.getRandomValues(new Uint8Array(n));
}

/** A copy whose buffer is a plain ArrayBuffer (what WebCrypto's types ask for). */
export function own(b: Uint8Array): Bytes {
  return new Uint8Array(b);
}

export function equalBytes(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a[i] ^ b[i];
  return d === 0;
}

/** 16 random bytes as hex: a request id. */
export function newRequestId(): string {
  return hex(randomBytes(16));
}

export const isRequestId = (s: unknown): s is string => typeof s === 'string' && /^[0-9a-f]{32}$/.test(s);

/** A Nostr public key as the link accepts it: 64 lowercase hex characters (x-only). */
export const isNostrPub = (s: unknown): s is string => typeof s === 'string' && /^[0-9a-f]{64}$/.test(s);
