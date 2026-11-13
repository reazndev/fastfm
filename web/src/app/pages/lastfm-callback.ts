import { ChangeDetectionStrategy, Component, inject, input, resource } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiError } from '../api/api-error';
import { LastfmService } from '../api/lastfm.service';
import { ErrorNotice } from '../shared/error-notice';
import { Loading } from '../shared/loading';

/**
 * Where last.fm sends the browser back with `?state=…&token=…` after the user approved this
 * server. It acts on load, which is safe because the server accepts only a state this session
 * started moments before.
 */
@Component({
  selector: 'app-lastfm-callback-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ErrorNotice, Loading, RouterLink],
  template: `
    <div class="page">
      @if (result.hasValue()) {
        <div class="stack" role="status">
          <h1>Last.fm connected</h1>
          <p class="muted">
            Scrobbles and loves from your apps now go on to {{ result.value().username }} on
            Last.fm.
          </p>
        </div>
        <div><a class="button primary" routerLink="/">Go to home</a></div>
      } @else if (result.error()) {
        <h1>Last.fm not connected</h1>
        <app-error-notice [error]="result.error()" />
        <div><a class="button" routerLink="/settings">Back to settings</a></div>
      } @else {
        <app-loading />
      }
    </div>
  `,
})
export class LastfmCallbackPage {
  private readonly lastfm = inject(LastfmService);
  /** From `?state=`, as the server put it in the callback URL. */
  readonly state = input<string>();
  /** From `?token=`. */
  readonly token = input<string>();
  protected readonly result = resource({
    params: () => ({ state: this.state() ?? '', token: this.token() ?? '' }),
    loader: ({ params }) =>
      params.token
        ? this.lastfm.finishAuth(params.state, params.token)
        : Promise.reject(new ApiError('token_not_found', 'Last.fm sent no token.', 400)),
  });
}
