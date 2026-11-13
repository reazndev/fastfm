import { ChangeDetectionStrategy, Component, input } from '@angular/core';
import { RouterLink } from '@angular/router';

import { CopyButton } from '../../shared/copy-button';

/** How to point a Last.fm client at this server. */
@Component({
  selector: 'app-client-setup',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CopyButton, RouterLink],
  template: `
    <section class="section" aria-labelledby="client-heading">
      <h2 id="client-heading">Use this server</h2>
      <p class="muted">
        In your app or scrobbler, replace
        <span class="mono">https://ws.audioscrobbler.com/2.0/</span> with this address, and use an
        <a routerLink="/keys">API key</a> from this server.
      </p>
      <div class="row notice">
        <span class="mono grow">{{ baseUrl() }}</span>
        <app-copy-button [value]="baseUrl()" ariaLabel="Copy the API address" />
      </div>
    </section>
  `,
})
export class ClientSetup {
  readonly baseUrl = input.required<string>();
}
