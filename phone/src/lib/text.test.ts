import { describe, expect, it } from 'vitest';
import { cleanName, cleanText } from './text';

describe('device names (as the desktop cleans them)', () => {
  it('removes control and format characters, then trims', () => {
    expect(cleanName("  Mi‮ke's​ phone\n")).toBe("Mike's phone");
  });
  it('cuts to 40 characters (code points, not UTF-16 units)', () => {
    expect(cleanName('x'.repeat(50))).toBe('x'.repeat(40));
    expect(Array.from(cleanName('📱'.repeat(50))).length).toBe(40);
  });
  it('falls back to "Phone" when nothing is left', () => {
    expect(cleanName('​')).toBe('Phone');
    expect(cleanName('   ')).toBe('Phone');
  });
  it('removes every bidi control and invisible separator the desktop removes', () => {
    const hidden = '​‌‍‎‏‪‫‬‭‮⁠⁡⁢⁣⁤⁥⁦⁧⁨⁩﻿­؜';
    expect(cleanName(`a${hidden}b`)).toBe('ab');
    expect(cleanName('a\u0000b\u0007c\u007fd\u0085e')).toBe('abcde');
  });
});

describe('text from the desktop', () => {
  it('removes format characters and turns control characters into spaces', () => {
    expect(cleanText('Will‮ it\train?\n', 100)).toBe('Will it rain?');
    expect(cleanText('a​b', 100)).toBe('ab');
  });
  it('keeps line breaks in multi-line text', () => {
    expect(cleanText('one\ntwo\n\n\n\nthree\u0007', 100, true)).toBe('one\ntwo\n\nthree');
  });
  it('caps the length with an ellipsis', () => {
    expect(cleanText('abcdefghij', 5)).toBe('abcd…');
  });
  it('gives an empty string for anything not a string', () => {
    expect(cleanText(5, 10)).toBe('');
    expect(cleanText(null, 10)).toBe('');
  });
});
