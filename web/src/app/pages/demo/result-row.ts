import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { Row } from './runner';
import { Scale, barFraction, formatBytes, formatMs, speedup } from './scale';

interface Bar {
  label: string;
  kind: 'lastfm' | 'first' | 'cached';
  ms: number;
}

/** One scenario: its timings as bars on the shared scale, with the exact numbers. */
@Component({
  selector: 'app-result-row',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @let r = row();
    <div class="head">
      <h3>{{ r.scenario.title }}</h3>
      @if (factor(); as f) {
        <span class="factor">{{ f }} faster</span>
      }
    </div>
    <p class="mono small muted">{{ r.scenario.detail }}</p>
    @if (r.outcome; as o) {
      <div class="bars">
        @for (bar of bars(); track bar.kind) {
          <span class="label small">{{ bar.label }}</span>
          <span class="track">
            <span class="bar" [class]="bar.kind" [style.--w]="width(bar.ms)"></span>
            <span class="value small">{{ ms(bar.ms) }}</span>
          </span>
        }
      </div>
      @if (!o.mirror) {
        <p class="small muted">
          The mirror sent this to Last.fm ({{ o.mirrorSource }}), so there is no local time.
        </p>
      }
      <p class="small muted">
        {{ size() }}
        @if (!total() && o.mirror) {
          · {{ o.identical ? 'Identical' : 'Differs' }}
        }
        @if (!o.lastfm.ok) {
          · Last.fm answered with an error
        }
      </p>
    } @else {
      <p class="small muted" role="status">{{ active() ? 'Measuring…' : 'Waiting' }}</p>
    }
  `,
  styleUrl: './result-row.css',
})
export class ResultRow {
  readonly row = input.required<Row>();
  readonly scale = input.required<Scale>();
  /** Whether this scenario is being measured now. */
  readonly active = input(false);
  /** Whether this is the total of every scenario. */
  readonly total = input(false);

  protected readonly bars = computed<Bar[]>(() => {
    const outcome = this.row().outcome;
    if (!outcome) {
      return [];
    }
    const lastfm: Bar = { label: 'Last.fm', kind: 'lastfm', ms: outcome.lastfm.ms };
    const mirror = outcome.mirror;
    return mirror
      ? [
          lastfm,
          { label: 'Mirror', kind: 'first', ms: mirror.firstMs },
          { label: 'Cached', kind: 'cached', ms: mirror.cachedMs },
        ]
      : [lastfm];
  });

  protected readonly factor = computed(() => {
    const outcome = this.row().outcome;
    return outcome?.mirror ? speedup(outcome.lastfm.ms, outcome.mirror.firstMs) : null;
  });

  protected readonly size = computed(() => {
    const outcome = this.row().outcome;
    return outcome ? formatBytes(outcome.mirror?.bytes ?? outcome.lastfm.bytes) : '';
  });

  protected width(ms: number): number {
    return barFraction(ms, this.scale());
  }

  protected ms(value: number): string {
    return formatMs(value);
  }
}
