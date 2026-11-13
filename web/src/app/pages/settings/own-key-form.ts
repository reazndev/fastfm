import { ChangeDetectionStrategy, Component, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ErrorNotice } from '../../shared/error-notice';

/** A Last.fm application's key and shared secret. */
export interface OwnKey {
  apiKey: string;
  secret: string;
}

/** Fields for the account's own Last.fm application. */
@Component({
  selector: 'app-own-key-form',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, ErrorNotice],
  template: `
    <form
      class="stack"
      (ngSubmit)="saved.emit({ apiKey: apiKey().trim(), secret: secret().trim() })"
    >
      <label class="field">
        API key
        <input name="apiKey" required autocomplete="off" spellcheck="false" [(ngModel)]="apiKey" />
      </label>
      <label class="field">
        Shared secret
        <input name="secret" type="password" required autocomplete="off" [(ngModel)]="secret" />
      </label>
      <p class="muted small">
        Create one at
        <a href="https://www.last.fm/api/account/create" target="_blank" rel="noopener"
          >last.fm/api/account/create</a
        >.
      </p>
      <app-error-notice [error]="error()" />
      <div>
        <button type="submit" class="button" [disabled]="busy() || !apiKey() || !secret()">
          Save key
        </button>
      </div>
    </form>
  `,
})
export class OwnKeyForm {
  readonly busy = input(false);
  readonly error = input<unknown>(null);
  readonly saved = output<OwnKey>();
  protected readonly apiKey = signal('');
  protected readonly secret = signal('');
}
