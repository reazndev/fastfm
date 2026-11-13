import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';

import { SignupMode } from '../../api/models';

const MODES: { mode: SignupMode; label: string; hint: string }[] = [
  { mode: 'approval', label: 'Approval', hint: 'Anyone can sign up; you approve each account.' },
  { mode: 'open', label: 'Open', hint: 'Anyone can sign up and use the server at once.' },
  { mode: 'closed', label: 'Closed', hint: 'Nobody new can sign up.' },
];

/** The instance's sign-up mode as three radio choices. */
@Component({
  selector: 'app-signup-mode',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <fieldset class="stack">
      <legend class="muted small">Who can create an account</legend>
      @for (option of modes; track option.mode) {
        <label class="choice">
          <input
            type="radio"
            name="signup-mode"
            [value]="option.mode"
            [checked]="option.mode === mode()"
            (change)="picked.emit(option.mode)"
          />
          <span>
            <span class="name">{{ option.label }}</span>
            <span class="muted small">{{ option.hint }}</span>
          </span>
        </label>
      }
    </fieldset>
  `,
  styles: `
    fieldset {
      border: 0;
      padding: 0;
      margin: 0;
      gap: var(--space-2);
    }
    legend {
      padding: 0;
      margin-bottom: var(--space-2);
    }
    .choice {
      display: flex;
      gap: var(--space-3);
      align-items: baseline;
      cursor: pointer;
    }
    .choice input {
      min-height: 0;
    }
    .name {
      font-weight: 550;
      margin-right: var(--space-2);
    }
  `,
})
export class SignupModePicker {
  readonly mode = input.required<SignupMode>();
  readonly picked = output<SignupMode>();
  protected readonly modes = MODES;
}
