import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ErrorNotice } from '../../shared/error-notice';
import { formatCount } from '../../shared/format';
import { Loading } from '../../shared/loading';
import { DemoTabs } from './demo-tabs';
import { EXAMPLE_USER } from './example-user';
import { ResultRow } from './result-row';
import { DemoRunner } from './runner';
import { scaleFor, totalRow } from './scale';

/** Times the same Last.fm API calls on Last.fm and on this mirror, side by side. */
@Component({
  selector: 'app-demo-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [DemoTabs, ErrorNotice, FormsModule, Loading, ResultRow],
  providers: [DemoRunner],
  templateUrl: './demo.html',
  styleUrl: './demo.css',
})
export class DemoPage {
  protected readonly runner = inject(DemoRunner);
  protected readonly username = signal(EXAMPLE_USER);
  protected readonly scale = computed(() =>
    scaleFor(this.runner.rows().flatMap((row) => (row.outcome ? [row.outcome] : []))),
  );
  protected readonly total = computed(() =>
    this.runner.phase() === 'done' ? totalRow(this.runner.rows()) : null,
  );

  protected compare(): void {
    void this.runner.compare(this.username().trim());
  }

  protected count(value: number): string {
    return formatCount(value);
  }
}
