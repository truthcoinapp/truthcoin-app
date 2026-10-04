import { describe, expect, it } from 'vitest';
import { defaultDeviceName, fmtChance, fmtChanceFine, fmtNum, fmtSats, fmtShares, keyFingerprint, parseShares, stateWord } from './format';

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
  it('words', () => {
    expect(stateWord('trading')).toBe('Trading');
    expect(stateWord('some_state')).toBe('Some state');
    expect(defaultDeviceName('Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)')).toBe('iPhone');
    expect(defaultDeviceName('Mozilla/5.0 (Linux; Android 14)')).toBe('Android phone');
  });
});
