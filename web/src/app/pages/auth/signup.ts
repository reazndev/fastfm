import { ChangeDetectionStrategy, Component, inject, input, signal } from '@angular/core';
import { Router, RouterLink } from '@angular/router';

import { ApiError } from '../../api/api-error';
import { SessionService } from '../../api/session.service';
import { homeFor, safeNext } from '../../guards';
import { Credentials, CredentialsForm } from './credentials-form';

@Component({
  selector: 'app-signup-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CredentialsForm, RouterLink],
  template: `
    <div class="page">
      <h1>Create account</h1>
      @if (closed()) {
        <p class="notice" role="status">
          Sign-up is closed on this server. Ask an admin to open it.
        </p>
      } @else {
        <p class="muted">The first account becomes the admin. Later ones may need approval.</p>
        <app-credentials-form
          submitLabel="Create account"
          [newPassword]="true"
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
  protected readonly busy = signal(false);
  protected readonly error = signal<unknown>(null);
  protected readonly closed = signal(false);

  protected async signup({ username, password }: Credentials): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    try {
      const account = await this.session.signup(username, password);
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
