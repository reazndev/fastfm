import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  resource,
  signal,
} from '@angular/core';
import { Router, RouterLink } from '@angular/router';

import { ApiError } from '../../api/api-error';
import { SignupInfo } from '../../api/models';
import { SessionService } from '../../api/session.service';
import { homeFor, safeNext } from '../../guards';
import { Credentials, CredentialsForm } from './credentials-form';

/** The line under the heading that says what happens to a new account. */
export function signupHint(info: SignupInfo): string | null {
  if (!info.hasAccounts) {
    return 'The first account becomes the admin.';
  }
  return info.mode === 'approval'
    ? 'An admin approves new accounts before they can be used.'
    : null;
}

@Component({
  selector: 'app-signup-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CredentialsForm, RouterLink],
  template: `
    <div class="page">
      <h1>Create account</h1>
      @let mode = signupInfo();
      @if (closed() || (mode?.hasAccounts && mode?.mode === 'closed')) {
        <p class="notice" role="status">
          Sign-up is closed on this server. Ask an admin to open it.
        </p>
      } @else {
        @if (mode && hint(mode); as line) {
          <p class="muted">{{ line }}</p>
        }
        <app-credentials-form
          submitLabel="Create account"
          [newPassword]="true"
          [askSetupToken]="mode?.setupTokenRequired ?? false"
          [busy]="busy()"
          [error]="error()"
          (submitted)="signup($event)"
        />
      }
      <p class="muted">
        Have an account? <a routerLink="/login" [queryParams]="{ next: next() }">Log in</a>
      </p>
    </div>
  `,
})
export class SignupPage {
  private readonly session = inject(SessionService);
  private readonly router = inject(Router);
  readonly next = input<string>();
  /** The sign-up mode; while it loads or if it fails, the form shows and the server decides. */
  private readonly info = resource({ loader: () => this.session.signupInfo() });
  protected readonly signupInfo = computed(() => (this.info.hasValue() ? this.info.value() : null));
  protected readonly hint = signupHint;
  protected readonly busy = signal(false);
  protected readonly error = signal<unknown>(null);
  protected readonly closed = signal(false);

  protected async signup({ username, password, setupToken }: Credentials): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    try {
      const account = await this.session.signup(username, password, setupToken);
      const target = account.status === 'active' ? safeNext(this.next()) : null;
      await this.router.navigateByUrl(target ?? homeFor(account));
    } catch (error) {
      this.closed.set(error instanceof ApiError && error.is('signup_closed'));
      this.error.set(error);
    } finally {
      this.busy.set(false);
    }
  }
}
