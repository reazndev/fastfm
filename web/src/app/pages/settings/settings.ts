import { ChangeDetectionStrategy, Component, inject, resource, signal } from '@angular/core';
import { Router } from '@angular/router';

import { LastfmService } from '../../api/lastfm.service';
import { SessionService } from '../../api/session.service';
import { ConfirmButton } from '../../shared/confirm-button';
import { ErrorNotice } from '../../shared/error-notice';
import { Loading } from '../../shared/loading';
import { OwnKey, OwnKeyForm } from './own-key-form';

/** Names the action in flight, so only its button shows busy. */
type Busy = 'connect' | 'key' | 'unlink' | null;

@Component({
  selector: 'app-settings-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ConfirmButton, ErrorNotice, Loading, OwnKeyForm],
  templateUrl: './settings.html',
})
export class SettingsPage {
  private readonly lastfm = inject(LastfmService);
  private readonly session = inject(SessionService);
  private readonly router = inject(Router);
  protected readonly status = resource({ loader: () => this.lastfm.status() });
  protected readonly busy = signal<Busy>(null);
  protected readonly error = signal<{ where: Busy; error: unknown } | null>(null);

  /** Sends the browser to last.fm, which comes back to `/lastfm/callback`. */
  protected connect(): Promise<void> {
    return this.run('connect', async () => {
      window.location.assign(await this.lastfm.authUrl());
    });
  }

  protected saveKey(key: OwnKey): Promise<void> {
    return this.run('key', () => this.lastfm.setOwnKey(key.apiKey, key.secret));
  }

  protected removeKey(): Promise<void> {
    return this.run('key', () => this.lastfm.clearOwnKey());
  }

  protected unlink(): Promise<void> {
    return this.run('unlink', () => this.lastfm.unlink());
  }

  protected async logout(): Promise<void> {
    await this.session.logout();
    await this.router.navigateByUrl('/login');
  }

  /** The error of the action named `where`, if it failed. */
  protected errorOf(where: Busy): unknown {
    const failed = this.error();
    return failed?.where === where ? failed.error : null;
  }

  private async run(where: Busy, action: () => Promise<void>): Promise<void> {
    this.busy.set(where);
    this.error.set(null);
    try {
      await action();
      this.status.reload();
    } catch (error) {
      this.error.set({ where, error });
    } finally {
      this.busy.set(null);
    }
  }
}
