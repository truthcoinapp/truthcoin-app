// Pairing (PROTOCOL.md, "Pairing"): the desktop's QR code carries its keys, its relays and a one-time code C; the
// phone makes its keys and seals a pairing request with C. The desktop answers with a commitment nonce N, chosen after
// the phone's keys are fixed; both screens then show the comparison code over D, P, E, C and N. The desktop's sealed
// yes or no follows. Nothing is kept as a pairing until the person confirms, on the phone, that their own computer
// showed the code and that they allowed it there (a crafted link's "desktop" can say yes at once, but can't make the
// person's computer ask).
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
import { fromTo, KIND, messageEvent, newNostrSecret, nostrPub, tag, type NostrEvent } from './nostr';
import { RelayPool, type WsFactory } from './relaypool';
import { LOCAL_RELAYS, relayList } from './relays';
import { cleanName } from './text';
import { BadAnswerError, paired, status, UPDATE_THE_APP } from './validate';

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

/** A desktop issues codes for 5 minutes; a link claiming more than 10 (clock skew allowed for) isn't one of its. */
export const MAX_AHEAD_S = 600;
/** An attempt lives at most this long from when it was made, whatever its link's expiry says. */
export const ATTEMPT_LIFE_S = 300;
/** After that, a late answer may still be waited for this long. */
export const GRACE_S = 30;

const DAMAGED = 'This pairing code is damaged. On your computer, show a new one in the Phone tab, and scan it.';

const nowSecs = () => Math.floor(Date.now() / 1000);

/** Read the `#pair=` value. Throws with words for people on anything odd. */
export async function parsePairValue(value: string, allowLocal = LOCAL_RELAYS, nowS = nowSecs()): Promise<PairLink> {
  let o: Record<string, unknown>;
  try {
    if (value.length > 4000) throw new Error('too long');
    const parsed: unknown = JSON.parse(fromUtf8(fromB64u(value)));
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('not an object');
    o = parsed as Record<string, unknown>;
  } catch {
    throw new Error(DAMAGED);
  }
  if (typeof o.v === 'number' && o.v > 1) {
    // A newer app than this page (a page cached from before): the page is what needs updating.
    throw new Error('This page is older than your Truthcoin App: reload it, then scan the code again.');
  }
  if (o.v !== 1) {
    throw new Error(
      `This page and your Truthcoin App don't speak the same version: ${UPDATE_THE_APP}, then show a new code.`,
    );
  }
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
  if (o.x > nowS + MAX_AHEAD_S) {
    throw new Error(
      "This pairing code says it lasts far longer than the Truthcoin App's codes do, so it isn't one your computer made. Show a new code on your own computer and scan it.",
    );
  }
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
 * One pairing code's keys, kept (IndexedDB) while that pairing is under way: a retry, or carrying on after a reload,
 * pairs with the same P, nP and E, the same request id and name, so the desktop doesn't see a second phone (it refuses
 * a code that two phone keys opened) and gets the same N, and both screens show the same comparison code. Dropped on
 * every way out of pairing but unloading the page, and never resumed without asking.
 */
export interface PairAttempt {
  v: 1;
  link: PairLink;
  /** When it was made, unix seconds. */
  made: number;
  pPriv: CryptoKey;
  pPub: Bytes;
  ePriv: CryptoKey;
  ePub: Bytes;
  nsec: Bytes;
  npub: string;
  id: string;
  name: string;
  /** The desktop's commitment nonce N (b64u) and the comparison code, once N has come. */
  n?: string;
  code?: string;
}

/** Until when (unix seconds) an attempt may still be waited on: 5 minutes from when it was made at most, and never
 * past its code's expiry, plus the grace for a late answer. */
export function attemptUntil(a: Pick<PairAttempt, 'made' | 'link'>, graceS = GRACE_S): number {
  return Math.min(a.link.x, a.made + ATTEMPT_LIFE_S) + graceS;
}

