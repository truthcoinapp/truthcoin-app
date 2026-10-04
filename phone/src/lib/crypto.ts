// The phone link's sealing (docs/PROTOCOL.md, "Pairing" and "Sealed messages"), in WebCrypto only: P-256 ECDH,
// HKDF-SHA256, AES-256-GCM, SHA-256. Every message is sealed on its own, because relays may drop, repeat or reorder
// it. Ephemeral keys and nonces can be passed in so the shared test vectors (src-tauri/testdata/protocol-v1.json)
// can drive these functions; in use they are fresh and random.
import { b64u, concat, fromB64u, fromUtf8, randomBytes, utf8, type Bytes } from './bytes';

export const PAIR_LABEL = 'tcr-pair-v1';
export const MSG_LABEL = 'tcr-msg-v1';
export const SAS_LABEL = 'tcr-pair-sas-v1';
/** Plaintexts are padded with spaces to one of these sizes, so relays can't tell a trade from a balance check. */
export const PAD_SIZES = [1024, 4096, 16384] as const;

const EC: EcKeyImportParams = { name: 'ECDH', namedCurve: 'P-256' };
const subtle = () => globalThis.crypto.subtle;

// ---------- keys ----------

/** A P-256 key pair whose private half no script can read (the phone's device key P, and ephemeral keys). */
export async function generateKey(): Promise<CryptoKeyPair> {
  return (await subtle().generateKey(EC, false, ['deriveBits'])) as CryptoKeyPair;
}

/** The 65-byte uncompressed SEC1 public key. */
export async function exportPub(key: CryptoKey): Promise<Bytes> {
  return new Uint8Array(await subtle().exportKey('raw', key));
}

/** A P-256 public key from its 65 uncompressed bytes. Compressed or off-curve points are refused. */
export async function importPub(raw: Uint8Array): Promise<CryptoKey> {
  if (raw.length !== 65 || raw[0] !== 4) throw new Error('public key must be 65-byte uncompressed SEC1');
  return subtle().importKey('raw', new Uint8Array(raw), EC, true, []);
}

/** A public key as it travels: b64u of the 65 uncompressed bytes. Throws on anything else. */
export function pubBytes(s: string): Bytes {
  const b = fromB64u(s);
  if (b.length !== 65 || b[0] !== 4) throw new Error('public key must be 65-byte uncompressed SEC1');
  return b;
}

/** A fixed private key (tests and the test desktop): `secretHex` is the 32-byte scalar, `pub` its public key. */
export async function importPrivate(secret: Uint8Array, pub: Uint8Array, extractable = false): Promise<CryptoKey> {
  if (secret.length !== 32 || pub.length !== 65 || pub[0] !== 4) throw new Error('bad key');
  const jwk: JsonWebKey = {
    kty: 'EC',
    crv: 'P-256',
    d: b64u(secret),
    x: b64u(pub.slice(1, 33)),
    y: b64u(pub.slice(33, 65)),
    ext: extractable,
  };
  return subtle().importKey('jwk', jwk, EC, extractable, ['deriveBits']);
}

// ---------- primitives ----------

export async function sha256(data: Uint8Array): Promise<Bytes> {
  return new Uint8Array(await subtle().digest('SHA-256', new Uint8Array(data)));
}

/** The 32-byte X coordinate of the shared point. */
export async function ecdh(priv: CryptoKey, pub: Uint8Array | CryptoKey): Promise<Bytes> {
  const p = pub instanceof Uint8Array ? await importPub(pub) : pub;
  return new Uint8Array(await subtle().deriveBits({ name: 'ECDH', public: p }, priv, 256));
}

export async function hkdf32(ikm: Uint8Array, salt: Uint8Array, info: string): Promise<Bytes> {
  const k = await subtle().importKey('raw', new Uint8Array(ikm), 'HKDF', false, ['deriveBits']);
  return new Uint8Array(
    await subtle().deriveBits({ name: 'HKDF', hash: 'SHA-256', salt: new Uint8Array(salt), info: utf8(info) }, k, 256),
  );
}

async function gcmKey(raw: Uint8Array): Promise<CryptoKey> {
  return subtle().importKey('raw', new Uint8Array(raw), 'AES-GCM', false, ['encrypt', 'decrypt']);
}

