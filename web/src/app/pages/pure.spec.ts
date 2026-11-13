import { Account, LinkStatus } from '../api/models';
import { byPendingFirst } from './admin/admin';
import { importing } from './home/home';

const writes = { forwarding: false, queued: 0, failed: 0, heldLocally: 0, lastError: null };
const linked: LinkStatus = {
  username: 'syntiiix',
  authorized: false,
  ownKey: false,
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
