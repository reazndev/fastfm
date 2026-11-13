import { Injectable, inject } from '@angular/core';

import { ApiClient } from './api-client';
import { DemoOutcome, DemoPrepared, ImportView, RacePage, RaceSide } from './models';

/** The speed comparison. The server times both sides; no Last.fm key reaches the browser. */
@Injectable({ providedIn: 'root' })
export class DemoService {
  private readonly api = inject(ApiClient);

  /** Whether the user is mirrored, and the scenarios to run if so. */
  prepare(username: string): Promise<DemoPrepared> {
    return this.api.post<DemoPrepared>('/demo/prepare', { username });
  }

  /** Starts a one-off import of the user's history. */
  startImport(username: string): Promise<ImportView> {
    return this.api.post<ImportView>('/demo/import', { username });
  }

  importProgress(username: string): Promise<ImportView> {
    return this.api.get<ImportView>('/demo/import', { username });
  }

  /** Times one scenario on Last.fm and on this mirror. */
  run(username: string, scenario: string): Promise<DemoOutcome> {
    return this.api.post<DemoOutcome>('/demo/run', { username, scenario });
  }

  /** One page of a mirrored user's history from `side`, keeping its last `tail` scrobbles. */
  racePage(username: string, side: RaceSide, page: number, tail: number): Promise<RacePage> {
    return this.api.post<RacePage>('/demo/race', { username, side, page, tail });
  }
}