/** A kept attempt answers this code only (same desktop, same code), and only while it lives. */
export function attemptFor(a: PairAttempt | null | undefined, link: PairLink, nowS: number, graceS = GRACE_S): PairAttempt | null {
  if (!a || a.v !== 1 || !a.pPriv || !a.ePriv || !(a.nsec instanceof Uint8Array) || !Number.isSafeInteger(a.made)) {
    return null;
  }
  if (a.link.c !== link.c || a.link.d !== link.d || a.link.n !== link.n) return null;
  return nowS < attemptUntil(a, graceS) ? a : null;
}

/** A new attempt for `link`: fresh keys (P and E non-extractable) and request id. */
export async function newAttempt(link: PairLink, name: string, nowS = nowSecs()): Promise<PairAttempt> {
  const p = await generateKey();
  const e = await newEphemeral();
  const nsec = newNostrSecret();
  return {
    v: 1,
    link,
    made: nowS,
    pPriv: p.privateKey,
    pPub: await exportPub(p.publicKey),
    ePriv: e.priv,
    ePub: e.pub,
    nsec,
    npub: nostrPub(nsec),
    id: newRequestId(),
    name: cleanName(name),
  };
}

/** The desktop said no (or the code was used up). */
export class PairRefusedError extends Error {}
/** The code's time is over. */
export class PairExpiredError extends Error {}
/** Nothing came back before the code ran out. */
export class PairNoAnswerError extends Error {}

export interface PairOptions {
  ws?: WsFactory;
  now?: () => number;
  /** The comparison code ("042 917"), once the desktop's nonce N has come. */
  onCode?: (code: string) => void;
  /** No answer for a while: the app may not be open on the computer. */
  onSlow?: () => void;
  slowAfterMs?: number;
  /** Wait at most this long after the attempt's time for a late answer. */
  graceMs?: number;
  /**
   * Send the request again at these times while no code has come, each time sealed afresh (a new event id, the same
   * P, E, id and name): relays may drop it, and the desktop's nonce message may be lost, and the same phone asking
   * again gets the same N again.
   */
  resendAt?: number[];
  /** No code after this long: the person must not allow anything on the computer yet. */
  onNoCode?: () => void;
  noCodeAfterMs?: number;
  /**
   * Also ask for `status` this often, sealed from P as a paired phone would. The desktop drops these until it allows
   * the phone, then answers them: so a lost pairing answer doesn't leave the phone waiting for nothing.
   */
  probeEveryMs?: number;
  /** Stop waiting when this turns true (the person left the screen). */
  cancelled?: () => boolean;
  /** This code's kept attempt, if any (reused: same keys, id and name). */
  attempt?: PairAttempt | null;
  /** Keep the attempt: before its request is first sent, and again when N comes. */
  keep?: (a: PairAttempt) => Promise<void> | void;
  /** How long a yes that came before N waits for N. */
  nonceWaitMs?: number;
}

/**
 * Pair with the desktop in `link`, as `name`. Resolves with the pairing once the desktop said yes; the caller keeps it
 * only after the person confirms. Rejects with PairRefusedError, PairExpiredError, PairNoAnswerError or another Error
 * with words for people.
 */
