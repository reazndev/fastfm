import { Injectable, computed, inject, signal } from '@angular/core';

import { ApiError } from '../../api/api-error';
import { DemoService } from '../../api/demo.service';
import { DemoOutcome, DemoPrepared, DemoScenario, ImportView } from '../../api/models';

/** Where the comparison stands. */
export type Phase =
  'idle' | 'preparing' | 'not_mirrored' | 'importing' | 'running' | 'done' | 'failed';

/** One scenario and, once it ran, its timings. */
export interface Row {
  scenario: DemoScenario;
  outcome: DemoOutcome | null;
}

/** How often to ask for import progress, in milliseconds. */
const POLL_EVERY = 2000;

const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * Drives one comparison: prepares the user, imports them when needed, then runs the scenarios
 * one after another so the page fills in as each finishes. Starting again abandons the old run.
 */
@Injectable()
export class DemoRunner {
  private readonly demo = inject(DemoService);
  readonly phase = signal<Phase>('idle');
  readonly prepared = signal<DemoPrepared | null>(null);
  readonly progress = signal<ImportView | null>(null);
  readonly rows = signal<Row[]>([]);
  readonly error = signal<unknown>(null);
  /** The scenario being measured now. */
  readonly current = computed(() =>
    this.phase() === 'running' ? this.rows().find((row) => !row.outcome)?.scenario.id : undefined,
  );
  readonly busy = computed(() => ['preparing', 'importing', 'running'].includes(this.phase()));
  pollEvery = POLL_EVERY;
  private generation = 0;

  async compare(username: string): Promise<void> {
    const run = this.begin('preparing');
    this.prepared.set(null);
    this.progress.set(null);
    await this.guard(run, () => this.continueWith(run, username));
  }

  /** Imports the prepared user that is not mirrored yet, then compares. */
  async importAndCompare(): Promise<void> {
    const username = this.prepared()?.username;
    if (!username) {
      return;
    }
    const run = this.begin('importing');
    await this.guard(run, async () => {
      this.progress.set(await this.demo.startImport(username));
      await this.awaitImport(run, username);
    });
  }

  private async continueWith(run: number, username: string): Promise<void> {
    const prepared = await this.demo.prepare(username);
    if (!this.live(run)) {
      return;
    }
    this.prepared.set(prepared);
    this.progress.set(prepared.import);
    if (prepared.state === 'mirrored') {
      await this.runAll(run, prepared);
    } else if (prepared.state === 'importing') {
      await this.awaitImport(run, prepared.username);
    } else {
      this.phase.set('not_mirrored');
    }
  }

  private async awaitImport(run: number, username: string): Promise<void> {
    this.phase.set('importing');
    for (;;) {
      const view = this.progress();
      if (view?.status === 'done') {
        return this.continueWith(run, username);
      }
      if (view?.status === 'failed') {
        throw new ApiError('import_failed', view.error ?? 'The import stopped.', 0);
      }
      await delay(this.pollEvery);
      if (!this.live(run)) {
        return;
      }
      this.progress.set(await this.demo.importProgress(username));
    }
  }

  private async runAll(run: number, prepared: DemoPrepared): Promise<void> {
    this.phase.set('running');
    this.rows.set(prepared.scenarios.map((scenario) => ({ scenario, outcome: null })));
    for (const { id } of prepared.scenarios) {
      const outcome = await this.demo.run(prepared.username, id);
      if (!this.live(run)) {
        return;
      }
      this.rows.update((rows) =>
        rows.map((row) => (row.scenario.id === id ? { ...row, outcome } : row)),
      );
    }
    this.phase.set('done');
  }

  private begin(phase: Phase): number {
    this.generation += 1;
    this.phase.set(phase);
    this.rows.set([]);
    this.error.set(null);
    return this.generation;
  }

  private live(run: number): boolean {
    return run === this.generation;
  }

  private async guard(run: number, work: () => Promise<void>): Promise<void> {
    try {
      await work();
    } catch (error) {
      if (this.live(run)) {
        this.error.set(error);
        this.phase.set('failed');
      }
    }
  }
}
