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

export function canManageTeam(session, workspace) {
  const scope = session.identity.scope;
  return !!workspace && scope.kind === 'workspace' && scope.workspace_id === workspace.id && scope.role === 'owner';
}

export function keyExpiry(value, now = Date.now()) {
  if (!value) return null;
  const seconds = new Date(value).getTime() / 1000;
  if (!Number.isSafeInteger(seconds) || seconds <= now / 1000) throw new Error('Choose a valid future expiry, or leave it empty for valid until revoked.');
  return seconds;
}

export class SecurityAttempt {
  #sent = false;
  constructor(session, action, body) {
    if (!['team/appoint', 'team/revoke', 'keys/issue', 'keys/revoke'].includes(action)) throw new Error('Unknown security action.');
    if (!canManageTeam(session, {id: body.workspace_id})) throw new Error('Use this workspace’s owner key.');
    this.kind = 'security'; this.workspace_id = body.workspace_id;
    this.path = '/api/v1/admin/' + action;
    this.requestBody = JSON.stringify(body);
    this.body = JSON.stringify({path: this.path, request: JSON.parse(this.requestBody)});
    this.session = signature(session); this.proof = session.csrf_token;
    Object.freeze(this);
  }
  request(session) {
    if (this.session !== signature(session)) throw new Error('The session changed. Do not send this security action under the new session.');
    if (this.#sent) throw new Error('Do not retry this security action. Inspect current metadata first.');
    this.#sent = true;
    return {body: this.requestBody, proof: this.proof};
  }
}

export function canTransferOwnership(session, workspace, admin) {
  return canManageTeam(session, workspace) && !workspace.archived && admin?.enabled === true
    && admin.is_owner === false && typeof admin.id === 'string' && !!admin.id
    && admin.id !== session.identity.principal_id;
}

export class TransferAttempt {
  #sent = false;
  constructor(session, workspace, admin, delivered, confirmation) {
    if (!canTransferOwnership(session, workspace, admin) || delivered !== true || confirmation !== admin.id) {
      throw new Error('Select another enabled workspace administrator, confirm key delivery/access and enter that principal’s exact ID.');
    }
    this.kind = 'transfer'; this.workspace_id = workspace.id;
    this.path = '/api/v1/admin/workspaces/transfer-owner';
    this.requestBody = JSON.stringify({workspace_id: workspace.id, new_owner_id: admin.id});
    this.body = JSON.stringify({path: this.path, request: JSON.parse(this.requestBody), former_owner_id: session.identity.principal_id});
    this.session = signature(session); this.proof = session.csrf_token;
    Object.freeze(this);
  }
  request(session) {
    if (this.session !== signature(session)) throw new Error('The session changed. Do not send this ownership transfer under the new session.');
    if (this.#sent) throw new Error('Do not retry this ownership transfer. Ask an authorized key holder to inspect current ownership.');
    this.#sent = true;
    return {body: this.requestBody, proof: this.proof};
  }
}

export function canManageInvitations(session, workspace) {
  const scope = session.identity.scope;
  return !!workspace && scope.kind === 'workspace' && scope.workspace_id === workspace.id && ['owner', 'admin'].includes(scope.role);
}

export function invitationDeadline(value, now = Date.now()) {
  const hours = Number(value), deadline = Math.floor(now / 1000) + hours * 3600;
  if (!Number.isInteger(hours) || hours < 1 || hours > 168 || !Number.isSafeInteger(deadline)) throw new Error('Choose 1–168 whole hours.');
  return deadline;
}

export function invitationStatus(value, id = value?.invitation_id) {
  if (typeof id !== 'string' || !id || value?.invitation_id !== id
    || !Number.isSafeInteger(value.uses) || value.uses < 0
    || value.max_uses !== null && (!Number.isSafeInteger(value.max_uses) || value.max_uses < 1 || value.uses > value.max_uses)
    || value.expires_at !== null && (!Number.isSafeInteger(value.expires_at) || value.expires_at < 0)
    || !['revoked', 'expired', 'initial_closed', 'issuer_authorized'].every(key => typeof value[key] === 'boolean')
    || value.target_client_id !== null && (typeof value.target_client_id !== 'string' || !value.target_client_id)) {
    throw new Error('Invitation metadata could not be verified within the browser’s exact numeric range. No status was assumed.');
  }
  return value;
}

export function invitationPage(value) {
  if (!Array.isArray(value?.invitations) || value.invitations.length > 16
    || value.next_cursor !== null && typeof value.next_cursor !== 'string') throw new Error('The invitation page could not be verified.');
  const ids = new Set();
  for (const item of value.invitations) {
    if (!['initial_group', 'individual'].includes(item?.kind) || !['read', 'read_write'].includes(item.role)) throw new Error('The invitation kind or role could not be verified.');
    const status = invitationStatus(item.status);
    if (ids.has(status.invitation_id)) throw new Error('Duplicate invitation metadata. Refresh before continuing.');
    ids.add(status.invitation_id);
  }
  if (value.next_cursor !== null && (!value.invitations.length || value.next_cursor !== value.invitations.at(-1).status.invitation_id)) throw new Error('The invitation cursor could not be verified.');
  return value;
}

export class AdmissionAttempt {
  #sent = false;
  constructor(session, workspace, action, body) {
    if (!canManageInvitations(session, workspace) || body.workspace_id !== workspace.id
      || !['individual', 'revoke'].includes(action) || action === 'individual' && (workspace.archived
        || !['read', 'read_write'].includes(body.role) || !Number.isSafeInteger(body.expires_at) || body.expires_at <= 0)
      || action === 'revoke' && (typeof body.invitation_id !== 'string' || !body.invitation_id)) throw new Error('Use the matching workspace key and an explicit valid invitation action.');
    this.kind = 'admission'; this.workspace_id = workspace.id;
    this.path = '/api/v1/admin/invitations/' + action;
    this.requestBody = JSON.stringify(action === 'individual'
      ? {workspace_id: workspace.id, role: body.role, expires_at: body.expires_at}
      : {workspace_id: workspace.id, invitation_id: body.invitation_id});
    this.body = JSON.stringify({path: this.path, request: JSON.parse(this.requestBody)});
    this.session = signature(session); this.proof = session.csrf_token;
    Object.freeze(this);
  }
  request(session) {
    if (this.session !== signature(session)) throw new Error('The session changed. Do not send this invitation action under the new session.');
    if (this.#sent) throw new Error('Do not retry this invitation action. Inspect current metadata first.');
    this.#sent = true;
    return {body: this.requestBody, proof: this.proof};
  }
}

export function enrollmentStatus(value, workspaceId) {
  if (value?.workspace_id !== workspaceId || !['unused', 'open', 'closed'].includes(value.phase)
    || !['has_admission_history', 'workspace_archived', 'can_open'].every(key => typeof value[key] === 'boolean')
    || value.closed_at !== null && (!Number.isSafeInteger(value.closed_at) || value.closed_at < 0)) throw new Error('Initial enrollment state could not be verified. No eligibility was assumed.');
  if (value.invitation !== null) invitationStatus(value.invitation);
  if (value.phase === 'unused' && (value.closed_at !== null || value.invitation !== null)
    || value.phase === 'open' && (value.closed_at !== null || !value.invitation || value.invitation.initial_closed || value.invitation.revoked || value.invitation.expired)
    || value.phase === 'closed' && (value.closed_at === null || value.invitation !== null && !value.invitation.initial_closed)
    || value.can_open && (value.phase !== 'unused' || value.has_admission_history || value.workspace_archived)) throw new Error('Initial enrollment metadata is inconsistent. Read current state before continuing.');
  return value;
}

export function initialBounds(capacity, hours, now = Date.now()) {
  const max = capacity.trim() === '' ? null : Number(capacity);
  const lifetime = hours.trim() === '' ? null : Number(hours);
  const expiry = lifetime === null ? null : Math.floor(now / 1000) + lifetime * 3600;
  if (max !== null && (!Number.isInteger(max) || max < 1 || max > 4294967295)
    || lifetime !== null && (!Number.isSafeInteger(lifetime) || lifetime < 1 || !Number.isSafeInteger(expiry) || expiry * 1000 > 8640000000000000)
    || max === null && expiry === null) throw new Error('Choose a positive client capacity (at most 4294967295), a positive whole lifetime in hours, or both within the browser’s date range.');
  return {max_clients: max, expires_at: expiry};
}

export class EnrollmentAttempt {
  #sent = false;
  constructor(session, state, action, body, confirmed) {
    enrollmentStatus(state, state.workspace_id);
    if (!canManageInvitations(session, {id: state.workspace_id}) || body.workspace_id !== state.workspace_id || confirmed !== true
      || !['initial', 'close'].includes(action) || action === 'close' && state.phase === 'closed'
      || action === 'initial' && (!state.can_open || !['read', 'read_write'].includes(body.role)
        || body.max_clients !== null && (!Number.isInteger(body.max_clients) || body.max_clients < 1 || body.max_clients > 4294967295)
        || body.expires_at !== null && (!Number.isSafeInteger(body.expires_at) || body.expires_at < 1)
        || body.max_clients === null && body.expires_at === null)) throw new Error('Use the current workspace key, explicit bounds and confirmation for this initial-window action.');
    this.kind = 'enrollment'; this.workspace_id = state.workspace_id;
    this.path = action === 'initial' ? '/api/v1/admin/invitations/initial' : '/api/v1/admin/enrollment/close';
    this.requestBody = JSON.stringify(action === 'initial' ? {workspace_id: state.workspace_id, role: body.role, max_clients: body.max_clients, expires_at: body.expires_at} : {workspace_id: state.workspace_id});
    this.body = JSON.stringify({path: this.path, request: JSON.parse(this.requestBody)});
    this.session = signature(session); this.proof = session.csrf_token; Object.freeze(this);
  }
  request(session) {
    if (this.session !== signature(session)) throw new Error('The session changed. Do not send this initial-window action under the new session.');
    if (this.#sent) throw new Error('Do not retry this initial-window action. Inspect current state first.');
    this.#sent = true; return {body: this.requestBody, proof: this.proof};
  }
}

export function clientRecoveryScope(session, workspace) {
  if (session.identity.scope.kind === 'instance') return {kind: 'instance'};
  if (canManageInvitations(session, workspace)) return {kind: 'workspace', workspace_id: workspace.id};
  throw new Error('Use an instance recovery key or this workspace’s owner/admin key.');
}
function recoveryScopeKey(scope) {
  if (scope?.kind === 'instance') return 'instance';
  if (scope?.kind === 'workspace' && typeof scope.workspace_id === 'string' && scope.workspace_id) return 'workspace:' + scope.workspace_id;
  throw new Error('The recovery scope could not be verified.');
}
export function recoveryStatus(value, clientId, scope) {
  if (value?.client_id !== clientId || recoveryScopeKey(value.scope) !== recoveryScopeKey(scope)
    || !Number.isSafeInteger(value.expires_at) || value.expires_at < 0
    || !['revoked', 'expired', 'redeemed', 'issuer_authorized'].every(key => typeof value[key] === 'boolean')) throw new Error('Recovery metadata did not match the client/scope or could not be verified.');
  return value;
}
export function recoveryPage(value, clientId, scope) {
  if (!Array.isArray(value?.invitations) || value.invitations.length > 16 || value.next_cursor !== null && typeof value.next_cursor !== 'string') throw new Error('The recovery page could not be verified.');
  const ids = new Set();
  for (const item of value.invitations) {
    if (typeof item?.invitation_id !== 'string' || !item.invitation_id || ids.has(item.invitation_id)) throw new Error('Recovery invitation IDs could not be verified.');
    ids.add(item.invitation_id); recoveryStatus(item.status, clientId, scope);
  }
  if (value.next_cursor !== null && (!value.invitations.length || value.next_cursor !== value.invitations.at(-1).invitation_id)) throw new Error('The recovery cursor could not be verified.');
  return value;
}
export class ClientAccessAttempt {
  #sent = false;
  constructor(session, workspace, client, change) {
    const scope = clientRecoveryScope(session, workspace);
    if (typeof client.client_id !== 'string' || !client.client_id || typeof client.client_disabled !== 'boolean'
      || client.revoked_at !== null && (!Number.isSafeInteger(client.revoked_at) || client.revoked_at < 0)
      || !['recover', 'revoke_recovery', 'readmit'].includes(change.action)) throw new Error('Select a verified registered client and explicit access action.');
    if (change.action !== 'revoke_recovery' && (client.client_disabled || !Number.isSafeInteger(change.expires_at) || change.expires_at < 1)) throw new Error('Disabled clients cannot be recovered or readmitted; choose a valid invitation deadline.');
    if (change.action === 'recover' && scope.kind === 'workspace' && client.revoked_at !== null) throw new Error('Workspace recovery requires a current grant row. It never restores revoked access.');
    if (change.action === 'readmit' && (scope.kind !== 'workspace' || workspace.archived || client.revoked_at === null || !['read', 'read_write'].includes(change.role))) throw new Error('Readmission needs an active workspace, revoked history and an explicit scoped data role.');
    if (change.action === 'revoke_recovery' && (typeof change.invitation_id !== 'string' || !change.invitation_id)) throw new Error('Select the exact recovery invitation to revoke.');
    this.kind = 'client_access'; this.client_id = client.client_id; this.workspace_id = workspace.id;
    this.scope = Object.freeze(scope); this.action = change.action;
    this.path = change.action === 'readmit' ? '/api/v1/admin/invitations/readmission' : '/api/v1/admin/recovery/' + (change.action === 'recover' ? 'issue' : 'revoke');
    this.requestBody = JSON.stringify(change.action === 'recover' ? {client_id: client.client_id, scope, expires_at: change.expires_at}
      : change.action === 'readmit' ? {workspace_id: workspace.id, client_id: client.client_id, role: change.role, expires_at: change.expires_at} : {invitation_id: change.invitation_id});
    this.body = JSON.stringify({path: this.path, request: JSON.parse(this.requestBody), client_id: client.client_id, scope});
    this.session = signature(session); this.proof = session.csrf_token; Object.freeze(this);
  }
  request(session) {
    if (this.session !== signature(session)) throw new Error('The session changed. Do not send this client access action under the new session.');
    if (this.#sent) throw new Error('Do not retry this client access action. Inspect current metadata first.');
    this.#sent = true; return {body: this.requestBody, proof: this.proof};
  }
}
