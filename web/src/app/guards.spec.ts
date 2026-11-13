import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import {
  ActivatedRouteSnapshot,
  RouterStateSnapshot,
  UrlTree,
  provideRouter,
} from '@angular/router';

import { Account } from './api/models';
import { activeGuard, adminGuard, homeFor, safeNext } from './guards';

const account: Account = {
  id: 1,
  username: 'robin',
  role: 'user',
  status: 'active',
  lastfmUser: null,
  createdAt: 0,
};

/** Runs `guard` for `url` while `/me` answers with `me` (or 401 for `null`). */
async function run(guard: typeof activeGuard, url: string, me: Account | null): Promise<string> {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), provideHttpClient(), provideHttpClientTesting()],
  });
  const state = { url } as RouterStateSnapshot;
  const result = TestBed.runInInjectionContext(() =>
    guard({} as ActivatedRouteSnapshot, state),
  ) as Promise<boolean | UrlTree>;
  const request = TestBed.inject(HttpTestingController).expectOne('/app/api/me');
  if (me) {
    request.flush({ account: me });
  } else {
    const body = { error: { code: 'not_logged_in', message: 'Log in first.' } };
    request.flush(body, { status: 401, statusText: 'Unauthorized' });
  }
  const outcome = await result;
  return outcome === true ? 'allowed' : String(outcome);
}

describe('guards', () => {
  it('sends logged-out visitors to login and back', async () => {
    expect(await run(activeGuard, '/keys', null)).toBe('/login?next=%2Fkeys');
  });

  it('sends pending accounts to the waiting screen', async () => {
    expect(await run(activeGuard, '/keys', { ...account, status: 'pending' })).toBe('/pending');
  });

  it('lets active accounts in and keeps users out of admin', async () => {
    expect(await run(activeGuard, '/keys', account)).toBe('allowed');
    TestBed.resetTestingModule();
    expect(await run(adminGuard, '/admin', account)).toBe('/');
    TestBed.resetTestingModule();
    expect(await run(adminGuard, '/admin', { ...account, role: 'admin' })).toBe('allowed');
  });

  it('only follows return paths on this site', () => {
    expect(safeNext('/authorize?token=t')).toBe('/authorize?token=t');
    expect(safeNext('//evil.example')).toBeNull();
    expect(safeNext('https://evil.example')).toBeNull();
    expect(homeFor({ ...account, status: 'pending' })).toBe('/pending');
  });
});
