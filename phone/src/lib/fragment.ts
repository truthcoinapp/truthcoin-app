// The pairing link's `#pair=…` fragment. The fragment never reaches a web server; the page reads it and removes it
// from the address bar before anything renders, so the one-time code doesn't linger in history, bookmarks or
// screenshots.

/** Read `#pair=…` from `loc` and strip it from the address bar at once. Null when there is none. */
export function takePairFragment(
  loc: { hash: string; pathname: string; search: string } = location,
  hist: Pick<History, 'replaceState' | 'state'> = history,
): string | null {
  if (!loc.hash.startsWith('#pair=')) return null;
  const v = loc.hash.slice('#pair='.length);
  hist.replaceState(hist.state, '', loc.pathname + loc.search);
  return v || null;
}

/**
 * The `#pair=` value in scanned or pasted text: a link with the fragment (any page: the value itself is checked
 * when it's read), or the bare value. Null for text that holds no pairing value, such as another QR code.
 */
export function pairValueFrom(text: string): string | null {
  const t = text.trim();
  const at = t.indexOf('#pair=');
  if (at < 0) return /^[A-Za-z0-9_-]{60,4000}$/.test(t) ? t : null;
  const before = t.slice(0, at);
  if (before && !/^https?:\/\/[^\s#]*$/i.test(before)) return null;
  const v = t.slice(at + '#pair='.length).split(/[\s&]/)[0];
  return /^[A-Za-z0-9_-]{1,4000}$/.test(v) ? v : null;
}