async function gcmSeal(key: Uint8Array, nonce: Uint8Array, pt: Uint8Array, aad: Uint8Array): Promise<Bytes> {
  const k = await gcmKey(key);
  return new Uint8Array(
    await subtle().encrypt({ name: 'AES-GCM', iv: new Uint8Array(nonce), additionalData: new Uint8Array(aad) }, k, new Uint8Array(pt)),
  );
}

async function gcmOpen(key: Uint8Array, nonce: Uint8Array, ct: Uint8Array, aad: Uint8Array): Promise<Bytes> {
  const k = await gcmKey(key);
  try {
    return new Uint8Array(
      await subtle().decrypt({ name: 'AES-GCM', iv: new Uint8Array(nonce), additionalData: new Uint8Array(aad) }, k, new Uint8Array(ct)),
    );
  } catch {
    throw new Error("doesn't open");
  }
}

// ---------- padding ----------

/** JSON padded with spaces to the smallest size that fits (JSON ignores trailing spaces). Bigger than 16 KiB throws. */
export function pad(json: Uint8Array): Bytes {
  const size = PAD_SIZES.find((s) => s >= json.length);
  if (size === undefined) throw new Error('message too big for one envelope');
  const out = new Uint8Array(size).fill(0x20);
  out.set(json);
  return out;
}

/** An object as a padded plaintext. */
export const padJson = (obj: unknown): Bytes => pad(utf8(JSON.stringify(obj)));

/** A plaintext back to its JSON value (the padding is whitespace, which JSON.parse skips). */
export function unpadJson(pt: Uint8Array): unknown {
  return JSON.parse(fromUtf8(pt));
}

// ---------- envelopes ----------

/** What travels in an event's content. */
export interface Envelope {
  v: 1;
  /** "pair" (a phone asking to pair) or "msg". */
  k: 'pair' | 'msg';
  /** The sender's ephemeral P-256 key, b64u uncompressed SEC1. */
  e: string;
  /** The 12-byte AES-GCM nonce, b64u. */
  n: string;
  /** Ciphertext and tag, b64u. */
  ct: string;
}

/** The envelope as compact JSON, its fields in the protocol's order. */
export function envelopeJson(env: Envelope): string {
  return JSON.stringify({ v: env.v, k: env.k, e: env.e, n: env.n, ct: env.ct });
}

/** An event's content as an envelope, or null when it isn't one. */
export function parseEnvelope(content: string): Envelope | null {
  let o: unknown;
  try {
    o = JSON.parse(content);
  } catch {
    return null;
  }
  if (!o || typeof o !== 'object' || Array.isArray(o)) return null;
  const r = o as Record<string, unknown>;
  if (r.v !== 1 || (r.k !== 'pair' && r.k !== 'msg')) return null;
  if (typeof r.e !== 'string' || typeof r.n !== 'string' || typeof r.ct !== 'string') return null;
  if (r.ct.length > 30000) return null; // 16 KiB plaintext + tag, in base64url, fits well within this
  return { v: 1, k: r.k, e: r.e, n: r.n, ct: r.ct };
}

function nonceOf(env: Envelope): Bytes {
  const n = fromB64u(env.n);
  if (n.length !== 12) throw new Error('bad nonce');
  return n;
}

/** An ephemeral key: its private half and its public bytes. */
export interface Ephemeral {
  priv: CryptoKey;
  pub: Bytes;
}

export async function newEphemeral(): Promise<Ephemeral> {
  const kp = await generateKey();
  return { priv: kp.privateKey, pub: await exportPub(kp.publicKey) };
}

// ---------- sealed messages ----------

/**
 * The key and associated data of a message from S to R with ephemeral E:
 * aad = "tcr-msg-v1" || S_pub || R_pub || E_pub; k = HKDF(ECDH(E, R) || ECDH(S, R), salt = SHA-256(aad), "tcr-msg-v1").
 */
async function msgKey(dhER: Uint8Array, dhSR: Uint8Array, sPub: Uint8Array, rPub: Uint8Array, ePub: Uint8Array) {
  const aad = concat(utf8(MSG_LABEL), sPub, rPub, ePub);
  const k = await hkdf32(concat(dhER, dhSR), await sha256(aad), MSG_LABEL);
  return { k, aad };
}

