import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  resource,
  signal,
} from '@angular/core';

import { LastfmService } from '../../api/lastfm.service';
import { LinkStatus } from '../../api/models';
import { ErrorNotice } from '../../shared/error-notice';
import { apiBaseUrl } from '../../shared/format';
import { Loading } from '../../shared/loading';
import { ClientSetup } from './client-setup';
import { LinkForm } from './link-form';
import { LinkStatusView } from './link-status';

/** How often to refresh while an import runs, in milliseconds. */
const POLL_EVERY = 3000;

/** Whether the import has not finished yet, so the numbers are still moving. */
export function importing(status: LinkStatus): boolean {
  if (!status.username) {
    return false;
  }
  const job = status.import?.status;
  return job === undefined || job === 'queued' || job === 'running';
}

@Component({
  selector: 'app-home-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ClientSetup, ErrorNotice, LinkForm, LinkStatusView, Loading],
  template: `
    <div class="page">
      @if (status.hasValue()) {
        @let current = status.value();
        @if (current.username) {
          <app-link-status [status]="current" />
          <app-client-setup [baseUrl]="baseUrl" />
        } @else {
          <app-link-form [busy]="linking()" [error]="linkError()" (linked)="link($event)" />
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
  protected readonly linking = signal(false);
  protected readonly linkError = signal<unknown>(null);
  protected readonly baseUrl = apiBaseUrl(window.location.origin);
  private readonly polling = computed(
    () => this.status.hasValue() && importing(this.status.value()),
  );

  constructor() {
    effect((onCleanup) => {
      if (this.polling() && !this.status.isLoading()) {
        const timer = setTimeout(() => this.status.reload(), POLL_EVERY);
        onCleanup(() => clearTimeout(timer));
      }
    });
  }

  protected async link(username: string): Promise<void> {
    this.linking.set(true);
    this.linkError.set(null);
    try {
      this.status.set(await this.lastfm.link(username));
    } catch (error) {
      this.linkError.set(error);
    } finally {
      this.linking.set(false);
    }
  }
}
