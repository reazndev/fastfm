/** Shapes the web API at `/app/api` returns, as `crates/web/src/views.rs` and the handlers define them. */

export type Role = 'admin' | 'user';
export type AccountStatus = 'pending' | 'active' | 'disabled';
export type KeyScope = 'read' | 'read_write';
export type SignupMode = 'approval' | 'open' | 'closed';
export type JobStatus = 'queued' | 'running' | 'done' | 'failed';
/** An import's status; `private` when the profile hides its listening history. */
export type ImportStatus = JobStatus | 'private';

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
  status: ImportStatus;
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
  credentials: {
    /** Whether this server asks every account for its own Last.fm key before linking. */
    ownKeyRequired: boolean;
    /** Set when saved credentials could not be read and must be entered again. */
    error: string | null;
  };
  import: ImportView | null;
  sync: SyncView | null;
  writes: WritesView;
}

export interface InstanceSettings {
  signupMode: SignupMode;
  requireOwnKey: boolean;
  lastfmKeyPool: { keys: number; canSign: boolean };
}

/** What the sign-up form will do, from the public `GET /signup-mode`. */
export interface SignupInfo {
  mode: SignupMode;
  hasAccounts: boolean;
  setupTokenRequired: boolean;
}

export interface AuthRequest {
  apiKey: string;
  label: string | null;
  owner: string | null;
  expiresAt: number;
  approved: boolean;
}

/** Where a Last.fm user stands for the speed comparison. */
export type DemoState = 'mirrored' | 'importing' | 'not_mirrored';

export interface DemoScenario {
  id: string;
  title: string;
  /** The method and parameters, such as `user.getRecentTracks limit=200 page=12`. */
  detail: string;
  requests: number;
}

export interface DemoPrepared {
  /** The name as Last.fm spells it. */
  username: string;
  state: DemoState;
  playcount: number;
  maxScrobbles: number;
  import: ImportView | null;
  /** Empty unless the user is mirrored. */
  scenarios: DemoScenario[];
}

/** One scenario's timings; every number is the total over its requests. */
export interface DemoOutcome {
  id: string;
  requests: number;
  lastfm: { ms: number; bytes: number; ok: boolean };
  /** Null when the mirror did not answer from local data. */
  mirror: { firstMs: number; cachedMs: number; bytes: number; ok: boolean } | null;
  mirrorSource: string;
  identical: boolean;
}

/** Which API a live race page comes from. */
export type RaceSide = 'lastfm' | 'mirror';

/** One scrobble in the live race. */
export interface RaceTrack {
  name: string;
  artist: string;
  /** The largest cover, or empty when there is none. */
  image: string;
  uts: number;
}

/** One page of history in the live race, trimmed to its oldest scrobbles. */
export interface RacePage {
  /** How long the request took, measured on the server. */
  ms: number;
  /** Scrobbles on the page, without a track playing now. */
  count: number;
  /** Scrobbles in the whole history. */
  total: number;
  totalPages: number;
  /** The page's last scrobbles, newest first. */
  tracks: RaceTrack[];
}
