// SPDX-License-Identifier: AGPL-3.0-or-later
// Thin client for the CCP HTTP admin API. Every call carries the admin key in
// the `x-ccp-admin-key` header; the server keeps no browser session.

export class ApiError extends Error {
  status: number;
  constructor(message: string, status: number) { super(message); this.status = status; }
}

const MESSAGES: Record<number, string> = {
  400: 'The server refused this request. Check the fields and try again.',
  401: 'This admin key was not accepted.',
  404: 'Not found. It may have been deleted; refresh and try again.',
  413: 'The request is too large.',
  500: 'The server reported an internal error.',
  502: 'The server is unreachable right now.',
  503: 'The server is unavailable right now.',
};

export async function call<T>(key: string, path: string, init: {method?: string; body?: unknown} = {}): Promise<T> {
  const headers: Record<string, string> = {'x-ccp-admin-key': key};
  if (init.body !== undefined) headers['content-type'] = 'application/json';
  let response: Response;
  try {
    response = await fetch(path, {
      method: init.method ?? 'GET',
      headers,
      body: init.body === undefined ? undefined : JSON.stringify(init.body),
      credentials: 'omit',
      cache: 'no-store',
      redirect: 'error',
    });
  } catch {
    throw new ApiError('No response from the server. Check your connection; a write may still have completed.', 0);
  }
  let data: unknown = null;
  try { data = await response.json(); } catch { /* empty or non-JSON body */ }
  if (!response.ok) {
    const detail = data && typeof data === 'object' && 'error' in data ? String((data as {error: unknown}).error) : '';
    throw new ApiError(detail && response.status !== 401 ? detail : (MESSAGES[response.status] ?? `Request failed (${response.status}).`), response.status);
  }
  return data as T;
}

export const enc = encodeURIComponent;

/** Unix seconds from the API's mixed timestamp formats (s, ms or SQLite UTC datetime). */
export function seconds(value: string | number | null | undefined): number {
  if (value === null || value === undefined || value === '') return 0;
  if (typeof value === 'number') return value > 1e12 ? value / 1000 : value;
  if (/^\d+(\.\d+)?$/.test(value)) { const n = Number(value); return n > 1e12 ? n / 1000 : n; }
  const parsed = Date.parse(value.includes('T') ? value : `${value.replace(' ', 'T')}Z`);
  return Number.isNaN(parsed) ? 0 : parsed / 1000;
}

export function utf8Bytes(text: string): number { return new TextEncoder().encode(text).length; }

/** Pretty-print text when it is a JSON object or array; otherwise return it unchanged. */
export function prettyJson(text: string): {pretty: string; json: boolean} {
  const trimmed = text.trim();
  if (!(trimmed.startsWith('{') || trimmed.startsWith('['))) return {pretty: text, json: false};
  try { return {pretty: JSON.stringify(JSON.parse(trimmed), null, 2), json: true}; }
  catch { return {pretty: text, json: false}; }
}
