import { ChangeDetectionStrategy, Component, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ErrorNotice } from '../../shared/error-notice';

/** A username and a password, and the setup token when the form asked for one. */
export interface Credentials {
  username: string;
  password: string;
  setupToken?: string;
}

/** The username and password form that login and sign-up share. */
@Component({
  selector: 'app-credentials-form',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, ErrorNotice],
  template: `
    <form class="stack" (ngSubmit)="submit()">
      <label class="field">
        Username
        <input
          name="username"
          autocomplete="username"
          required
          [(ngModel)]="username"
          autocapitalize="off"
          spellcheck="false"
        />
      </label>
      <label class="field">
        Password
        <input
          name="password"
          type="password"
          required
          [attr.autocomplete]="newPassword() ? 'new-password' : 'current-password'"
          [(ngModel)]="password"
        />
        @if (newPassword()) {
          <span class="muted small">At least 10 characters.</span>
        }
      </label>
      @if (askSetupToken()) {
        <label class="field">
          Setup token
          <input
            name="setupToken"
            required
            autocomplete="off"
            spellcheck="false"
            [(ngModel)]="setupToken"
          />
          <span class="muted small">From LFM_SETUP_TOKEN in the server's settings.</span>
        </label>
      }
      <app-error-notice [error]="error()" />
      <button type="submit" class="button primary" [disabled]="busy()">{{ submitLabel() }}</button>
    </form>
  `,
})
export class CredentialsForm {
  readonly submitLabel = input.required<string>();
  readonly newPassword = input(false);
  /** Shows the setup token field, for the first account on a server that sets one. */
  readonly askSetupToken = input(false);
  readonly busy = input(false);
  readonly error = input<unknown>(null);
  readonly submitted = output<Credentials>();
  protected readonly username = signal('');
  protected readonly password = signal('');
  protected readonly setupToken = signal('');

  protected submit(): void {
    const credentials = { username: this.username(), password: this.password() };
    this.submitted.emit(
      this.askSetupToken() ? { ...credentials, setupToken: this.setupToken().trim() } : credentials,
    );
  }
}
