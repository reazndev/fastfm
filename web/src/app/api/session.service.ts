import { Injectable, computed, inject, signal } from '@angular/core';

import { ApiClient } from './api-client';
import { ApiError } from './api-error';
import { Account } from './models';

interface AccountBody {
  account: Account;
}

/** The logged-in account: `undefined` until the first check, `null` when logged out. */
@Injectable({ providedIn: 'root' })
export class SessionService {
  private readonly api = inject(ApiClient);
  private readonly current = signal<Account | null | undefined>(undefined);

  readonly account = this.current.asReadonly();
  readonly isAdmin = computed(() => this.current()?.role === 'admin');

  /** The account, asking the server only the first time. */
  async load(): Promise<Account | null> {
    const known = this.current();
    return known === undefined ? this.refresh() : known;
  }

  /** Asks the server who is logged in. */
  async refresh(): Promise<Account | null> {
    try {
      const { account } = await this.api.get<AccountBody>('/me');
      this.current.set(account);
    } catch (error) {
      if (!(error instanceof ApiError) || (error.status !== 401 && error.status !== 403)) {
        throw error;
      }
      this.current.set(null);
    }
    return this.current() ?? null;
  }

  async signup(username: string, password: string): Promise<Account> {
    return this.start(this.api.post<AccountBody>('/signup', { username, password }));
  }

  async login(username: string, password: string): Promise<Account> {
    return this.start(this.api.post<AccountBody>('/login', { username, password }));
  }

  async logout(): Promise<void> {
    await this.api.post('/logout');
    this.current.set(null);
  }

  private async start(request: Promise<AccountBody>): Promise<Account> {
    const { account } = await request;
    this.current.set(account);
    return account;
  }
}
