// Pairing (PROTOCOL.md, "Pairing"): the desktop's QR code carries its keys, its relays and a one-time code C; the
// phone makes its keys, seals a pairing request with C, shows the comparison code, and waits for the desktop's
// sealed yes or no.
import { b64u, fromB64u, fromUtf8, isNostrPub, newRequestId, type Bytes } from './bytes';
import {
  envelopeJson,
  exportPub,
  generateKey,
  importPub,
  newEphemeral,
  openMsg,
  padJson,
  pairCode,
  parseEnvelope,
  pubBytes,
  sealMsg,
  sealPair,
  unpadJson,
} from './crypto';
import { parseReply, type LinkKeys } from './link';
import { KIND, messageEvent, newNostrSecret, nostrPub, tag, type NostrEvent } from './nostr';
import { RelayPool, type WsFactory } from './relaypool';
import { LOCAL_RELAYS, relayList } from './relays';
import { cleanName } from './text';
import { BadAnswerError, paired, status } from './validate';

/** What the QR code says. */
export interface PairLink {
  v: 1;
  /** Relays, 1 to 5, wss:// only. */
  r: string[];
  /** The desktop's Nostr key nD, hex. */
  n: string;
  /** The desktop's static key D, b64u. */
  d: string;
  /** The one-time code C, b64u of 16 bytes. */
  c: string;
  /** Expiry, unix seconds. */
  x: number;
}

const DAMAGED = 'This pairing code is damaged. On your computer, open Settings › Phone for a new one, and scan it.';

/** Read the `#pair=` value. Throws with words for people on anything odd. */
export async function parsePairValue(value: string, allowLocal = LOCAL_RELAYS): Promise<PairLink> {
  let o: Record<string, unknown>;
  try {
    if (value.length > 4000) throw new Error('too long');
    const parsed: unknown = JSON.parse(fromUtf8(fromB64u(value)));
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('not an object');
    o = parsed as Record<string, unknown>;
  } catch {
    throw new Error(DAMAGED);
  }
  if (o.v !== 1) throw new Error('This pairing code is from another version of the app. Update the app on your computer, or reload this page.');
  const r = relayList(o.r, allowLocal);
  if (!r) throw new Error('This pairing code has no usable relay addresses (they must start with wss://).');
  if (!isNostrPub(o.n)) throw new Error(DAMAGED);
  if (typeof o.d !== 'string') throw new Error(DAMAGED);
  try {
    await importPub(pubBytes(o.d)); // on the curve
  } catch {
    throw new Error(DAMAGED);
  }
  let c: Bytes;
  try {
    c = fromB64u(String(o.c ?? ''));
  } catch {
    throw new Error(DAMAGED);
  }
  if (c.length !== 16) throw new Error(DAMAGED);
  if (typeof o.x !== 'number' || !Number.isSafeInteger(o.x) || o.x <= 0) throw new Error(DAMAGED);
  return { v: 1, r, n: o.n, d: o.d, c: o.c as string, x: o.x };
}

/** The pairing this phone keeps (IndexedDB; the device key is a non-extractable CryptoKey). */
export interface Pairing {
  v: 1;
  /** The desktop's D (b64u) and nD (hex). */
  d: string;
  nd: string;
  relays: string[];
  pPriv: CryptoKey;
  pPub: string;
  /** The phone's Nostr secret nP (raw bytes) and public key. */
  nsec: Bytes;
  npub: string;
  /** This phone's name as the desktop shows it. */
  name: string;
  limitSats: number;
  pairedAt: number;
}

export function linkKeys(p: Pairing): LinkKeys {
  return { pPriv: p.pPriv, pPub: pubBytes(p.pPub), dPub: pubBytes(p.d), nsec: p.nsec, npub: p.npub, nd: p.nd };
}

/**
 * One pairing code's keys, kept (IndexedDB) from the moment the phone first answers that code: a retry or a reload
 * pairs with the same P, nP and E, the same request id and name, so the desktop doesn't see a second phone (it refuses
 * a code that two phone keys opened) and both screens keep showing the same comparison code.
 */
