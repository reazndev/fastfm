import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { RouterLink } from '@angular/router';

import { LinkStatus } from '../../api/models';
import { formatCount, formatTime } from '../../shared/format';

/** Import progress, live sync and write forwarding for the linked Last.fm user. */
@Component({
  selector: 'app-link-status',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink],
  templateUrl: './link-status.html',
  styles: `
    :host {
      display: flex;
      flex-direction: column;
      gap: var(--space-8);
    }
    progress {
      width: 100%;
      height: 6px;
      accent-color: var(--text);
    }
  `,
})
export class LinkStatusView {
  readonly status = input.required<LinkStatus>();
  protected readonly count = formatCount;
  protected readonly time = formatTime;
  protected readonly profileUrl = computed(
    () => `https://www.last.fm/user/${encodeURIComponent(this.status().username ?? '')}`,
  );
  /** Pending, failed and held writes, only the ones there are. */
  protected readonly backlog = computed(() => {
    const writes = this.status().writes;
    return [
      { label: 'Waiting to send', value: writes.queued },
      { label: 'Failed', value: writes.failed },
      { label: 'Kept here only', value: writes.heldLocally },
    ].filter((item) => item.value > 0);
  });
}
