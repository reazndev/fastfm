import { ChangeDetectionStrategy, Component, DestroyRef, inject, signal } from '@angular/core';
import { Router } from '@angular/router';

import { SessionService } from '../api/session.service';
import { ErrorNotice } from '../shared/error-notice';

/** How often to ask whether an admin approved the account, in milliseconds. */
const CHECK_EVERY = 15_000;

/** What a pending account sees until an admin approves it. */
@Component({
  selector: 'app-pending-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ErrorNotice],
  template: `
    <div class="page">
      <div class="stack">
        <h1>Waiting for approval</h1>
        <p class="muted">
          An admin needs to approve {{ session.account()?.username }} before you can link Last.fm or
          create API keys. This page moves on by itself once that happens.
        </p>
      </div>
      <app-error-notice [error]="error()" />
      <div class="row">
        <button type="button" class="button primary" [disabled]="busy()" (click)="check()">
          Check again
        </button>
        <button type="button" class="button text" (click)="logout()">Log out</button>
      </div>
    </div>
  `,
})
export class PendingPage {
  protected readonly session = inject(SessionService);
  private readonly router = inject(Router);
  protected readonly busy = signal(false);
  protected readonly error = signal<unknown>(null);

  constructor() {
    const timer = setInterval(() => void this.check(), CHECK_EVERY);
    inject(DestroyRef).onDestroy(() => clearInterval(timer));
  }

  protected async check(): Promise<void> {
    this.busy.set(true);
    try {
      const account = await this.session.refresh();
      this.error.set(null);
      if (account?.status !== 'pending') {
        await this.router.navigateByUrl(account ? '/' : '/login');
      }
    } catch (error) {
      this.error.set(error);
    } finally {
      this.busy.set(false);
    }
  }

  protected async logout(): Promise<void> {
    await this.session.logout();
    await this.router.navigateByUrl('/login');
  }
}
