import { ChangeDetectionStrategy, Component, inject, input, resource } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiError } from '../api/api-error';
import { LastfmService } from '../api/lastfm.service';
import { ErrorNotice } from '../shared/error-notice';
import { Loading } from '../shared/loading';

/** Where last.fm sends the browser back with `?token=` after the user approved this server. */
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
  /** From `?token=`. */
  readonly token = input<string>();
  protected readonly result = resource({
    params: () => this.token() ?? '',
    loader: ({ params }) =>
      params
        ? this.lastfm.finishAuth(params)
        : Promise.reject(new ApiError('token_not_found', 'Last.fm sent no token.', 400)),
  });
}
