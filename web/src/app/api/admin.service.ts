import { Injectable, inject } from '@angular/core';

import { ApiClient } from './api-client';
import { Account, InstanceSettings, Role, SignupMode } from './models';

/** What an admin can do to an account with a plain `POST`. */
export type AccountAction = 'approve' | 'reject' | 'disable' | 'enable';

/** Accounts and instance settings, for admins. */
@Injectable({ providedIn: 'root' })
export class AdminService {
  private readonly api = inject(ApiClient);

  async accounts(): Promise<Account[]> {
    const { accounts } = await this.api.get<{ accounts: Account[] }>('/admin/accounts');
    return accounts;
  }

  act(id: number, action: AccountAction): Promise<void> {
    return this.api.post(`/admin/accounts/${id}/${action}`);
  }

  setRole(id: number, role: Role): Promise<void> {
    return this.api.put(`/admin/accounts/${id}/role`, { role });
  }

  settings(): Promise<InstanceSettings> {
    return this.api.get<InstanceSettings>('/admin/settings');
  }

  setSignupMode(signupMode: SignupMode): Promise<InstanceSettings> {
    return this.api.put<InstanceSettings>('/admin/settings', { signupMode });
  }

  setRequireOwnKey(requireOwnKey: boolean): Promise<InstanceSettings> {
    return this.api.put<InstanceSettings>('/admin/settings', { requireOwnKey });
  }
}
