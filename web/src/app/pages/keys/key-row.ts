import { ChangeDetectionStrategy, Component, input, output, signal } from '@angular/core';

import { ApiKey } from '../../api/models';
import { ConfirmButton } from '../../shared/confirm-button';
import { CopyButton } from '../../shared/copy-button';
import { formatTime } from '../../shared/format';

/** One key: its label, the key and secret to copy, and revoke. */
@Component({
  selector: 'app-key-row',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ConfirmButton, CopyButton],
  template: `
    @let key = apiKey();
    <article class="stack key" [attr.aria-label]="key.label">
      <div class="row">
        <h2 class="grow">{{ key.label }}</h2>
        <app-confirm-button
          label="Revoke"
          question="Apps using this key stop working."
          confirmLabel="Revoke key"
          [busy]="busy()"
          (confirmed)="revoked.emit()"
        />
      </div>
      <p class="muted small">
        {{ key.scope === 'read_write' ? 'Read and scrobble' : 'Read only' }} ·
        {{ key.lastUsed ? 'Last used ' + time(key.lastUsed) : 'Never used' }}
      </p>
      <div class="row">
        <span class="label small muted">Key</span>
        <span class="mono grow">{{ key.key }}</span>
        <app-copy-button [value]="key.key" ariaLabel="Copy the key" />
      </div>
      <div class="row">
        <span class="label small muted">Secret</span>
        <span class="mono grow">{{ shown() ? key.secret : '••••••••••••••••' }}</span>
        <button type="button" class="button text" (click)="shown.set(!shown())">
          {{ shown() ? 'Hide' : 'Reveal' }}
        </button>
        <app-copy-button [value]="key.secret" ariaLabel="Copy the secret" />
      </div>
    </article>
  `,
  styles: `
    .key {
      gap: var(--space-1);
      padding-top: var(--space-4);
      border-top: 1px solid var(--line);
    }
    .label {
      width: 48px;
    }
  `,
})
export class KeyRow {
  readonly apiKey = input.required<ApiKey>();
  readonly busy = input(false);
  readonly revoked = output();
  protected readonly shown = signal(false);
  protected readonly time = formatTime;
}
