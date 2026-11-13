import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';

import { ApiClient } from './api-client';
import { ApiError } from './api-error';

function setup(): { client: ApiClient; http: HttpTestingController } {
  TestBed.configureTestingModule({
    providers: [provideHttpClient(), provideHttpClientTesting()],
  });
  return { client: TestBed.inject(ApiClient), http: TestBed.inject(HttpTestingController) };
}

async function failure(promise: Promise<unknown>): Promise<ApiError> {
  try {
    await promise;
  } catch (error) {
    if (error instanceof ApiError) {
      return error;
    }
  }
  throw new Error('expected an ApiError');
}

describe('ApiClient', () => {
  it('sends the CSRF header on every request', async () => {
    const { client, http } = setup();
    const done = client.post<{ ok: boolean }>('/keys', { label: 'x' });
    const request = http.expectOne('/app/api/keys');
    expect(request.request.headers.get('X-Requested-With')).toBe('XMLHttpRequest');
    request.flush({ ok: true });
    expect(await done).toEqual({ ok: true });
  });

  it('maps the error body to a typed error with our wording', async () => {
    const { client, http } = setup();
    const done = failure(client.post('/signup', {}));
    const body = { error: { code: 'signup_closed', message: 'Sign-up is closed.' } };
    http.expectOne('/app/api/signup').flush(body, { status: 403, statusText: 'Forbidden' });
    const error = await done;
    expect(error.is('signup_closed')).toBe(true);
    expect(error.status).toBe(403);
    expect(error.message).toContain('Ask an admin');
  });

  it('keeps the server message for codes without our own wording', async () => {
    const { client, http } = setup();
    const done = failure(client.put('/lastfm', {}));
    const body = { error: { code: 'lastfm_error', message: 'Last.fm said: User not found' } };
    http.expectOne('/app/api/lastfm').flush(body, { status: 502, statusText: 'Bad Gateway' });
    expect((await done).message).toBe('Last.fm said: User not found');
  });

  it('reports a request that got no answer as a network error', async () => {
    const { client, http } = setup();
    const done = failure(client.get('/me'));
    http.expectOne('/app/api/me').error(new ProgressEvent('error'));
    expect((await done).is('network')).toBe(true);
  });
});
