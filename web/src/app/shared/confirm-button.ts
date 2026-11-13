import { ChangeDetectionStrategy, Component, input, output, signal } from '@angular/core';

/** A quiet button that asks once more before emitting `confirmed`. */
@Component({
  selector: 'app-confirm-button',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (asking()) {
      <span class="row" role="group" [attr.aria-label]="question()">
        <span class="small">{{ question() }}</span>
        <button type="button" class="button text danger" [disabled]="busy()" (click)="confirm()">
          {{ confirmLabel() }}
        </button>
        <button type="button" class="button text" (click)="asking.set(false)">Cancel</button>
      </span>
    } @else {
      <button type="button" class="button text" [disabled]="busy()" (click)="asking.set(true)">
        {{ label() }}
      </button>
    }
  `,
})
export class ConfirmButton {
  readonly label = input.required<string>();
  readonly question = input.required<string>();
  readonly confirmLabel = input.required<string>();
  readonly busy = input(false);
  readonly confirmed = output();
  protected readonly asking = signal(false);

  protected confirm(): void {
    this.asking.set(false);
    this.confirmed.emit();
  }
}
