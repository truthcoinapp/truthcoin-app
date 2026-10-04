// Relay addresses: wss:// only (PROTOCOL.md, "Pairing"), at most 5. A local build (`npm run build:local`) and the
// tests also take ws://127.0.0.1 and ws://localhost, for the test relay in dev/test-relay.mjs.

declare const __LOCAL_RELAYS__: boolean;
export const LOCAL_RELAYS: boolean = typeof __LOCAL_RELAYS__ !== 'undefined' && __LOCAL_RELAYS__;

export const MAX_RELAYS = 5;

/** The relay address as given when it is one the page may use, else null. */
export function validRelay(u: unknown, allowLocal = LOCAL_RELAYS): string | null {
  if (typeof u !== 'string' || u.length > 200 || !/^[\x21-\x7e]+$/.test(u)) return null;
  let url: URL;
  try {
    url = new URL(u);
  } catch {
    return null;
  }
  if (url.username || url.password || url.hash) return null;
  if (url.protocol === 'wss:' && url.hostname) return u;
  if (allowLocal && url.protocol === 'ws:' && (url.hostname === '127.0.0.1' || url.hostname === 'localhost')) return u;
  return null;
}

/** A list of 1 to 5 usable relays (duplicates removed), or null when any entry isn't one. */
export function relayList(v: unknown, allowLocal = LOCAL_RELAYS): string[] | null {
  if (!Array.isArray(v) || v.length === 0 || v.length > MAX_RELAYS) return null;
  const out: string[] = [];
  for (const u of v) {
    const r = validRelay(u, allowLocal);
    if (!r) return null;
    if (!out.includes(r)) out.push(r);
  }
  return out;
}

/** "wss://relay.damus.io" -> "relay.damus.io", for showing. */
export function relayHost(u: string): string {
  try {
    return new URL(u).host;
  } catch {
    return u;
  }
}
