import { ChangeDetectionStrategy, Component } from '@angular/core';

/** What a section shows while its data loads. */
@Component({
  selector: 'app-loading',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<p class="muted" role="status">Loading…</p>`,
})
export class Loading {}
