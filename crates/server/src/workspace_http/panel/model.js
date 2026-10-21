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

export function canPublish(session, scope, workspace) {
  if (scope.kind === 'global') return session.identity.scope.kind === 'instance';
  return scope.kind === 'workspace' && workspace?.id === scope.workspace_id
    && canWrite(session, workspace);
}

function guidanceRevision(value) {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error('Guidance revision is outside the browser’s exact numeric range. Use the CLI.');
  return value;
}

function scopeKey(scope) {
  if (scope?.kind === 'global') return 'global';
  if (scope?.kind === 'workspace' && typeof scope.workspace_id === 'string') return 'workspace:' + scope.workspace_id;
  throw new Error('The guidance scope could not be verified.');
}

export function guidanceBundle(bundle, workspaceId) {
  if (bundle?.precedence !== 'global_first' || scopeKey(bundle.global?.scope) !== 'global'
    || (workspaceId ? scopeKey(bundle.workspace?.scope) !== 'workspace:' + workspaceId : bundle.workspace !== null)) {
    throw new Error('The guidance response did not match the requested scope.');
  }
  for (const board of [bundle.global, bundle.workspace].filter(Boolean)) {
    guidanceRevision(board.revision);
    if (typeof board.text !== 'string' || new TextEncoder().encode(board.text).length > 8192) {
      throw new Error('The guidance text could not be verified.');
    }
  }
  return bundle;
}

export class GuidanceIntent {
  constructor(session, scope, revision, text, operation = crypto.randomUUID()) {
    guidanceRevision(revision);
    scopeKey(scope);
    if (revision === Number.MAX_SAFE_INTEGER || typeof text !== 'string' || new TextEncoder().encode(text).length > 8192) {
      throw new Error('Guidance needs at most 8192 UTF-8 bytes and an exactly representable next revision.');
    }
    this.session = signature(session);
    this.proof = session.csrf_token;
    this.body = JSON.stringify({scope, operation_id: operation, expected_revision: revision, text});
    Object.freeze(this);
  }
  request(session) {
    if (this.session !== signature(session)) throw new Error('The session changed. This guidance save cannot be retried with the new session.');
    return {body: this.body, proof: this.proof};
  }
  receipt(value) {
    const original = JSON.parse(this.body);
    if (scopeKey(value?.scope) !== scopeKey(original.scope) || value.operation_id !== original.operation_id
      || guidanceRevision(value.revision) !== original.expected_revision + 1) {
      throw new Error('The guidance save response could not be verified. Keep the original operation.');
    }
    return value.revision;
  }
}

export function canChangeLifecycle(session, workspace) {
  const scope = session.identity.scope;
  return !!workspace && (scope.kind === 'instance'
    || scope.kind === 'workspace' && scope.workspace_id === workspace.id && scope.role === 'owner');
}

export function retentionSeconds(value, unit) {
  const n = Number(value), scale = Number(unit), seconds = n * scale;
  if (!Number.isSafeInteger(n) || n <= 0 || ![1, 3600, 86400].includes(scale)
    || !Number.isSafeInteger(seconds) || seconds <= 0) {
    throw new Error('Choose a positive whole duration within the browser’s exact numeric range.');
  }
  return seconds;
}

export function retentionPolicy(value, workspaceId) {
  if (value?.workspace_id !== workspaceId || !Number.isSafeInteger(value.revision) || value.revision < 0
    || value.retention_seconds !== null && (!Number.isSafeInteger(value.retention_seconds) || value.retention_seconds <= 0)
    || (value.revision === 0 ? value.retention_seconds !== null || value.changed_at !== null
      : !Number.isSafeInteger(value.changed_at) || value.changed_at < 0)) {
    throw new Error('The retention policy could not be verified. No default policy was substituted.');
  }
  return value;
}

export function workspaceSnapshot(value, workspaceId) {
  if (value?.workspace?.id !== workspaceId || typeof value.workspace.name !== 'string'
    || typeof value.workspace.archived !== 'boolean') throw new Error('The current workspace state could not be verified.');
  return value.workspace;
}

export class RetentionIntent {
  constructor(session, policy, seconds, confirmTimed, operation = crypto.randomUUID()) {
    retentionPolicy(policy, policy.workspace_id);
    if (policy.revision >= Number.MAX_SAFE_INTEGER || typeof confirmTimed !== 'boolean'
      || (seconds === null ? confirmTimed : !Number.isSafeInteger(seconds) || seconds <= 0 || !confirmTimed)
      || (seconds === null) === (policy.retention_seconds === null)) {
      throw new Error('Enable cleanup only from manual retention, with an explicit duration and confirmation. Disable an existing policy before changing its duration.');
    }
    this.kind = 'retention'; this.workspace_id = policy.workspace_id;
    this.session = signature(session); this.proof = session.csrf_token;
    this.body = JSON.stringify({workspace_id: policy.workspace_id, operation_id: operation,
      expected_revision: policy.revision, retention_seconds: seconds, confirm_timed_purge: confirmTimed});
    Object.freeze(this);
  }
  request(session) {
    if (this.session !== signature(session)) throw new Error('The session changed. This retention change cannot be retried under the new session.');
    return {body: this.body, proof: this.proof};
  }
  receipt(value) {
    const original = JSON.parse(this.body);
    const policy = retentionPolicy(value?.policy, this.workspace_id);
    if (value.operation_id !== original.operation_id || policy.revision !== original.expected_revision + 1
      || policy.retention_seconds !== original.retention_seconds) throw new Error('The retention receipt could not be verified. Keep the original change.');
    return policy;
  }
}

export class LifecycleAttempt {
  #sent = false;
  constructor(session, workspace) {
    workspaceSnapshot({workspace}, workspace.id);
    this.kind = 'lifecycle'; this.workspace_id = workspace.id;
    this.archived = !workspace.archived;
    this.path = '/api/v1/admin/workspaces/' + (this.archived ? 'archive' : 'reactivate');
    this.requestBody = JSON.stringify({workspace_id: workspace.id});
    this.body = JSON.stringify({path: this.path, request: JSON.parse(this.requestBody), target_archived: this.archived});
    this.session = signature(session); this.proof = session.csrf_token;
    Object.freeze(this);
  }
  request(session) {
    if (this.session !== signature(session)) throw new Error('The session changed. Do not send this lifecycle request under the new session.');
    if (this.#sent) throw new Error('Do not retry this lifecycle request. Read current state before deciding on another action.');
    this.#sent = true;
    return {body: this.requestBody, proof: this.proof};
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
