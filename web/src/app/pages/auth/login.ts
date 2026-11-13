import { ChangeDetectionStrategy, Component, inject, input, signal } from '@angular/core';
import { Router, RouterLink } from '@angular/router';

import { SessionService } from '../../api/session.service';
import { homeFor, safeNext } from '../../guards';
import { Credentials, CredentialsForm } from './credentials-form';

@Component({
  selector: 'app-login-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CredentialsForm, RouterLink],
  template: `
    <div class="page">
      <h1>Log in</h1>
      <app-credentials-form
        submitLabel="Log in"
        [busy]="busy()"
        [error]="error()"
        (submitted)="login($event)"
      />
      <p class="muted">
        New here? <a routerLink="/signup" [queryParams]="{ next: next() }">Create an account</a>
      </p>
    </div>
  `,
})
export class LoginPage {
  private readonly session = inject(SessionService);
  private readonly router = inject(Router);
  /** Where to go after logging in, from `?next=`. */
  readonly next = input<string>();
  protected readonly busy = signal(false);
  protected readonly error = signal<unknown>(null);

  protected async login({ username, password }: Credentials): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    try {
      const account = await this.session.login(username, password);
      const target = account.status === 'active' ? safeNext(this.next()) : null;
      await this.router.navigateByUrl(target ?? homeFor(account));
    } catch (error) {
      this.error.set(error);
    } finally {
      this.busy.set(false);
    }
  }
}
