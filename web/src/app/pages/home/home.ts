import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  resource,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { LastfmService } from '../../api/lastfm.service';
import { LinkStatus } from '../../api/models';
import { ErrorNotice } from '../../shared/error-notice';
import { apiBaseUrl } from '../../shared/format';
import { Loading } from '../../shared/loading';
import { OwnKey, OwnKeyForm } from '../settings/own-key-form';
import { ClientSetup } from './client-setup';
import { LinkForm } from './link-form';
import { LinkStatusView } from './link-status';

/** How often to refresh while an import runs, in milliseconds. */
const POLL_EVERY = 3000;
/** Refreshes after a retry, until the import shows it started again. */
const POLLS_AFTER_RETRY = 5;

/** Whether the import has not finished yet, so the numbers are still moving. */
export function importing(status: LinkStatus): boolean {
  if (!status.username) {
    return false;
  }
  const job = status.import?.status;
  return job === undefined || job === 'queued' || job === 'running';
}

/** Whether an unlinked account must save its own Last.fm key before linking. */
export function needsOwnKey(status: LinkStatus): boolean {
  return !status.username && status.credentials.ownKeyRequired && !status.ownKey;
}

@Component({
  selector: 'app-home-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ClientSetup, ErrorNotice, LinkForm, LinkStatusView, Loading, OwnKeyForm, RouterLink],
  template: `
    <div class="page">
      @if (status.hasValue()) {
        @let current = status.value();
        @if (current.credentials.error; as problem) {
          <p class="notice" role="alert">
            {{ problem }} <a routerLink="/settings">Open settings</a>
          </p>
        }
        @if (current.username) {
          <app-link-status [status]="current" [retrying]="busy()" (retried)="retry()" />
          <app-error-notice [error]="actionError()" />
          <app-client-setup [baseUrl]="baseUrl" />
        } @else if (keyFirst()) {
          <div class="stack">
            <h1>Add your Last.fm API key</h1>
            <p class="muted">
              This server asks every account for its own key before mirroring a profile. You can
              change it later in <a routerLink="/settings">settings</a>.
            </p>
          </div>
          <app-own-key-form [busy]="busy()" [error]="actionError()" (saved)="saveKey($event)" />
        } @else {
          <app-link-form [busy]="busy()" [error]="actionError()" (linked)="link($event)" />
        }
      } @else if (status.error()) {
        <h1>Home</h1>
        <app-error-notice [error]="status.error()" />
        <button type="button" class="button" (click)="status.reload()">Try again</button>
      } @else {
        <app-loading />
      }
    </div>
  `,
})
export class HomePage {
  private readonly lastfm = inject(LastfmService);
  protected readonly status = resource({ loader: () => this.lastfm.status() });
  protected readonly busy = signal(false);
  protected readonly actionError = signal<unknown>(null);
  protected readonly baseUrl = apiBaseUrl(window.location.origin);
  protected readonly keyFirst = computed(
    () => this.status.hasValue() && needsOwnKey(this.status.value()),
  );
  private readonly retryPolls = signal(0);
  private readonly polling = computed(
    () => this.status.hasValue() && (importing(this.status.value()) || this.retryPolls() > 0),
  );

  constructor() {
    effect((onCleanup) => {
      if (this.polling() && !this.status.isLoading()) {
        const timer = setTimeout(() => {
          this.retryPolls.update((left) => Math.max(0, left - 1));
          this.status.reload();
        }, POLL_EVERY);
        onCleanup(() => clearTimeout(timer));
      }
    });
  }

  protected link(username: string): Promise<void> {
    return this.run(async () => this.status.set(await this.lastfm.link(username)));
  }

  protected saveKey(key: OwnKey): Promise<void> {
    return this.run(async () => {
      await this.lastfm.setOwnKey(key.apiKey, key.secret);
      this.status.reload();
    });
  }

  protected retry(): Promise<void> {
    return this.run(async () => {
      this.status.set(await this.lastfm.retryImport());
      this.retryPolls.set(POLLS_AFTER_RETRY);
    });
  }

  private async run(action: () => Promise<void>): Promise<void> {
    this.busy.set(true);
    this.actionError.set(null);
    try {
      await action();
    } catch (error) {
      this.actionError.set(error);
    } finally {
      this.busy.set(false);
    }
  }
}
