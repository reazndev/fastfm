import { DemoOutcome } from '../../api/models';
import { Row } from './runner';

/** The shared log scale of every bar, in milliseconds. */
export interface Scale {
  min: number;
  max: number;
}

/** The shortest time the scale shows; anything faster draws as the shortest bar. */
const FLOOR_MS = 0.01;
/** The longest bar never stands for less than this. */
const CEILING_MS = 100;
/** The share of the track the shortest bar takes, so it stays visible. */
const SHORTEST = 0.02;

/** A scale from {@link FLOOR_MS} to the slowest time measured so far. */
export function scaleFor(outcomes: DemoOutcome[]): Scale {
  const times = outcomes.flatMap((o) => [o.lastfm.ms, o.mirror?.firstMs ?? 0]);
  return { min: FLOOR_MS, max: Math.max(CEILING_MS, ...times) };
}

/** How much of the track a bar for `ms` fills, from {@link SHORTEST} to 1. */
export function barFraction(ms: number, scale: Scale): number {
  const span = Math.log10(scale.max) - Math.log10(scale.min);
  const share = (Math.log10(Math.max(ms, scale.min)) - Math.log10(scale.min)) / span;
  return Math.min(1, Math.max(SHORTEST, share));
}

const whole = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 });
const tenths = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });
const hundredths = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });

/** `312 ms`, `4.2 ms`, `0.25 ms`. */
export function formatMs(ms: number): string {
  const format = ms >= 100 ? whole : ms >= 1 ? tenths : hundredths;
  return `${format.format(ms)} ms`;
}

/** How many times faster the mirror was: `×1,234`, or `×2.5` below ten. */
export function speedup(lastfmMs: number, mirrorMs: number): string {
  const factor = lastfmMs / Math.max(mirrorMs, 0.001);
  return `×${(factor >= 10 ? whole : tenths).format(factor)}`;
}

/** `950 B`, `14.2 kB`, `1.3 MB`. */
export function formatBytes(bytes: number): string {
  if (bytes < 1000) {
    return `${bytes} B`;
  }
  return bytes < 1_000_000
    ? `${tenths.format(bytes / 1000)} kB`
    : `${tenths.format(bytes / 1_000_000)} MB`;
}

/** The sum of every finished scenario the mirror answered locally, as one more row. */
export function totalRow(rows: Row[]): Row | null {
  const done = rows.flatMap((row) => (row.outcome?.mirror ? [row.outcome] : []));
  if (done.length === 0) {
    return null;
  }
  const sum = (pick: (o: DemoOutcome) => number) => done.reduce((total, o) => total + pick(o), 0);
  const requests = sum((o) => o.requests);
  const same = done.filter((o) => o.identical).length;
  const detail = `${requests} requests, ${same} of ${done.length} identical`;
  return {
    scenario: { id: 'total', title: 'Total', detail, requests },
    outcome: {
      id: 'total',
      requests,
      lastfm: { ms: sum((o) => o.lastfm.ms), bytes: sum((o) => o.lastfm.bytes), ok: true },
      mirror: {
        firstMs: sum((o) => o.mirror?.firstMs ?? 0),
        cachedMs: sum((o) => o.mirror?.cachedMs ?? 0),
        bytes: sum((o) => o.mirror?.bytes ?? 0),
        ok: true,
      },
      mirrorSource: 'local',
      identical: done.every((o) => o.identical),
    },
  };
}
