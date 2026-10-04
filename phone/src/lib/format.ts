// Numbers and words as the page shows them. Amounts are sats; a share pays 1 sat if its outcome happens.
import { sha256 } from '@noble/hashes/sha256';
import { fromB64u, hex } from './bytes';

function group(n: number): string {
  return String(n).replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

/** 52250 -> "52,250 sats"; 1 -> "1 sat". Fractions are rounded. */
export function fmtSats(n: number): string {
  if (!Number.isFinite(n)) return '—';
  const r = Math.round(n);
  return `${r < 0 ? '-' : ''}${group(Math.abs(r))} ${Math.abs(r) === 1 ? 'sat' : 'sats'}`;
}

/** A whole number with thousands separators: 1000 -> "1,000". */
export function fmtNum(n: number): string {
  return Number.isFinite(n) ? `${n < 0 ? '-' : ''}${group(Math.abs(Math.round(n)))}` : '—';
}

/** 100000 -> "100,000"; 12.345 -> "12.35". */
export function fmtShares(n: number): string {
  if (!Number.isFinite(n)) return '—';
  if (Number.isInteger(n)) return group(n);
  const [i, f] = n.toFixed(2).split('.');
  return `${group(Number(i))}.${f}`;
}

/** A probability as a chance: 0.625 -> "63%"; tiny but not zero -> "<1%"; nearly sure -> ">99%". */
export function fmtChance(p: number): string {
  if (!Number.isFinite(p)) return '—';
  if (p > 0 && p < 0.005) return '<1%';
  if (p < 1 && p > 0.995) return '>99%';
  return `${Math.round(p * 100)}%`;
}

/** A probability with one decimal, for showing how a trade moves it: 0.525 -> "52.5%". */
export function fmtChanceFine(p: number): string {
  if (!Number.isFinite(p)) return '—';
  return `${(Math.round(p * 1000) / 10).toFixed(1)}%`;
}

export function fmtHeight(n: number): string {
  return group(n);
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** "5 Oct 2026": the same everywhere (10/5/2026 reads as 10 May in most places). */
export function fmtDate(d: Date): string {
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}`;
}

/** Today: "14:05" (the phone's own clock style); another day: "5 Oct 2026". */
export function fmtTime(unix: number): string {
  if (!unix) return '';
  const d = new Date(unix * 1000);
  const now = new Date();
  return d.toDateString() === now.toDateString()
    ? d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })
    : fmtDate(d);
}

/** How the phone's daily limit works, in one place for every screen that explains it. */
export function limitSentence(limitSats: number): string {
  return `Up to ${fmtSats(limitSats)} of trades a day go through by themselves; bigger ones wait for your OK on the computer. A buy counts at its most; a sell at its number of shares.`;
}

/** The miner fee every trade pays from the wallet's coin, on top of a buy's price and out of a sell's proceeds. */
export const MINER_FEE_SATS = 1000;

/** What a share of a settled market's outcome paid: "1 sat", "0.5 sat", or "nothing". */
export function fmtPaid(p: number): string {
  if (!Number.isFinite(p) || p <= 0) return 'nothing';
  const r = Math.round(p * 1000) / 1000;
  return `${r} sat`;
}

/** "a1b2c3…d4e5f6" for long ids. */
export function shortId(s: string, n = 8): string {
  return s.length > n * 2 + 1 ? `${s.slice(0, n)}…${s.slice(-n)}` : s;
}

/** A short fingerprint of the desktop's key D: the first 6 bytes of SHA-256(D_pub), in three groups. */
export function keyFingerprint(dB64u: string): string {
  try {
    const h = hex(sha256(fromB64u(dB64u))).slice(0, 12);
    return `${h.slice(0, 4)} ${h.slice(4, 8)} ${h.slice(8, 12)}`;
  } catch {
    return '—';
  }
}

/** A market's state in words. */
export function stateWord(s: string): string {
  switch (s) {
    case 'trading':
      return 'Trading';
    case 'settled':
      return 'Settled';
    case 'cancelled':
      return 'Cancelled';
    case 'invalid':
      return 'Invalid';
    case 'voting':
      return 'Being decided';
    default:
      return s ? s[0].toUpperCase() + s.slice(1).replace(/_/g, ' ') : '';
  }
}

/** Parse a number of shares as typed: a whole number, commas or spaces allowed. null when it isn't one. */
export function parseShares(input: string): number | null {
  const s = input.trim().replace(/[,\s_]/g, '');
  if (!/^\d{1,13}$/.test(s)) return null;
  const n = Number(s);
  return n > 0 && Number.isSafeInteger(n) ? n : null;
}

export function defaultDeviceName(ua = typeof navigator !== 'undefined' ? navigator.userAgent : ''): string {
  if (/Android/i.test(ua)) return 'Android phone';
  if (/iPad/i.test(ua)) return 'iPad';
  if (/iPhone/i.test(ua)) return 'iPhone';
  return 'Phone';
}
