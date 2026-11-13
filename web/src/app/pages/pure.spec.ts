import { Account, LinkStatus } from '../api/models';
import { byPendingFirst } from './admin/admin';
import { signupHint } from './auth/signup';
import { importing, needsOwnKey } from './home/home';

const writes = { forwarding: false, queued: 0, failed: 0, heldLocally: 0, lastError: null };
const linked: LinkStatus = {
  username: 'syntiiix',
  authorized: false,
  ownKey: false,
  credentials: { ownKeyRequired: false, error: null },
  import: null,
  sync: null,
  writes,
};

describe('home', () => {
  it('polls until the import is done or failed', () => {
    const job = { imported: 1, estimatedTotal: 2, error: null };
    expect(importing({ ...linked, username: null })).toBe(false);
    expect(importing(linked)).toBe(true);
    expect(importing({ ...linked, import: { ...job, status: 'running' } })).toBe(true);
    expect(importing({ ...linked, import: { ...job, status: 'done' } })).toBe(false);
    expect(importing({ ...linked, import: { ...job, status: 'failed' } })).toBe(false);
    expect(importing({ ...linked, import: { ...job, status: 'private' } })).toBe(false);
  });

  it('asks for the own key first only when the server requires one', () => {
    const unlinked = { ...linked, username: null };
    const required = { ...unlinked, credentials: { ownKeyRequired: true, error: null } };
    expect(needsOwnKey(unlinked)).toBe(false);
    expect(needsOwnKey(required)).toBe(true);
    expect(needsOwnKey({ ...required, ownKey: true })).toBe(false);
    expect(needsOwnKey({ ...required, username: 'syntiiix' })).toBe(false);
  });
});

describe('sign-up', () => {
  it('says what happens to a new account', () => {
    const info = { mode: 'approval', hasAccounts: true, setupTokenRequired: false } as const;
    expect(signupHint({ ...info, hasAccounts: false })).toBe(
      'The first account becomes the admin.',
    );
    expect(signupHint(info)).toBe('An admin approves new accounts before they can be used.');
    expect(signupHint({ ...info, mode: 'open' })).toBeNull();
  });
});

describe('admin', () => {
  it('lists pending accounts first, then by age', () => {
    const base = { role: 'user', lastfmUser: null } as const;
    const accounts: Account[] = [
      { ...base, id: 1, username: 'a', status: 'active', createdAt: 1 },
      { ...base, id: 2, username: 'b', status: 'pending', createdAt: 3 },
      { ...base, id: 3, username: 'c', status: 'disabled', createdAt: 2 },
      { ...base, id: 4, username: 'd', status: 'pending', createdAt: 2 },
    ];
    expect(byPendingFirst(accounts).map((a) => a.username)).toEqual(['d', 'b', 'a', 'c']);
  });
});
