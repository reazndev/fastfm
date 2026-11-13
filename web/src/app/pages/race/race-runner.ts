import { Injectable, WritableSignal, computed, inject, signal } from '@angular/core';

import { DemoService } from '../../api/demo.service';
import { RacePage, RaceSide, RaceTrack } from '../../api/models';

/** Scrobbles each lane shows. */
export const WINDOW = 30;

/** One side's walk through the history so far. */
export interface Lane {
  /** The newest {@link WINDOW} scrobbles fetched. */
  tracks: RaceTrack[];
  fetched: number;
  /** Scrobbles in the whole history, once the first page is in. */
  total: number;
  /** Time spent in requests, measured on the server. */
  ms: number;
  done: boolean;
  error: unknown;
}

const EMPTY: Lane = { tracks: [], fetched: 0, total: 0, ms: 0, done: false, error: null };

/**
 * Merges a fetched page into the visible window. `page` holds the page's last scrobbles,
 * newest first, so it continues back in time from the window's last entry.
 */
export function slide(window: RaceTrack[], page: RaceTrack[]): RaceTrack[] {
  // TODO(human): return the WINDOW scrobbles the lane should show after this page.
  return page;
}

/** The lane after one more page. */
export function advance(lane: Lane, page: RacePage): Lane {
  return {
    ...lane,
    tracks: slide(lane.tracks, page.tracks),
    fetched: lane.fetched + page.count,
    total: page.total,
    ms: lane.ms + page.ms,
  };
}

/**
 * Walks a user's history page by page on Last.fm and on this mirror at the same time, each
 * side as fast as it answers. Starting again or stopping abandons the old walk.
 */
@Injectable()
export class RaceRunner {
  private readonly demo = inject(DemoService);
  readonly lastfm = signal<Lane>(EMPTY);
  readonly mirror = signal<Lane>(EMPTY);
  readonly running = signal(false);
  readonly started = computed(() => this.running() || this.lastfm().fetched > 0);
  private generation = 0;

  async start(username: string): Promise<void> {
    const run = ++this.generation;
    this.lastfm.set(EMPTY);
    this.mirror.set(EMPTY);
    this.running.set(true);
    await Promise.all([
      this.walk(run, username, 'lastfm', this.lastfm),
      this.walk(run, username, 'mirror', this.mirror),
    ]);
    if (run === this.generation) {
      this.running.set(false);
    }
  }

  stop(): void {
    this.generation += 1;
    this.running.set(false);
  }

  private async walk(
    run: number,
    username: string,
    side: RaceSide,
    lane: WritableSignal<Lane>,
  ): Promise<void> {
    for (let page = 1; ; page += 1) {
      let result: RacePage;
      try {
        result = await this.demo.racePage(username, side, page, WINDOW);
      } catch (error) {
        if (run === this.generation) {
          lane.update((l) => ({ ...l, error, done: true }));
        }
        return;
      }
      if (run !== this.generation) {
        return;
      }
      const last = page >= result.totalPages || result.count === 0;
      lane.update((l) => ({ ...advance(l, result), done: last }));
      if (last) {
        return;
      }
    }
  }
}
