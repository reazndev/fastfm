import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  resource,
  signal,
} from '@angular/core';

import { AccountAction, AdminService } from '../../api/admin.service';
import { Account, SignupMode } from '../../api/models';
import { SessionService } from '../../api/session.service';
import { ErrorNotice } from '../../shared/error-notice';
import { Loading } from '../../shared/loading';
import { AccountChange, AccountRow } from './account-row';
import { SignupModePicker } from './signup-mode';

/** Pending accounts first, then the rest oldest first. */
export function byPendingFirst(accounts: Account[]): Account[] {
  const rank = (account: Account) => (account.status === 'pending' ? 0 : 1);
  return [...accounts].sort((a, b) => rank(a) - rank(b) || a.createdAt - b.createdAt);
}

@Component({
  selector: 'app-admin-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AccountRow, ErrorNotice, Loading, SignupModePicker],
  templateUrl: './admin.html',
})
export class AdminPage {
  private readonly admin = inject(AdminService);
  protected readonly session = inject(SessionService);
  protected readonly accounts = resource({ loader: () => this.admin.accounts() });
  protected readonly settings = resource({ loader: () => this.admin.settings() });
  protected readonly sorted = computed(() =>
    this.accounts.hasValue() ? byPendingFirst(this.accounts.value()) : [],
  );
  protected readonly pending = computed(
    () => this.sorted().filter((account) => account.status === 'pending').length,
  );
  protected readonly working = signal<number | null>(null);
  protected readonly accountError = signal<unknown>(null);
  protected readonly modeError = signal<unknown>(null);

  protected async change(account: Account, change: AccountChange): Promise<void> {
    this.working.set(account.id);
    this.accountError.set(null);
    try {
      await (change.kind === 'role'
        ? this.admin.setRole(account.id, change.role)
        : this.admin.act(account.id, change.kind satisfies AccountAction));
      this.accounts.reload();
    } catch (error) {
      this.accountError.set(error);
    } finally {
      this.working.set(null);
    }
  }

  protected async setMode(mode: SignupMode): Promise<void> {
    this.modeError.set(null);
    try {
      this.settings.set(await this.admin.setSignupMode(mode));
    } catch (error) {
      this.modeError.set(error);
      this.settings.reload();
    }
  }
}
