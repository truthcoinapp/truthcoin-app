import { describe, expect, it } from 'vitest';
import {
  defaultDeviceName,
  fmtChance,
  fmtChanceFine,
  fmtDate,
  fmtNum,
  fmtPaid,
  fmtSats,
  fmtShares,
  keyFingerprint,
  limitSentence,
  parseShares,
  settledLine,
  stateWord,
  votingWords,
} from './format';

describe('formatting', () => {
  it('sats', () => {
    expect(fmtSats(52250)).toBe('52,250 sats');
    expect(fmtSats(1)).toBe('1 sat');
    expect(fmtSats(0)).toBe('0 sats');
    expect(fmtSats(2.6)).toBe('3 sats');
    expect(fmtSats(NaN)).toBe('—');
    expect(fmtNum(1234567)).toBe('1,234,567');
  });
  it('shares', () => {
    expect(fmtShares(100000)).toBe('100,000');
    expect(fmtShares(1234.567)).toBe('1,234.57');
  });
  it('chances', () => {
    expect(fmtChance(0.525)).toBe('53%');
    expect(fmtChance(0.001)).toBe('<1%');
    expect(fmtChance(0.999)).toBe('>99%');
    expect(fmtChance(0)).toBe('0%');
    expect(fmtChance(1)).toBe('100%');
    expect(fmtChanceFine(0.525)).toBe('52.5%');
  });
  it('typed shares: whole numbers only', () => {
    expect(parseShares('2,000')).toBe(2000);
    expect(parseShares(' 100 000 ')).toBe(100000);
    for (const bad of ['', '0', '-5', '1.5', '1e5', 'abc', '99999999999999']) expect(parseShares(bad)).toBeNull();
  });
  it('the desktop key fingerprint', () => {
    expect(keyFingerprint('BAIX5hfwtkQ5KCePlpmeaaI6TywVK99tbN9m5bgCgtTtGUp968uXcS0t2jyoWqh2Wlb0X8dYWZZS8ol8ZTBuV5Q')).toMatch(
      /^[0-9a-f]{4} [0-9a-f]{4} [0-9a-f]{4}$/,
    );
    expect(keyFingerprint('!!')).toBe('—');
  });
  it('dates as "5 Oct 2026", payouts per share, the limit sentence', () => {
    expect(fmtDate(new Date(2026, 9, 5))).toBe('5 Oct 2026');
    expect(fmtPaid(1)).toBe('1 sat');
    expect(fmtPaid(0.5)).toBe('0.5 sat');
    expect(fmtPaid(0)).toBe('nothing');
    expect(limitSentence(100000)).toBe(
      'Up to 100,000 sats of trades a day go through by themselves; bigger ones wait for your OK on the computer. A buy counts at its most; a sell at its number of shares.',
    );
  });

  it('when voters decide, and what a settled market paid', () => {
    expect(votingWords(3, 1, 10, false)).toBe('Voters decide in period 3 (in about 20 blocks)');
    expect(votingWords(3, 1, null, false)).toBe('Voters decide in period 3 (about 2 quarters from now)');
    expect(votingWords(3, 2, null, false)).toBe('Voters decide in period 3 (about 1 quarter from now)');
    expect(votingWords(3, 3, null, false)).toBe('Voters decide in period 3 (now)');
    expect(votingWords(3, 5, 10, true)).toBe('Decided in period 3');
    expect(
      settledLine({
        winners: ['Yes'],
        paid: 50000,
        outcomes: [
          { label: 'Yes', shares: 50000, perShare: 1 },
          { label: 'No', shares: 20000, perShare: 0 },
        ],
      }),
    ).toBe('Settled: Yes · You got 50,000 sats (50,000 Yes shares; 20,000 No shares paid nothing)');
    expect(
      settledLine({ winners: ['No', 'Yes'], paid: 5000, outcomes: [{ label: 'Yes', shares: 10000, perShare: 0.5 }] }),
    ).toBe('Settled: No, Yes · You got 5,000 sats (10,000 Yes shares at 0.5 sat each)');
  });

  it('words', () => {
    expect(stateWord('trading')).toBe('Trading');
    expect(stateWord('some_state')).toBe('Some state');
    expect(defaultDeviceName('Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)')).toBe('iPhone');
    expect(defaultDeviceName('Mozilla/5.0 (Linux; Android 14)')).toBe('Android phone');
  });
});
