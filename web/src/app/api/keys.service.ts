import { Injectable, inject } from '@angular/core';

import { ApiClient } from './api-client';
import { ApiKey, KeyScope } from './models';

/** The account's API keys for the Last.fm-compatible API. */
@Injectable({ providedIn: 'root' })
export class KeysService {
  private readonly api = inject(ApiClient);

  async list(): Promise<ApiKey[]> {
    const { keys } = await this.api.get<{ keys: ApiKey[] }>('/keys');
    return keys;
  }

  async create(label: string, scope: KeyScope): Promise<ApiKey> {
    const { key } = await this.api.post<{ key: ApiKey }>('/keys', { label, scope });
    return key;
  }

  revoke(key: string): Promise<void> {
    return this.api.delete(`/keys/${encodeURIComponent(key)}`);
  }
}
