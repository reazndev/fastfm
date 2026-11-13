import { TestBed } from '@angular/core/testing';

import { ApiError } from '../../api/api-error';
import { DemoService } from '../../api/demo.service';
import { DemoOutcome, DemoPrepared, ImportView } from '../../api/models';
import { DemoRunner } from './runner';

const scenarios = ['latest', 'day', 'profile'].map((id) => ({
  id,
  title: id,
  detail: `user.getRecentTracks ${id}`,
  requests: 1,
}));

const mirrored: DemoPrepared = {
  username: 'Syntiiix',
  state: 'mirrored',
  playcount: 160_643,
  maxScrobbles: 2_000_000,
  import: null,
  scenarios,
};

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

function job(status: ImportView['status'], imported: number, error: string | null = null) {
  return { status, imported, estimatedTotal: 10, error };
}

/** A promise with its resolve and reject at hand, so a test decides when a call answers. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
  return { promise, resolve, reject };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

function setup(fake: Partial<DemoService>): DemoRunner {
  TestBed.configureTestingModule({
    providers: [DemoRunner, { provide: DemoService, useValue: fake }],
  });
  const runner = TestBed.inject(DemoRunner);
  runner.pollEvery = 0;
  return runner;
}

describe('DemoRunner', () => {
  it('runs the scenarios one at a time and fills in each result', async () => {
    const answers = scenarios.map(() => deferred<DemoOutcome>());
    const asked: string[] = [];
    const runner = setup({
      prepare: async () => mirrored,
      run: (_user: string, id: string) => {
        asked.push(id);
        return answers[asked.length - 1].promise;
      },
    });
    const done = runner.compare('syntiiix');
    await settle();
    expect(runner.phase()).toBe('running');
    expect(asked).toEqual(['latest']);
    expect(runner.current()).toBe('latest');

    answers[0].resolve(outcome('latest'));
    await settle();
    expect(asked).toEqual(['latest', 'day']);
    expect(runner.rows().map((row) => row.outcome !== null)).toEqual([true, false, false]);
    expect(runner.current()).toBe('day');

    answers[1].resolve(outcome('day', false));
    answers[2].resolve(outcome('profile'));
    await done;
    expect(runner.phase()).toBe('done');
    expect(runner.current()).toBeUndefined();
    expect(runner.rows().every((row) => row.outcome !== null)).toBe(true);
  });

  it('keeps finished rows and shows the error when a scenario fails', async () => {
    let calls = 0;
    const runner = setup({
      prepare: async () => mirrored,
      run: async (_user: string, id: string) => {
        calls += 1;
        if (calls === 2) {
          throw new ApiError('demo_request_failed', 'A timed request did not complete.', 502);
        }
        return outcome(id);
      },
    });
    await runner.compare('syntiiix');
    expect(runner.phase()).toBe('failed');
    expect((runner.error() as ApiError).is('demo_request_failed')).toBe(true);
    expect(runner.rows().map((row) => row.outcome?.id ?? null)).toEqual(['latest', null, null]);
    expect(calls).toBe(2);
  });
});

describe('DemoRunner errors and imports', () => {
  it('reports an unknown Last.fm user', async () => {
    const runner = setup({
      prepare: async () => {
        throw new ApiError('lastfm_user_not_found', 'Last.fm has no user with that name.', 404);
      },
    });
    await runner.compare('nobody');
    expect(runner.phase()).toBe('failed');
    expect((runner.error() as Error).message).toBe('Last.fm has no user with that name.');
  });

  it('offers an import, follows its progress, then compares', async () => {
    const progress = [job('running', 4), job('done', 10)];
    let prepared = 0;
    const runner = setup({
      prepare: async () => {
        prepared += 1;
        return prepared === 1 ? { ...mirrored, state: 'not_mirrored', scenarios: [] } : mirrored;
      },
      startImport: async () => job('queued', 0),
      importProgress: async () => progress.shift() ?? job('done', 10),
      run: async (_user: string, id: string) => outcome(id),
    });
    await runner.compare('syntiiix');
    expect(runner.phase()).toBe('not_mirrored');

    const seen: string[] = [];
    const done = runner.importAndCompare();
    seen.push(runner.phase());
    await done;
    expect(seen).toEqual(['importing']);
    expect(runner.phase()).toBe('done');
    expect(runner.rows()).toHaveLength(3);
  });

  it('stops with the reason when the import fails', async () => {
    const runner = setup({
      prepare: async () => ({ ...mirrored, state: 'importing', import: job('running', 1) }),
      importProgress: async () => job('failed', 3, 'Last.fm did not answer.'),
    });
    await runner.compare('syntiiix');
    expect(runner.phase()).toBe('failed');
    expect((runner.error() as Error).message).toBe('Last.fm did not answer.');
  });
});
