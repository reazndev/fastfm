import { DemoOutcome } from '../../api/models';
import { barFraction, formatMs, scaleFor, speedup, totalRow } from './scale';

const scenarios = ['latest', 'day', 'profile'].map((id) => ({
  id,
  title: id,
  detail: id,
  requests: 1,
}));

function outcome(id: string, identical = true): DemoOutcome {
  return {
    id,
    requests: 1,
    lastfm: { ms: 300, bytes: 2000, ok: true },
    mirror: { firstMs: 1.5, cachedMs: 0.2, bytes: 2000, ok: true },
    mirrorSource: 'local',
    identical,
  };
}

describe('demo scale', () => {
  it('draws times on a shared log scale that keeps tiny bars visible', () => {
    const scale = scaleFor([outcome('a')]);
    expect(scale).toEqual({ min: 0.01, max: 300 });
    expect(barFraction(300, scale)).toBe(1);
    expect(barFraction(0.001, scale)).toBe(0.02);
    expect(barFraction(1.5, scale)).toBeGreaterThan(barFraction(0.2, scale));
    expect(barFraction(1.5, scale)).toBeLessThan(0.6);
  });

  it('formats times and speed-ups for reading at a glance', () => {
    expect(formatMs(312.4)).toBe('312 ms');
    expect(formatMs(4.24)).toBe('4.2 ms');
    expect(formatMs(0.253)).toBe('0.25 ms');
    expect(speedup(300, 1.5)).toBe('×200');
    expect(speedup(3, 1.2)).toBe('×2.5');
  });

  it('adds up finished scenarios into a total row', () => {
    const rows = [
      { scenario: scenarios[0], outcome: outcome('latest') },
      { scenario: scenarios[1], outcome: outcome('day', false) },
      { scenario: scenarios[2], outcome: null },
    ];
    const total = totalRow(rows);
    expect(total?.outcome?.lastfm.ms).toBe(600);
    expect(total?.outcome?.mirror?.firstMs).toBe(3);
    expect(total?.scenario.detail).toBe('2 requests, 1 of 2 identical');
    expect(totalRow([])).toBeNull();
  });
});
