import { ChangeDetectionStrategy, Component, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ErrorNotice } from '../../shared/error-notice';

/** The Last.fm username to mirror, with the one action that starts it all. */
@Component({
  selector: 'app-link-form',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, ErrorNotice],
  template: `
    <div class="stack">
      <h1>Mirror your Last.fm profile</h1>
      <p class="muted">
        This server imports your scrobbles and keeps them in sync. Reading needs only your public
        profile.
      </p>
    </div>
    <form class="stack" (ngSubmit)="linked.emit(username().trim())">
      <label class="field">
        Last.fm username
        <input
          name="lastfm"
          required
          autocapitalize="off"
          spellcheck="false"
          [(ngModel)]="username"
        />
      </label>
      <app-error-notice [error]="error()" />
      <button type="submit" class="button primary" [disabled]="busy() || !username().trim()">
        {{ busy() ? 'Starting…' : 'Start import' }}
      </button>
    </form>
  `,
  styles: `
    :host {
      display: flex;
      flex-direction: column;
      gap: var(--space-6);
    }
  `,
})
export class LinkForm {
  readonly busy = input(false);
  readonly error = input<unknown>(null);
  readonly linked = output<string>();
  protected readonly username = signal('');
}
