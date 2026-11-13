import { Injectable, inject } from '@angular/core';

import { ApiClient } from './api-client';
import { AuthRequest } from './models';

/** The approval step of the emulated Last.fm auth flow (`auth.getToken`, then this page). */
@Injectable({ providedIn: 'root' })
export class AppAuthService {
  private readonly api = inject(ApiClient);

  request(token: string): Promise<AuthRequest> {
    return this.api.get<AuthRequest>('/auth/request', { token });
  }

  approve(token: string): Promise<void> {
    return this.api.post('/auth/approve', { token });
  }
}
