import { ChangeDetectionStrategy, Component, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { KeyScope } from '../../api/models';
import { ErrorNotice } from '../../shared/error-notice';

/** A label and a scope for a new key. */
@Component({
  selector: 'app-key-form',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, ErrorNotice],
  template: `
    <form class="stack" (ngSubmit)="submit()">
      <div class="row fields">
        <label class="field grow">
          Label
          <input
            name="label"
            required
            maxlength="64"
            placeholder="Phone scrobbler"
            [(ngModel)]="label"
          />
        </label>
        <label class="field">
          Access
          <select name="scope" [(ngModel)]="scope">
            <option value="read">Read only</option>
            <option value="read_write">Read and scrobble</option>
          </select>
        </label>
      </div>
      <app-error-notice [error]="error()" />
      <button type="submit" class="button primary" [disabled]="busy() || !label().trim()">
        Create key
      </button>
    </form>
  `,
  styles: `
    .fields {
      align-items: end;
    }
    .grow {
      flex: 1 1 200px;
    }
  `,
})
export class KeyForm {
  readonly busy = input(false);
  readonly error = input<unknown>(null);
  readonly created = output<{ label: string; scope: KeyScope }>();
  protected readonly label = signal('');
  protected readonly scope = signal<KeyScope>('read');

  protected submit(): void {
    this.created.emit({ label: this.label().trim(), scope: this.scope() });
    this.label.set('');
  }
}
