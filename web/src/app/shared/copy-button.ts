import { ChangeDetectionStrategy, Component, input, signal } from '@angular/core';

/** Copies `value` to the clipboard and says so for a moment. */
@Component({
  selector: 'app-copy-button',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <button type="button" class="button text" (click)="copy()" [attr.aria-label]="ariaLabel()">
      {{ copied() ? 'Copied' : 'Copy' }}
    </button>
  `,
})
export class CopyButton {
  readonly value = input.required<string>();
  readonly ariaLabel = input('Copy');
  protected readonly copied = signal(false);

  protected async copy(): Promise<void> {
    await navigator.clipboard.writeText(this.value());
    this.copied.set(true);
    setTimeout(() => this.copied.set(false), 1500);
  }
}
