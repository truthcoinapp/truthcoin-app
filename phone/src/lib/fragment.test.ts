import { describe, expect, it, vi } from 'vitest';
import { pairValueFrom, takePairFragment } from './fragment';

describe('the #pair= fragment', () => {
  it('is read and stripped from the address bar at once', () => {
    const hist = { state: { x: 1 }, replaceState: vi.fn() };
    const v = takePairFragment({ hash: '#pair=abc_DEF-123', pathname: '/truthcoin-app/', search: '?q=1' }, hist);
    expect(v).toBe('abc_DEF-123');
    expect(hist.replaceState).toHaveBeenCalledWith({ x: 1 }, '', '/truthcoin-app/?q=1');
  });

  it('leaves other fragments and the address bar alone', () => {
    const hist = { state: null, replaceState: vi.fn() };
    expect(takePairFragment({ hash: '#other', pathname: '/', search: '' }, hist)).toBeNull();
    expect(takePairFragment({ hash: '', pathname: '/', search: '' }, hist)).toBeNull();
    expect(hist.replaceState).not.toHaveBeenCalled();
  });

  it('is stripped even when empty', () => {
    const hist = { state: null, replaceState: vi.fn() };
    expect(takePairFragment({ hash: '#pair=', pathname: '/', search: '' }, hist)).toBeNull();
    expect(hist.replaceState).toHaveBeenCalled();
  });
});

describe('pairing values in scanned or pasted text', () => {
  const value = 'eyJ2IjoxfQ' + 'A'.repeat(60);
  it('come from a whole link', () => {
    expect(pairValueFrom(`https://truthcoinapp.github.io/truthcoin-app/#pair=${value}`)).toBe(value);
    expect(pairValueFrom(`  http://127.0.0.1:4174/#pair=${value}\n`)).toBe(value);
  });
  it('or from the bare value', () => {
    expect(pairValueFrom(value)).toBe(value);
  });
  it('and not from other QR codes or text', () => {
    expect(pairValueFrom('https://example.com/')).toBeNull();
    expect(pairValueFrom('hello')).toBeNull();
    expect(pairValueFrom('javascript:alert(1)#pair=abc')).toBeNull();
    expect(pairValueFrom('https://x/#pair=<script>')).toBeNull();
  });
});
