import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { ErrorNotice } from '../../shared/error-notice';
import { formatCount, formatTime } from '../../shared/format';
import { Lane } from './race-runner';

const percent = new Intl.NumberFormat(undefined, {
  style: 'percent',
  maximumFractionDigits: 1,
});
const seconds = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });

/** One side of the race: how far it got, and the newest scrobbles it fetched. */
@Component({
  selector: 'app-race-lane',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ErrorNotice],
  templateUrl: './race-lane.html',
  styleUrl: './race-lane.css',
})
export class RaceLane {
  readonly title = input.required<string>();
  readonly lane = input.required<Lane>();

  protected readonly share = computed(() => {
    const { fetched, total } = this.lane();
    return total > 0 ? percent.format(fetched / total) : '–';
  });
  protected readonly time = computed(() => `${seconds.format(this.lane().ms / 1000)} s`);

  protected count(value: number): string {
    return formatCount(value);
  }

  protected date(uts: number): string {
    return formatTime(uts);
  }
}
