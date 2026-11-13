/**
 * Error codes the web API sends in `{"error": {"code", "message"}}`, plus `network` for no answer
 * and `import_failed` for an import that stopped.
 */
export type ErrorCode =
  | 'account_pending'
  | 'account_disabled'
  | 'admin_only'
  | 'csrf_header_missing'
  | 'demo_import_running'
  | 'demo_request_failed'
  | 'demo_unavailable'
  | 'history_too_large'
  | 'import_failed'
  | 'internal'
  | 'invalid_credentials'
  | 'invalid_key'
  | 'invalid_label'
  | 'invalid_username'
  | 'last_admin'
  | 'lastfm_error'
  | 'lastfm_not_configured'
  | 'lastfm_unavailable'
  | 'lastfm_user_not_found'
  | 'lastfm_user_taken'
  | 'name_taken'
  | 'network'
  | 'not_found'
  | 'not_logged_in'
  | 'not_mirrored'
  | 'rate_limited'
  | 'signup_closed'
  | 'token_expired'
  | 'token_not_found'
  | 'token_taken'
  | 'unknown_scenario'
  | 'weak_password';

/** Our wording for codes whose server message needs context in the UI. */
const MESSAGES: Partial<Record<ErrorCode, string>> = {
  account_pending: 'Your account is waiting for an admin to approve it.',
  account_disabled: 'Your account is disabled. Ask an admin to enable it.',
  not_logged_in: 'Your session ended. Log in again.',
  rate_limited: 'Too many attempts. Wait a few minutes and try again.',
  csrf_header_missing: 'The request was blocked as a possible forgery. Reload the page.',
  signup_closed: 'Sign-up is closed on this server. Ask an admin to open it.',
  name_taken: 'That username is taken. Pick another one.',
  last_admin: 'This is the last active admin. Make someone else an admin first.',
  lastfm_user_taken: 'Another account on this server already links that Last.fm user.',
  token_not_found: 'This request is unknown or has expired. Start again from the app.',
  token_expired: 'This request has expired. Start again from the app.',
  lastfm_not_configured:
    'This server has no Last.fm shared secret. Add your own Last.fm API key in settings first.',
  network: 'The server did not answer. Check your connection and try again.',
  internal: 'Something went wrong on the server. Try again later.',
};

/** A failed web API call. */
export class ApiError extends Error {
  constructor(
    readonly code: ErrorCode | string,
    readonly serverMessage: string,
    readonly status: number,
  ) {
    super(MESSAGES[code as ErrorCode] ?? serverMessage);
    this.name = 'ApiError';
  }

  /** Whether this is the given code. */
  is(code: ErrorCode): boolean {
    return this.code === code;
  }
}

/** Turns anything a call threw into an [ApiError]. */
export function toApiError(error: unknown): ApiError {
  if (error instanceof ApiError) {
    return error;
  }
  return new ApiError('network', String(error), 0);
}