export interface PairAttempt {
  v: 1;
  link: PairLink;
  pPriv: CryptoKey;
  pPub: Bytes;
  ePriv: CryptoKey;
  ePub: Bytes;
  nsec: Bytes;
  npub: string;
  id: string;
  name: string;
  code: string;
}

/** A kept attempt answers this code only: same desktop, same code, and not yet past its expiry and grace. */
export function attemptFor(a: PairAttempt | null | undefined, link: PairLink, nowS: number, graceS = 30): PairAttempt | null {
  if (!a || a.v !== 1 || !a.pPriv || !a.ePriv || !(a.nsec instanceof Uint8Array)) return null;
  if (a.link.c !== link.c || a.link.d !== link.d || a.link.n !== link.n) return null;
  return nowS < link.x + graceS ? a : null;
}

/** A new attempt for `link`: fresh keys (P and E non-extractable), request id, the comparison code. */
export async function newAttempt(link: PairLink, name: string): Promise<PairAttempt> {
  const dPub = pubBytes(link.d);
  const p = await generateKey();
  const e = await newEphemeral();
  const pPub = await exportPub(p.publicKey);
  const nsec = newNostrSecret();
  return {
    v: 1,
    link,
    pPriv: p.privateKey,
    pPub,
    ePriv: e.priv,
    ePub: e.pub,
    nsec,
    npub: nostrPub(nsec),
    id: newRequestId(),
    name: cleanName(name),
    code: await pairCode(dPub, pPub, e.pub, fromB64u(link.c)),
  };
}

/** The desktop said no (or the code was used up). */
export class PairRefusedError extends Error {}
/** The code's 5 minutes are over. */
export class PairExpiredError extends Error {}
/** Nothing came back before the code ran out. */
export class PairNoAnswerError extends Error {}

export interface PairOptions {
  ws?: WsFactory;
  now?: () => number;
  /** The comparison code ("042 917"), as soon as the request is ready. */
  onCode?: (code: string) => void;
  /** No answer for a while: the app may not be open on the computer. */
  onSlow?: () => void;
  slowAfterMs?: number;
  /** Wait at most this long after the code's expiry for a late answer. */
  graceMs?: number;
  /** Send the same signed request again at these times (relays may drop it; the desktop drops repeats). */
  resendAt?: number[];
  /**
   * Also ask for `status` this often, sealed from P as a paired phone would. The desktop drops these until it allows
   * the phone, then answers them: so a lost pairing answer doesn't leave the phone waiting for nothing.
   */
  probeEveryMs?: number;
  /** Stop waiting when this turns true (the person left the screen). */
  cancelled?: () => boolean;
  /** This code's kept attempt, if any (reused: same keys, id and name). */
  attempt?: PairAttempt | null;
  /** Keep a new attempt before its request is first sent. */
  keep?: (a: PairAttempt) => Promise<void> | void;
}

/**
 * Pair with the desktop in `link`, as `name`. Resolves with the pairing to keep, or rejects with PairRefusedError,
 * PairExpiredError, PairNoAnswerError or another Error with words for people.
 */
