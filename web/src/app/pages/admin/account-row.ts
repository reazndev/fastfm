import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';

import { AccountAction } from '../../api/admin.service';
import { Account, Role } from '../../api/models';
import { ConfirmButton } from '../../shared/confirm-button';
import { formatTime } from '../../shared/format';

/** What an admin asked to do to an account. */
export type AccountChange = { kind: AccountAction } | { kind: 'role'; role: Role };

/** One account with the actions that fit its state. */
@Component({
  selector: 'app-account-row',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ConfirmButton],
  template: `
    @let a = account();
    <article class="row account" [attr.aria-label]="a.username">
      <div class="grow">
        <p class="name">
          {{ a.username }}
          @if (self()) {
            <span class="muted small">(you)</span>
          }
        </p>
        <p class="muted small">
          {{ a.role === 'admin' ? 'Admin' : 'User' }} · {{ statusText[a.status] }} ·
          {{ a.lastfmUser ?? 'Not linked' }} · Joined {{ time(a.createdAt) }}
        </p>
      </div>
      @switch (a.status) {
        @case ('pending') {
          <button type="button" class="button" [disabled]="busy()" (click)="emit('approve')">
            Approve
          </button>
          <app-confirm-button
            label="Reject"
            question="Delete this account?"
            confirmLabel="Reject"
            [busy]="busy()"
            (confirmed)="emit('reject')"
          />
        }
        @case ('active') {
          <button type="button" class="button text" [disabled]="busy()" (click)="toggleRole()">
            {{ a.role === 'admin' ? 'Make user' : 'Make admin' }}
          </button>
          @if (!self()) {
            <app-confirm-button
              label="Disable"
              question="Stop sync and reject its keys?"
              confirmLabel="Disable"
              [busy]="busy()"
              (confirmed)="emit('disable')"
            />
          }
        }
        @case ('disabled') {
          <button type="button" class="button text" [disabled]="busy()" (click)="emit('enable')">
            Enable
          </button>
        }
      }
    </article>
  `,
  styles: `
    .account {
      padding: var(--space-2) 0;
    }
    .name {
      font-weight: 550;
    }
  `,
})
export class AccountRow {
  readonly account = input.required<Account>();
  readonly self = input(false);
  readonly busy = input(false);
  readonly changed = output<AccountChange>();
  protected readonly time = formatTime;
  protected readonly statusText = { pending: 'Pending', active: 'Active', disabled: 'Disabled' };

  protected emit(kind: AccountAction): void {
    this.changed.emit({ kind });
  }

  protected toggleRole(): void {
    const role: Role = this.account().role === 'admin' ? 'user' : 'admin';
    this.changed.emit({ kind: 'role', role });
  }
}
