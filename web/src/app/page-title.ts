import { Injectable, inject } from '@angular/core';
import { Title } from '@angular/platform-browser';
import { RouterStateSnapshot, TitleStrategy } from '@angular/router';

/** `Keys · lfm-mirror` in the tab. */
@Injectable({ providedIn: 'root' })
export class PageTitle extends TitleStrategy {
  private readonly title = inject(Title);

  override updateTitle(snapshot: RouterStateSnapshot): void {
    const page = this.buildTitle(snapshot);
    this.title.setTitle(page ? `${page} · lfm-mirror` : 'lfm-mirror');
  }
}
