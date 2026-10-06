import { describe, expect, it } from 'vitest';
import { answered, clock, NO_REACH, noAnswer, QUIET_MS, reachBanner, reachState, relaysBack } from './reach';

const T = new Date(2026, 9, 7, 10, 42).getTime();

describe('reach', () => {
  it('is unknown until the computer answers, then answering', () => {
    expect(reachState(NO_REACH, true, T)).toBe('unknown');
    expect(reachState(answered(NO_REACH, T), true, T)).toBe('answering');
  });

  it('is never "connected" with no relay open', () => {
    const a = answered(NO_REACH, T);
    expect(reachState(a, false, T)).toBe('connecting');
    expect(reachState(NO_REACH, false, T)).toBe('connecting');
    // A request found no relay at all: this phone is offline.
    expect(reachState(noAnswer(a, T + 1000, 0), false, T + 1000)).toBe('offline');
  });

  it('wants a fresh answer once a relay comes back', () => {
    const a = relaysBack(answered(NO_REACH, T), T + 60_000);
    expect(reachState(a, true, T + 60_000)).toBe('unknown');
    expect(reachState(answered(a, T + 61_000), true, T + 61_000)).toBe('answering');
  });

  it('goes silent when a relay took a request and no answer came, and back on a newer answer', () => {
    const a = answered(NO_REACH, T);
    const s = noAnswer(a, T + 60_000, 2);
    expect(reachState(s, true, T + 60_000)).toBe('silent');
    expect(s.silentSince).toBe(T + 60_000);
    expect(s.lastAnswer).toBe(T);
    // Later misses keep the first time it went silent.
    expect(noAnswer(s, T + 120_000, 1).silentSince).toBe(T + 60_000);
    const back = answered(s, T + 180_000);
    expect(reachState(back, true, T + 180_000)).toBe('answering');
    expect(back.silentSince).toBeNull();
  });

  it("an old reply delivered late (or replayed by a relay) doesn't end a silence", () => {
    const s = noAnswer(answered(NO_REACH, T), T + 60_000, 1);
    const late = answered(s, T + 30_000); // sent before the silence began
    expect(reachState(late, true, T + 90_000)).toBe('silent');
    expect(late.lastAnswer).toBe(T + 30_000);
  });

  it('gives the time it last answered when that was a while ago', () => {
    const a = answered(NO_REACH, T);
    expect(reachState(a, true, T + QUIET_MS - 1000)).toBe('answering');
    expect(reachState(a, true, T + QUIET_MS + 1000)).toBe('quiet');
  });

  it('banners say what to do and how old the figures are', () => {
    expect(reachBanner(answered(NO_REACH, T), true, T)).toBeNull();
    const b = reachBanner(noAnswer(answered(NO_REACH, T), T + 60_000, 1), true, T + 90_000);
    expect(b?.title).toContain("isn't answering");
    expect(b?.body).toContain('The Truthcoin App has to be open on your computer');
    expect(b?.body).toContain(`from ${clock(T, T + 90_000)}`);
    const never = reachBanner(noAnswer(NO_REACH, T, 1), true, T);
    expect(never?.body).toContain('Nothing has come from it yet');
    expect(reachBanner(noAnswer(NO_REACH, T, 0), false, T)?.title).toContain("can't reach the relays");
    // Merely reconnecting: no banner.
    expect(reachBanner(answered(NO_REACH, T), false, T)).toBeNull();
  });

  it('dates older times', () => {
    expect(clock(T, T)).not.toMatch(/yesterday/);
    expect(clock(T - 86_400_000, T)).toMatch(/^yesterday /);
    expect(clock(T - 5 * 86_400_000, T)).toMatch(/Oct/);
  });
});
