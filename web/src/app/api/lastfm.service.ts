import { Injectable, inject } from '@angular/core';

import { ApiClient } from './api-client';
import { LinkStatus } from './models';

/** The account's Last.fm link: the mirrored user, its own key, and web auth for writes. */
@Injectable({ providedIn: 'root' })
export class LastfmService {
  private readonly api = inject(ApiClient);

  status(): Promise<LinkStatus> {
    return this.api.get<LinkStatus>('/lastfm');
  }

  /** Links the Last.fm user and starts the import. */
  link(username: string): Promise<LinkStatus> {
    return this.api.put<LinkStatus>('/lastfm', { username });
  }

  unlink(): Promise<void> {
    return this.api.delete('/lastfm');
  }

  setOwnKey(apiKey: string, secret: string): Promise<void> {
    return this.api.put('/lastfm/key', { apiKey, secret });
  }

  clearOwnKey(): Promise<void> {
    return this.api.delete('/lastfm/key');
  }

  /** The last.fm page where the user approves this server. */
  async authUrl(): Promise<string> {
    const { url } = await this.api.get<{ url: string }>('/lastfm/auth/start');
    return url;
  }

  /** Trades the token Last.fm sent back for a session key; `state` ties it to this session. */
  finishAuth(state: string, token: string): Promise<LinkStatus> {
    return this.api.get<LinkStatus>('/lastfm/auth/callback', { state, token });
  }

  /** Tries the import again, after a private profile was made public. */
  retryImport(): Promise<LinkStatus> {
    return this.api.post<LinkStatus>('/lastfm/import/retry');
  }
}
