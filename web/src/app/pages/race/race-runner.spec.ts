import { TestBed } from '@angular/core/testing';

import { ApiError } from '../../api/api-error';
import { DemoService } from '../../api/demo.service';
import { RacePage, RaceSide } from '../../api/models';
import { RaceRunner, WINDOW } from './race-runner';

function page(totalPages: number, n: number): RacePage {
  const track = { name: `Song ${n}`, artist: 'Cher', image: '', uts: 1000 - n };
  return { ms: 2, count: 200, total: totalPages * 200, totalPages, tracks: [track] };
}

type Fetch = (side: RaceSide, n: number) => Promise<RacePage>;

function setup(fetch: Fetch): { runner: RaceRunner; calls: string[] } {
  const calls: string[] = [];
  const fake: Partial<DemoService> = {
    racePage: (_user, side, n, tail) => {
      expect(tail).toBe(WINDOW);
      calls.push(`${side}:${n}`);
      return fetch(side, n);
    },
  };
  TestBed.configureTestingModule({
    providers: [RaceRunner, { provide: DemoService, useValue: fake }],
  });
  return { runner: TestBed.inject(RaceRunner), calls };
}

describe('RaceRunner', () => {
  it('walks both sides to their last page', async () => {
    const { runner, calls } = setup(async (side, n) => page(side === 'mirror' ? 3 : 2, n));
    await runner.start('syntiiix');

    expect(runner.running()).toBe(false);
    expect(runner.mirror()).toEqual(expect.objectContaining({ fetched: 600, total: 600 }));
    expect(runner.mirror().done).toBe(true);
    expect(runner.lastfm()).toEqual(expect.objectContaining({ fetched: 400, ms: 4 }));
    expect(calls.filter((c) => c.startsWith('mirror'))).toEqual([
      'mirror:1',
      'mirror:2',
      'mirror:3',
    ]);
  });

  it('stops only the lane whose request failed', async () => {
    const failure = new ApiError('race_page_failed', 'This page did not load.', 502);
    const { runner } = setup(async (side, n) => {
      if (side === 'lastfm' && n === 2) {
        throw failure;
      }
      return page(3, n);
    });
    await runner.start('syntiiix');

    expect(runner.lastfm()).toEqual(
      expect.objectContaining({ fetched: 200, done: true, error: failure }),
    );
    expect(runner.mirror()).toEqual(expect.objectContaining({ fetched: 600, error: null }));
  });

  it('abandons the walk when stopped', async () => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    const { runner, calls } = setup(async (_side, n) => {
      await held;
      return page(5, n);
    });
    const done = runner.start('syntiiix');
    runner.stop();
    release();
    await done;

    expect(runner.running()).toBe(false);
    expect(runner.mirror().fetched).toBe(0);
    expect(calls).toEqual(['lastfm:1', 'mirror:1']);
  });
});