/** Seal a padded plaintext from the static key S to R. */
export async function sealMsg(
  o: { sPriv: CryptoKey; sPub: Uint8Array; rPub: Uint8Array; e?: Ephemeral; nonce?: Uint8Array },
  pt: Uint8Array,
): Promise<Envelope> {
  const e = o.e ?? (await newEphemeral());
  const nonce = o.nonce ?? randomBytes(12);
  const r = await importPub(o.rPub);
  const { k, aad } = await msgKey(await ecdh(e.priv, r), await ecdh(o.sPriv, r), o.sPub, o.rPub, e.pub);
  return { v: 1, k: 'msg', e: b64u(e.pub), n: b64u(nonce), ct: b64u(await gcmSeal(k, nonce, pt, aad)) };
}

/** Open a message to R that claims to come from S. Anything that doesn't open throws, and is to be dropped. */
export async function openMsg(o: { rPriv: CryptoKey; rPub: Uint8Array; sPub: Uint8Array }, env: Envelope): Promise<Bytes> {
  if (env.v !== 1 || env.k !== 'msg') throw new Error('not a v1 message');
  const ePub = pubBytes(env.e);
  const e = await importPub(ePub);
  const { k, aad } = await msgKey(await ecdh(o.rPriv, e), await ecdh(o.rPriv, o.sPub), o.sPub, o.rPub, ePub);
  return gcmOpen(k, nonceOf(env), fromB64u(env.ct), aad);
}

// ---------- pairing ----------

/** k = HKDF(ECDH(E, D), salt = C, "tcr-pair-v1"); aad = "tcr-pair-v1" || D_pub || E_pub. */
async function pairKey(dh: Uint8Array, c: Uint8Array, dPub: Uint8Array, ePub: Uint8Array) {
  return { k: await hkdf32(dh, c, PAIR_LABEL), aad: concat(utf8(PAIR_LABEL), dPub, ePub) };
}

/** The phone's pairing request, sealed to the desktop's D with the code C from the QR code. */
export async function sealPair(
  o: { dPub: Uint8Array; c: Uint8Array; e?: Ephemeral; nonce?: Uint8Array },
  pt: Uint8Array,
): Promise<{ env: Envelope; ePub: Bytes }> {
  const e = o.e ?? (await newEphemeral());
  const nonce = o.nonce ?? randomBytes(12);
  const { k, aad } = await pairKey(await ecdh(e.priv, o.dPub), o.c, o.dPub, e.pub);
  const env: Envelope = { v: 1, k: 'pair', e: b64u(e.pub), n: b64u(nonce), ct: b64u(await gcmSeal(k, nonce, pt, aad)) };
  return { env, ePub: e.pub };
}

/** The desktop's side of pairing (tests and the test desktop only). */
export async function openPair(
  o: { dPriv: CryptoKey; dPub: Uint8Array; c: Uint8Array },
  env: Envelope,
): Promise<{ pt: Bytes; ePub: Bytes }> {
  if (env.v !== 1 || env.k !== 'pair') throw new Error('not a v1 pairing request');
  const ePub = pubBytes(env.e);
  const { k, aad } = await pairKey(await ecdh(o.dPriv, ePub), o.c, o.dPub, ePub);
  return { pt: await gcmOpen(k, nonceOf(env), fromB64u(env.ct), aad), ePub };
}

/** "042917" -> "042 917". */
function groupCode(n: number): string {
  const s = String(n).padStart(6, '0');
  return `${s.slice(0, 3)} ${s.slice(3)}`;
}

/**
 * The comparison code both screens show at pairing: SHA-256("tcr-pair-sas-v1" || D_pub || P_pub || E_pub || C), its
 * first 4 bytes big-endian, modulo 1,000,000, as six digits in two groups ("042 917"). Someone else pairing with the
 * same QR code has another P and E, so their code differs.
 */
export async function pairCode(dPub: Uint8Array, pPub: Uint8Array, ePub: Uint8Array, c: Uint8Array): Promise<string> {
  if (dPub.length !== 65 || pPub.length !== 65 || ePub.length !== 65 || c.length !== 16) {
    throw new Error('bad input for the comparison code');
  }
  const h = await sha256(concat(utf8(SAS_LABEL), dPub, pPub, ePub, c));
  const u32 = ((h[0] << 24) | (h[1] << 16) | (h[2] << 8) | h[3]) >>> 0;
  return groupCode(u32 % 1_000_000);
}
