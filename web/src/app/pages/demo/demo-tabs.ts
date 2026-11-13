import { ChangeDetectionStrategy, Component } from '@angular/core';
import { RouterLink, RouterLinkActive } from '@angular/router';

/** Switches between the two speed pages. */
@Component({
  selector: 'app-demo-tabs',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, RouterLinkActive],
  template: `
    <nav class="row" aria-label="Speed">
      <a routerLink="/demo" routerLinkActive="current" [routerLinkActiveOptions]="{ exact: true }"
        >Benchmarks</a
      >
      <a routerLink="/demo/live" routerLinkActive="current">Live race</a>
    </nav>
  `,
  styles: `
    a {
      color: var(--muted);
      text-decoration: none;
    }

    a.current,
    a:hover {
      color: var(--text);
    }
  `,
})
export class DemoTabs {}
