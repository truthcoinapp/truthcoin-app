// Amounts: sats everywhere (a share pays 1 sat if its outcome wins), with thousands separators.

export function sats(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return "—";
  return `${Math.round(n).toLocaleString("en-US")} sats`;
}

export function num(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return "—";
  return Math.round(n).toLocaleString("en-US");
}

/** A price (0..1) as a chance: "62%", "0.4%", "<0.1%". */
export function chance(p: number | null | undefined): string {
  if (p === null || p === undefined || !Number.isFinite(p)) return "—";
  const v = p * 100;
  if (v > 0 && v < 0.1) return "<0.1%";
  if (v < 10 && v !== 0) return `${v.toFixed(1)}%`;
  return `${Math.round(v)}%`;
}

/** A whole number from a text field ("1,000" or "1000"), or NaN. */
export function parseWhole(s: string): number {
  const t = s.replace(/[,\s_]/g, "");
  return /^\d+$/.test(t) ? Number(t) : NaN;
}

/** A size on disk: "812 KB", "1.4 GB". */
export function bytes(n: number): string {
  if (n < 1000) return `${n} bytes`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1000;
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

export function when(unix: number): string {
  if (!unix) return "";
  const d = new Date(unix * 1000);
  return d.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

export function short(s: string, n = 10): string {
  return s.length > n * 2 + 1 ? `${s.slice(0, n)}…${s.slice(-n)}` : s;
}

export const STATUS_WORDS: Record<string, string> = {
  sending: "May have gone: check again",
  pending: "Waiting for the next block",
  done: "Done",
  failed: "Refused by the node",
  cancelled: "Cancelled",
  dropped: "Dropped by the node",
};
