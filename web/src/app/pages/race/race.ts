import { ChangeDetectionStrategy, Component, OnDestroy, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { DemoTabs } from '../demo/demo-tabs';
import { EXAMPLE_USER } from '../demo/example-user';
import { RaceLane } from './race-lane';
import { RaceRunner } from './race-runner';

/** Walks a mirrored user's whole history on Last.fm and on this mirror, side by side. */
@Component({
  selector: 'app-race-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [DemoTabs, FormsModule, RaceLane],
  providers: [RaceRunner],
  templateUrl: './race.html',
  styleUrl: './race.css',
})
export class RacePage implements OnDestroy {
  protected readonly runner = inject(RaceRunner);
  protected readonly username = signal(EXAMPLE_USER);

  protected toggle(): void {
    if (this.runner.running()) {
      this.runner.stop();
    } else {
      void this.runner.start(this.username().trim());
    }
  }

  ngOnDestroy(): void {
    this.runner.stop();
  }
}
