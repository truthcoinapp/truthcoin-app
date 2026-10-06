// Is the computer answering? The relays being up says only that this phone reaches them: the computer may be off,
// asleep, or the Truthcoin App closed there (operator 2026-10-07: "i shutdown on the computer.. phone app. did not
// realise it could not sync"). So the page keeps when the computer last answered and since when it hasn't, and says
// so in the header, in a banner, and on Home's figures.

export interface Reach {
  /** When the computer last answered anything (ms, when it sent the answer), or null if it hasn't since the page opened. */
  lastAnswer: number | null;
  /** When a relay last came back after none was open (ms): an answer from before then doesn't say it's there now. */
  relaysBackAt: number;
  /** Since when a request went out through a relay and no answer came (ms); null once it answers. */
  silentSince: number | null;
  /** No relay took the last request: this phone (or the relays) is offline, so the computer can't be asked. */
  relaysDown: boolean;
}

export const NO_REACH: Reach = { lastAnswer: null, relaysBackAt: 0, silentSince: null, relaysDown: false };

/** An answer from the computer (a reply, a refusal, "busy" or "held" all count), sent at `at`. One sent before the
 * silence began (a relay delivering an old reply late, or replaying one) doesn't end it. */
export function answered(r: Reach, at: number): Reach {
  const lastAnswer = Math.max(at, r.lastAnswer ?? 0);
  const silentSince = r.silentSince !== null && at < r.silentSince ? r.silentSince : null;
  return { ...r, lastAnswer, silentSince, relaysDown: false };
}

/** A relay is open again after none was. */
export function relaysBack(r: Reach, now: number): Reach {
  return { ...r, relaysBackAt: now };
}

/** A request got no answer. `accepted`: how many relays took it (none: the phone or the relays are offline). */
export function noAnswer(r: Reach, now: number, accepted: number): Reach {
  if (!accepted) return { ...r, relaysDown: true };
  return { ...r, silentSince: r.silentSince ?? now, relaysDown: false };
}

export type ReachState = 'connecting' | 'offline' | 'silent' | 'unknown' | 'answering' | 'quiet';

/** After this long without hearing from it, the header gives the time it last answered rather than "connected". */
export const QUIET_MS = 5 * 60_000;

/**
 * - connecting: no relay is open (yet, or for now);
 * - offline: no relay is open, and a request went unanswered for want of one;
 * - silent: requests went out and the computer didn't answer;
 * - unknown: nothing has come back since the page opened or a relay came back;
 * - answering: it answered lately;
 * - quiet: it answered, but not for a while (nothing has been asked lately).
 */
export function reachState(r: Reach, relaysOpen: boolean, now = Date.now()): ReachState {
  if (!relaysOpen) return r.relaysDown ? 'offline' : 'connecting';
  if (r.silentSince !== null) return 'silent';
  if (r.lastAnswer === null || r.lastAnswer < r.relaysBackAt) return 'unknown';
  return now - r.lastAnswer > QUIET_MS ? 'quiet' : 'answering';
}

/** "10:42", or "yesterday 10:42" / a date for older times. */
export function clock(at: number, now: number): string {
  const d = new Date(at);
  const t = d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((day(new Date(now)) - day(d)) / 86_400_000);
  if (days <= 0) return t;
  if (days === 1) return `yesterday ${t}`;
  return `${d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })} ${t}`;
}

/** The banner's words while the computer isn't answering, or null when there's nothing to say. */
export function reachBanner(r: Reach, relaysOpen: boolean, now: number): { title: string; body: string } | null {
  const s = reachState(r, relaysOpen, now);
  const shown = r.lastAnswer !== null ? `What you see here is from ${clock(r.lastAnswer, now)}.` : 'Nothing has come from it yet.';
  if (s === 'silent') {
    return {
      title: `Your computer isn't answering (since ${clock(r.silentSince ?? now, now)})`,
      body: `The Truthcoin App has to be open on your computer, and the computer awake and online, for this phone to reach it. ${shown}`,
    };
  }
  if (s === 'offline') {
    return {
      title: "This phone can't reach the relays",
      body: `Check this phone's connection. Until then it can't reach your computer. ${shown}`,
    };
  }
  return null;
}
