import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  resource,
  signal,
} from '@angular/core';

import { AppAuthService } from '../../api/app-auth.service';
import { ApiError } from '../../api/api-error';
import { SessionService } from '../../api/session.service';
import { ErrorNotice } from '../../shared/error-notice';
import { Loading } from '../../shared/loading';

/** The approval page Last.fm clients open after `auth.getToken`, via `/api/auth/`. */
@Component({
  selector: 'app-authorize-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ErrorNotice, Loading],
  templateUrl: './authorize.html',
})
export class AuthorizePage {
  private readonly service = inject(AppAuthService);
  protected readonly session = inject(SessionService);
  /** From `?token=`. */
  readonly token = input<string>();
  protected readonly request = resource({
    params: () => this.token() ?? '',
    loader: ({ params }) =>
      params ? this.service.request(params) : Promise.reject(missingToken()),
  });
  protected readonly approving = signal(false);
  protected readonly approveError = signal<unknown>(null);
  protected readonly done = computed(
    () => this.request.hasValue() && this.request.value().approved,
  );

  protected async approve(): Promise<void> {
    const token = this.token();
    if (!token) {
      return;
    }
    this.approving.set(true);
    this.approveError.set(null);
    try {
      await this.service.approve(token);
      this.request.update((request) => request && { ...request, approved: true });
    } catch (error) {
      this.approveError.set(error);
    } finally {
      this.approving.set(false);
    }
  }
}

function missingToken(): ApiError {
  return new ApiError('token_not_found', 'This link has no request token.', 400);
}
