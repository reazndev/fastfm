import { ChangeDetectionStrategy, Component, inject, resource, signal } from '@angular/core';

import { KeysService } from '../../api/keys.service';
import { ApiKey, KeyScope } from '../../api/models';
import { ErrorNotice } from '../../shared/error-notice';
import { Loading } from '../../shared/loading';
import { KeyForm } from './key-form';
import { KeyRow } from './key-row';

@Component({
  selector: 'app-keys-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ErrorNotice, KeyForm, KeyRow, Loading],
  template: `
    <div class="page">
      <div class="stack">
        <h1>API keys</h1>
        <p class="muted">
          Apps use a key and its secret to call this server the way they call Last.fm.
        </p>
      </div>
      <app-key-form [busy]="creating()" [error]="createError()" (created)="create($event)" />
      <section class="section" aria-label="Your keys">
        @if (keys.hasValue()) {
          @for (key of keys.value(); track key.key) {
            <app-key-row [apiKey]="key" [busy]="revoking() === key.key" (revoked)="revoke(key)" />
          } @empty {
            <p class="muted">No keys yet. Create one for each app you use.</p>
          }
          <app-error-notice [error]="revokeError()" />
        } @else if (keys.error()) {
          <app-error-notice [error]="keys.error()" />
          <button type="button" class="button" (click)="keys.reload()">Try again</button>
        } @else {
          <app-loading />
        }
      </section>
    </div>
  `,
})
export class KeysPage {
  private readonly service = inject(KeysService);
  protected readonly keys = resource({ loader: () => this.service.list() });
  protected readonly creating = signal(false);
  protected readonly createError = signal<unknown>(null);
  protected readonly revoking = signal<string | null>(null);
  protected readonly revokeError = signal<unknown>(null);

  protected async create(request: { label: string; scope: KeyScope }): Promise<void> {
    this.creating.set(true);
    this.createError.set(null);
    try {
      const key = await this.service.create(request.label, request.scope);
      this.keys.update((keys) => [key, ...(keys ?? [])]);
    } catch (error) {
      this.createError.set(error);
    } finally {
      this.creating.set(false);
    }
  }

  protected async revoke(key: ApiKey): Promise<void> {
    this.revoking.set(key.key);
    this.revokeError.set(null);
    try {
      await this.service.revoke(key.key);
      this.keys.update((keys) => keys?.filter((other) => other.key !== key.key));
    } catch (error) {
      this.revokeError.set(error);
    } finally {
      this.revoking.set(null);
    }
  }
}