export async function pairPhone(link: PairLink, name: string, o: PairOptions = {}): Promise<Pairing> {
  const now = o.now ?? (() => Math.floor(Date.now() / 1000));
  const graceS = Math.ceil((o.graceMs ?? 30_000) / 1000);
  // A kept attempt may still hear its answer after the expiry (within the grace); a new one may not start.
  let a = attemptFor(o.attempt, link, now(), graceS);
  if (!a && now() >= link.x) throw new PairExpiredError('This pairing code has expired.');
  if (a && now() >= link.x + graceS) throw new PairExpiredError('This pairing code has expired.');
  if (!a) {
    a = await newAttempt(link, name);
    try {
      await o.keep?.(a);
    } catch {
      // Storage refused (private browsing): this run still pairs; a reload would start afresh.
    }
  }
  const dPub = pubBytes(link.d);
  const c = fromB64u(link.c);
  const { pPriv, pPub, nsec, npub, id } = a;
  const shownName = a.name;
  // Sealed afresh each run (a new nonce, so a new event the relays pass on); the same P, E and id, so the same claim.
  const pt = padJson({ t: 'pair', p: b64u(pPub), np: npub, name: shownName, id });
  const { env } = await sealPair({ dPub, c, e: { priv: a.ePriv, pub: a.ePub } }, pt);
  o.onCode?.(a.code);
  const ev = messageEvent(nsec, link.n, envelopeJson(env), now());
  const keys: LinkKeys = { pPriv, pPub, dPub, nsec, npub, nd: link.n };

  return new Promise<Pairing>((resolve, reject) => {
    const timers: ReturnType<typeof setTimeout>[] = [];
    const probes = new Set<string>();
    let watch: ReturnType<typeof setInterval> | undefined;
    let probing: ReturnType<typeof setInterval> | undefined;
    let over = false;
    const end = (fn: () => void) => {
      if (over) return;
      over = true;
      for (const t of timers) clearTimeout(t);
      clearInterval(watch);
      clearInterval(probing);
      pool.stop();
      fn();
    };
    const pool = new RelayPool({
      relays: link.r,
      ws: o.ws,
      filter: () => ({ kinds: [KIND], '#p': [npub], since: now() - 120 }),
      onEvent: (got: NostrEvent) => {
        void answer(got).then(
          (r) => r && end(r),
          () => undefined,
        );
      },
    });

    // The desktop's sealed answer to this request, or null for anything else.
    async function answer(got: NostrEvent): Promise<(() => void) | null> {
      if (got.pubkey !== link.n || got.kind !== KIND || tag(got, 'p') !== npub) return null;
      const env = parseEnvelope(got.content);
      if (!env || env.k !== 'msg') return null;
      let reply;
      try {
        reply = parseReply(unpadJson(await openMsg({ rPriv: keys.pPriv, rPub: pPub, sPub: dPub }, env)));
      } catch {
        return null;
      }
      if (!reply || reply.k === 'held') return null;
      if (probes.has(reply.re)) {
        // The desktop answers only paired phones: any sealed answer to a probe means this phone was allowed.
        let st: ReturnType<typeof status> | null = null;
        try {
          st = reply.k === 'ok' ? status(reply.ok) : null;
        } catch {
          // an answer this page can't read still proves the pairing; Home asks for the status again
        }
        return () => resolve(keep(st?.name ?? shownName, st?.limitSats ?? 0));
      }
      if (reply.re !== id || (reply.k !== 'ok' && reply.k !== 'err')) return null;
      if (reply.k === 'err') return () => reject(new PairRefusedError(reply.err));
      try {
        const yes = paired(reply.ok);
        return () => resolve(keep(yes.name || shownName, yes.limitSats));
      } catch (err) {
        return () => reject(err instanceof BadAnswerError ? err : new Error(String(err)));
      }
    }

    function keep(shown: string, limitSats: number): Pairing {
      return {
        v: 1,
        d: link.d,
        nd: link.n,
        relays: link.r,
        pPriv: pPriv,
        pPub: b64u(pPub),
        nsec,
        npub,
        name: cleanName(shown),
        limitSats,
        pairedAt: Date.now(),
      };
    }

    async function probe() {
      const pid = newRequestId();
      probes.add(pid);
      const env = await sealMsg({ sPriv: pPriv, sPub: pPub, rPub: dPub }, padJson({ id: pid, ts: now(), m: 'status', a: {} }));
      if (!over) void pool.publish(messageEvent(nsec, link.n, envelopeJson(env), now()));
    }

    pool.start();
    void pool.publish(ev);
    for (const at of o.resendAt ?? [3_000, 10_000, 30_000, 60_000, 120_000]) {
      timers.push(setTimeout(() => void pool.publish(ev), at));
    }
    timers.push(setTimeout(() => o.onSlow?.(), o.slowAfterMs ?? 25_000));
    probing = setInterval(() => void probe().catch(() => undefined), o.probeEveryMs ?? 5_000);
    const lastMs = Math.max(0, (link.x - now()) * 1000) + (o.graceMs ?? 30_000);
    timers.push(setTimeout(() => end(() => reject(new PairNoAnswerError('No answer from your computer.'))), lastMs));
    watch = setInterval(() => {
      if (o.cancelled?.()) end(() => reject(new Error('cancelled')));
    }, 250);
  });
}
