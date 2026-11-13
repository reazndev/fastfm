import { HttpClient, HttpErrorResponse, HttpHeaders, HttpParams } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { firstValueFrom, Observable } from 'rxjs';

import { ApiError } from './api-error';

/** Where the web API is mounted. */
export const API_BASE = '/app/api';

type Params = Record<string, string>;

/**
 * The one client for `/app/api`. Every request carries `X-Requested-With`, which the server
 * requires on state-changing calls, and every failure becomes an {@link ApiError}.
 */
@Injectable({ providedIn: 'root' })
export class ApiClient {
  private readonly http = inject(HttpClient);
  private readonly headers = new HttpHeaders({ 'X-Requested-With': 'XMLHttpRequest' });

  get<T>(path: string, params: Params = {}): Promise<T> {
    const options = { headers: this.headers, params: new HttpParams({ fromObject: params }) };
    return this.send(this.http.get<T>(API_BASE + path, options));
  }

  post<T = void>(path: string, body: unknown = {}): Promise<T> {
    return this.send(this.http.post<T>(API_BASE + path, body, { headers: this.headers }));
  }

  put<T = void>(path: string, body: unknown): Promise<T> {
    return this.send(this.http.put<T>(API_BASE + path, body, { headers: this.headers }));
  }

  delete(path: string): Promise<void> {
    return this.send(this.http.delete<void>(API_BASE + path, { headers: this.headers }));
  }

  private async send<T>(request: Observable<T>): Promise<T> {
    try {
      return await firstValueFrom(request);
    } catch (error) {
      throw fromHttp(error);
    }
  }
}

interface ErrorBody {
  error?: { code?: unknown; message?: unknown };
}

function fromHttp(error: unknown): ApiError {
  if (!(error instanceof HttpErrorResponse)) {
    return new ApiError('network', String(error), 0);
  }
  if (error.status === 0) {
    return new ApiError('network', error.message, 0);
  }
  const body = (error.error ?? {}) as ErrorBody;
  const code = body.error?.code;
  const message = body.error?.message;
  if (typeof code === 'string') {
    return new ApiError(code, typeof message === 'string' ? message : code, error.status);
  }
  return new ApiError('internal', error.message, error.status);
}