export async function pairPhone(link: PairLink, name: string, o: PairOptions = {}): Promise<Pairing> {
  const now = o.now ?? nowSecs;
  const graceS = Math.ceil((o.graceMs ?? GRACE_S * 1000) / 1000);
  let a = attemptFor(o.attempt, link, now(), graceS);
  if (!a && now() >= link.x) throw new PairExpiredError('This pairing code has expired.');
  const keep = async (x: PairAttempt) => {
    if (o.cancelled?.()) return; // a cancelled run keeps nothing (it may finish a step after the Cancel)
    try {
      await o.keep?.(x);
    } catch {
      // Storage refused (private browsing): this run still pairs; it just can't carry on after a reload.
    }
  };
  if (!a) {
    a = await newAttempt(link, name, now());
    await keep(a);
  }
  const att = a;
  const dPub = pubBytes(link.d);
  const c = fromB64u(link.c);
  const { pPriv, pPub, nsec, npub, id } = att;
  // Sealed afresh for every send (a new GCM nonce, so a new event id the desktop doesn't drop as a repeat); the same
  // P, E, id and name, so the same claim and, from the desktop, the same N.
  const pt = padJson({ t: 'pair', p: b64u(pPub), np: npub, name: att.name, id });
  const request = async (): Promise<NostrEvent> => {
    const { env } = await sealPair({ dPub, c, e: { priv: att.ePriv, pub: att.ePub } }, pt);
    return messageEvent(nsec, link.n, envelopeJson(env), now());
  };
  const first = await request();
  const keys: LinkKeys = { pPriv, pPub, dPub, nsec, npub, nd: link.n };
  if (att.code) o.onCode?.(att.code); // carrying on: N came before the reload

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
    // The yes can overtake N on the relays: then wait a moment for N, so the confirm step can show the code.
    let gotN: (() => void) | null = null;
    const nonceOrTimeout = () =>
      att.code
        ? Promise.resolve()
        : new Promise<void>((r) => {
            gotN = r;
            timers.push(setTimeout(r, o.nonceWaitMs ?? 3000));
          });
    const pool = new RelayPool({
      relays: link.r,
      ws: o.ws,
      filter: () => ({ kinds: [KIND], '#p': [npub], since: now() - 120 }),
      accept: fromTo(link.n, npub),
      onEvent: (got: NostrEvent) => {
        void answer(got).then(
          async (r) => {
            if (!r || over) return;
            await nonceOrTimeout();
            // A run cancelled meanwhile ends as cancelled, whatever came (the watch only looks every 250 ms).
            if (o.cancelled?.()) end(() => reject(new Error('cancelled')));
            else end(r);
          },
          () => undefined,
        );
      },
    });

    // The desktop's sealed answer to this request: what to end with, or null to keep waiting.
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
      if (!reply || reply.k === 'held' || reply.k === 'unsure') return null;
      if (reply.k === 'nonce') {
        // The first N for this request fixes the code; the desktop sends the same N to the same phone again.
        if (reply.re === id && !att.n && !over) {
          att.n = b64u(reply.nonce);
          att.code = await pairCode(dPub, pPub, att.ePub, c, reply.nonce);
          await keep(att);
          if (!over) o.onCode?.(att.code);
          (gotN as (() => void) | null)?.();
        }
        return null;
      }
      if (probes.has(reply.re)) {
        // The desktop answers only paired phones: any sealed answer to a probe means this phone was allowed.
        let st: ReturnType<typeof status> | null = null;
        try {
          st = reply.k === 'ok' ? status(reply.ok) : null;
        } catch {
          // an answer this page can't read still proves the pairing; Home asks for the status again
        }
        return () => resolve(pairingOf(st?.name ?? att.name, st?.limitSats ?? 0));
      }
      if (reply.re !== id) return null;
      if (reply.k === 'err') return () => reject(new PairRefusedError(reply.err));
      try {
        const yes = paired(reply.ok);
        return () => resolve(pairingOf(yes.name || att.name, yes.limitSats));
      } catch (err) {
        return () => reject(err instanceof BadAnswerError ? err : new Error(String(err)));
      }
    }

    function pairingOf(shown: string, limitSats: number): Pairing {
      return {
        v: 1,
        d: link.d,
        nd: link.n,
        relays: link.r,
        pPriv,
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
    void pool.publish(first);
    for (const at of o.resendAt ?? [3_000, 6_000, 10_000, 15_000, 30_000, 60_000, 120_000]) {
      timers.push(
        setTimeout(() => {
          if (over || att.code) return;
          void request().then((ev) => {
            if (!over) void pool.publish(ev);
          });
        }, at),
      );
    }
    timers.push(
      setTimeout(() => {
        if (!over && !att.code) o.onNoCode?.();
      }, o.noCodeAfterMs ?? 10_000),
    );
    timers.push(setTimeout(() => o.onSlow?.(), o.slowAfterMs ?? 25_000));
    probing = setInterval(() => void probe().catch(() => undefined), o.probeEveryMs ?? 5_000);
    const lastMs = Math.max(0, (attemptUntil(att, graceS) - now()) * 1000);
    timers.push(setTimeout(() => end(() => reject(new PairNoAnswerError('No answer from your computer.'))), lastMs));
    watch = setInterval(() => {
      if (o.cancelled?.()) end(() => reject(new Error('cancelled')));
    }, 250);
  });
}
