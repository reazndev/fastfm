/** Shapes the web API at `/app/api` returns, as `crates/web/src/views.rs` and the handlers define them. */

export type Role = 'admin' | 'user';
export type AccountStatus = 'pending' | 'active' | 'disabled';
export type KeyScope = 'read' | 'read_write';
export type SignupMode = 'approval' | 'open' | 'closed';
export type JobStatus = 'queued' | 'running' | 'done' | 'failed';

export interface Account {
  id: number;
  username: string;
  role: Role;
  status: AccountStatus;
  lastfmUser: string | null;
  createdAt: number;
}

export interface ApiKey {
  key: string;
  secret: string;
  label: string;
  scope: KeyScope;
  createdAt: number;
  lastUsed: number | null;
}

export interface ImportView {
  status: JobStatus;
  imported: number;
  estimatedTotal: number;
  error: string | null;
}

export interface SyncView {
  /** Everything before this Unix time is mirrored. */
  cursor: number;
  scrobbles: number;
}

export interface WritesView {
  forwarding: boolean;
  queued: number;
  failed: number;
  heldLocally: number;
  lastError: string | null;
}

export interface LinkStatus {
  username: string | null;
  authorized: boolean;
  ownKey: boolean;
  import: ImportView | null;
  sync: SyncView | null;
  writes: WritesView;
}

export interface InstanceSettings {
  signupMode: SignupMode;
  lastfmKeyPool: { keys: number; canSign: boolean };
}

export interface AuthRequest {
  apiKey: string;
  label: string | null;
  owner: string | null;
  expiresAt: number;
  approved: boolean;
}
