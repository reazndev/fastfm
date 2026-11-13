import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { RouterLink } from '@angular/router';

import { toApiError } from '../api/api-error';

/** Shows a failed call's message, with a way back in when the session ended. */
@Component({
  selector: 'app-error-notice',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink],
  template: `
    @if (apiError(); as e) {
      <p class="error" role="alert">
        {{ e.message }}
        @if (e.is('not_logged_in')) {
          <a routerLink="/login">Log in</a>
        }
      </p>
    }
  `,
  styles: `
    :host {
      display: contents;
    }
  `,
})
export class ErrorNotice {
  readonly error = input<unknown>(null);
  protected readonly apiError = computed(() => {
    const error = this.error();
    return error === null || error === undefined ? null : toApiError(error);
  });
}
