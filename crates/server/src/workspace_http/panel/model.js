// SPDX-License-Identifier: AGPL-3.0-or-later
// No persistent browser storage. A session signature includes its CSRF token,
// so even another login using the same key invalidates this page's intents.
export const signature = session => JSON.stringify([
  session.instance_id, session.identity.key_id, session.identity.principal_id,
  session.identity.scope, session.csrf_token,
]);

export function canWrite(session, workspace) {
  return !workspace.archived && session.identity.scope.kind === 'workspace'
    && session.identity.scope.workspace_id === workspace.id;
}

export function exactRevision(value) {
  if (!Number.isSafeInteger(value) || value < 1) {
    throw new Error('This revision exceeds the browser’s exact numeric range. Use the CLI.');
  }
  return value;
}

export class Intent {
  constructor(session, workspace, change, operation = crypto.randomUUID()) {
    this.session = signature(session);
    this.proof = session.csrf_token;
    this.body = JSON.stringify({workspace_id: workspace, operation_id: operation, agent: null, change});
    Object.freeze(this);
  }
  request(session) {
    if (this.session !== signature(session)) {
      throw new Error('The session changed. This write cannot be retried with the new session.');
    }
    return {body: this.body, proof: this.proof};
  }
}

export function takeFragment(location, history) {
  const fragment = location.hash;
  // Scrub before any exchange, including malformed fragments. Never log it.
  if (fragment) history.replaceState(null, '', '/admin');
  const params = new URLSearchParams(fragment.slice(1));
  if (params.size !== 1 || !params.has('key')) return '';
  return params.get('key') || '';
}

export async function wire(path, body, proof) {
  let response;
  try {
    response = await fetch(path, {
      method: body === undefined ? 'GET' : 'POST',
      credentials: 'same-origin', cache: 'no-store', redirect: 'error',
      headers: body === undefined ? {} : {'Content-Type': 'application/json', 'X-CCP-CSRF': proof},
      body,
    });
  } catch {
    throw new Error('No confirmed response. Check your connection; the request may have completed.');
  }
  let envelope;
  try { envelope = await response.json(); }
  catch { throw new Error('The server response could not be verified. The outcome may be unknown.'); }
  if (!response.ok || envelope.api_version !== 1 || !('data' in envelope)) {
    const messages = {
      400: 'The request was refused. Check the fields and active session.',
      401: 'This session is no longer authorized. Copy unsaved work, then sign in again.',
      403: 'The current key cannot perform this action, or its session changed.',
      404: 'This item is unavailable. Refresh its parent list.',
      409: 'The content changed or needs conversion. Keep your draft and compare the current revision.',
      413: 'The request is too large. Shorten the text before creating a new write.',
      503: 'The server is unavailable. The request outcome may be unknown.',
    };
    const error = new Error(messages[response.status] || 'The request could not be verified.');
    error.status = response.status;
    throw error;
  }
  return envelope.data;
}
