// Nostr events (NIP-01) for the phone link: just what the link needs. An event's id is SHA-256 of
// `[0, pubkey, created_at, kind, tags, content]` serialized without spaces; its signature is BIP340 Schnorr
// (secp256k1) over the id. WebCrypto has no secp256k1, so @noble/curves signs and verifies. The Nostr keys only sign
// events: who sent a message is proven by the sealing inside (crypto.ts), never by the Nostr key alone.
import { schnorr } from '@noble/curves/secp256k1';
import { sha256 } from '@noble/hashes/sha256';
import { fromHex, hex, isNostrPub, own, randomBytes, utf8, type Bytes } from './bytes';

/** The link's event kind: ephemeral (relays pass it on and don't keep it), claimed by no NIP as of October 2026. */
export const KIND = 21913;
/** Events carry a NIP-40 expiration this far ahead, for relays that keep ephemeral events anyway. */
export const EXPIRES_SECS = 300;

export interface NostrEvent {
  id: string;
  pubkey: string;
  created_at: number;
  kind: number;
  tags: string[][];
  content: string;
  sig: string;
}

/** NIP-01's id: SHA-256 of the canonical JSON array. */
export function eventId(pubkey: string, createdAt: number, kind: number, tags: string[][], content: string): Bytes {
  return own(sha256(utf8(JSON.stringify([0, pubkey, createdAt, kind, tags, content]))));
}

/** A fresh Nostr secret key (32 bytes, a valid secp256k1 scalar). */
export function newNostrSecret(): Bytes {
  return own(schnorr.utils.randomPrivateKey());
}

/** The x-only public key, hex (how Nostr writes keys). */
export function nostrPub(secret: Uint8Array): string {
  return hex(schnorr.getPublicKey(secret));
}

/** A signed event; `aux` is the BIP340 auxiliary randomness (random in use, fixed in the test vectors). */
export function signEvent(
  secret: Uint8Array,
  createdAt: number,
  kind: number,
  tags: string[][],
  content: string,
  aux: Uint8Array = randomBytes(32),
): NostrEvent {
  const pubkey = nostrPub(secret);
  const id = eventId(pubkey, createdAt, kind, tags, content);
  const sig = schnorr.sign(id, secret, aux);
  return { id: hex(id), pubkey, created_at: createdAt, kind, tags, content, sig: hex(sig) };
}

/** An event of the link's kind to `to` (a Nostr pubkey), carrying `content`. */
export function messageEvent(secret: Uint8Array, to: string, content: string, now: number): NostrEvent {
  const tags = [
    ['p', to],
    ['expiration', String(now + EXPIRES_SECS)],
  ];
  return signEvent(secret, now, KIND, tags, content);
}

const HEX64 = /^[0-9a-f]{64}$/;
const HEX128 = /^[0-9a-f]{128}$/;

/** Is this a well-formed event whose id is right and whose signature is good? */
export function verifyEvent(e: unknown): e is NostrEvent {
  if (!e || typeof e !== 'object') return false;
  const v = e as Record<string, unknown>;
  if (typeof v.id !== 'string' || !HEX64.test(v.id)) return false;
  if (!isNostrPub(v.pubkey)) return false;
  if (typeof v.sig !== 'string' || !HEX128.test(v.sig)) return false;
  if (!Number.isSafeInteger(v.created_at) || (v.created_at as number) < 0) return false;
  if (!Number.isSafeInteger(v.kind) || (v.kind as number) < 0) return false;
  if (typeof v.content !== 'string') return false;
  if (!Array.isArray(v.tags) || !v.tags.every((t) => Array.isArray(t) && t.every((x) => typeof x === 'string'))) {
    return false;
  }
  const id = eventId(v.pubkey as string, v.created_at as number, v.kind as number, v.tags as string[][], v.content);
  if (hex(id) !== v.id) return false;
  try {
    return schnorr.verify(fromHex(v.sig), id, fromHex(v.pubkey as string));
  } catch {
    return false;
  }
}

/** The value of the first tag named `name`. */
export function tag(e: NostrEvent, name: string): string | undefined {
  const t = e.tags.find((t) => t[0] === name);
  return t?.[1];
}
