import { inject } from '@angular/core';
import { CanActivateFn, Router, UrlTree } from '@angular/router';

import { Account } from './api/models';
import { SessionService } from './api/session.service';

/** Where an account belongs after logging in. */
export function homeFor(account: Account): string {
  return account.status === 'pending' ? '/pending' : '/';
}

/** A return path from the query, kept only when it stays on this site. */
export function safeNext(next: string | null | undefined): string | null {
  return next?.startsWith('/') && !next.startsWith('//') ? next : null;
}

async function current(session: SessionService): Promise<Account | null> {
  try {
    return await session.load();
  } catch {
    return null;
  }
}

/** Login and sign-up: only when logged out. */
export const loggedOutGuard: CanActivateFn = async () => {
  const [session, router] = [inject(SessionService), inject(Router)];
  const account = await current(session);
  return account ? router.parseUrl(homeFor(account)) : true;
};

/** The waiting screen: only for pending accounts. */
export const pendingGuard: CanActivateFn = async () => {
  const [session, router] = [inject(SessionService), inject(Router)];
  const account = await current(session);
  if (!account) {
    return router.parseUrl('/login');
  }
  return account.status === 'pending' ? true : router.parseUrl('/');
};

async function activeOr(
  session: SessionService,
  router: Router,
  url: string,
): Promise<Account | UrlTree> {
  const account = await current(session);
  if (!account) {
    return router.createUrlTree(['/login'], { queryParams: { next: url } });
  }
  return account.status === 'active' ? account : router.parseUrl(homeFor(account));
}

/** Screens for active accounts; others go to login (and come back) or to the waiting screen. */
export const activeGuard: CanActivateFn = async (_route, state) => {
  const result = await activeOr(inject(SessionService), inject(Router), state.url);
  return result instanceof UrlTree ? result : true;
};

/** The admin screen. */
export const adminGuard: CanActivateFn = async (_route, state) => {
  const router = inject(Router);
  const result = await activeOr(inject(SessionService), router, state.url);
  if (result instanceof UrlTree) {
    return result;
  }
  return result.role === 'admin' ? true : router.parseUrl('/');
};
