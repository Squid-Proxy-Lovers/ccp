// SPDX-License-Identifier: AGPL-3.0-or-later
import {clientRecoveryScope, recoveryPage, recoveryStatus, ClientAccessAttempt, canManageInvitations, invitationDeadline, invitationPage, enrollmentStatus, wire} from './model.js';
import {byId, el, button, field, select, actions, notice, stamp, secret, copy} from './view.js';
const BASE = '/api/v1/admin/recovery/';
const scopeLabel = (scope, workspace) => scope.kind === 'instance' ? 'Instance-wide · all still-authorized workspaces' : `This workspace only · ${workspace.name} (${workspace.id})`;

export async function showClientAccess(api, client, after = null) {
  if (!api.mayLeave()) return;
  const workspace = api.state.workspace, scope = clientRecoveryScope(api.state.session, workspace);
  const root = byId('content');
  let page;
  try { page = recoveryPage(await api.request(BASE + 'list', {client_id: client.client_id, scope, after, limit: 16}), client.client_id, scope); }
  catch (error) {
    if (api.state.locked) return;
    root.replaceChildren(el('h2', 'Recovery history unavailable'), el('p', error.message), el('p', 'No empty history or recoverable access was assumed.'));
    actions(root, button('Read client recovery history', () => api.run(() => showClientAccess(api, client))), button('Back to workspace access', () => api.run(() => api.reloadWorkspace()))); return;
  }
  root.replaceChildren(el('h2', 'Client recovery & readmission'), el('p', `${client.client_name} · ${client.client_id}`, 'identifier'), el('p', scopeLabel(scope, workspace)));
  root.append(el('p', `Last read grant row: ${client.revoked_at === null ? 'Current' : 'Revoked'}${client.client_disabled ? ' · Client disabled instance-wide' : ''}. This row and the history below are observations, not future authority.`, 'muted'));
  actions(root, button('Back to workspace access', () => api.run(async () => { if (api.mayLeave()) await api.reloadWorkspace(); })), button('Read current recovery history', () => api.run(() => showClientAccess(api, client))));
  root.append(el('h3', 'Recovery invitations'));
  const table = el('table'), head = el('thead'), row = el('tr'), body = el('tbody');
  for (const name of ['Invitation', 'Recorded state', 'Actions']) row.append(el('th', name)); head.append(row); table.append(head, body);
  for (const item of page.invitations) {
    const status = item.status, state = el('td', 'Deadline ' + stamp(status.expires_at));
    state.append(el('small', `${status.revoked ? 'Revoked' : 'Not revoked'} · ${status.expired ? 'Expiry observed' : 'Expiry not observed'}`), el('small', status.redeemed ? 'Redeemed' : 'Not redeemed'), el('small', status.issuer_authorized ? 'Issuer currently authorized' : 'Issuer currently not authorized'));
    const controls = el('td');
    if (!status.revoked) controls.append(button('Revoke recovery invitation', () => api.run(async () => {
      if (!api.mayLeave() || !confirm(`Revoke recovery invitation ${item.invitation_id} for client ${client.client_id}? This blocks redemption and receipt retrieval. It does NOT revoke an already-issued credential or membership.`)) return;
      await change(api, new ClientAccessAttempt(api.state.session, workspace, client, {action: 'revoke_recovery', invitation_id: item.invitation_id}));
    }), 'danger'));
    const entry = el('tr'); entry.append(el('td', item.invitation_id, 'identifier'), state, controls); body.append(entry);
  }
  const scroll = el('div', undefined, 'table-scroll'); scroll.append(table); root.append(scroll);
  if (!page.invitations.length) root.append(el('p', 'No recovery invitations on this page in this exact scope.', 'empty'));
  if (page.next_cursor) actions(root, button('Next recovery invitations', () => api.run(() => showClientAccess(api, client, page.next_cursor))));
  if (after) actions(root, button('First recovery invitations', () => api.run(() => showClientAccess(api, client))));
  root.append(el('p', 'Invitation expiry/revocation is separate from credentials already issued. These flags do not prove the client or its captured grants can still be recovered. Redemption rechecks current authority. Pages are separate observations, and secrets cannot be retrieved.', 'muted'));
  if (client.client_disabled) root.append(el('p', 'This client is disabled instance-wide. No fresh recovery or readmission is offered. Existing invitation history can still be revoked.'));
  else {
    if (scope.kind === 'instance' || client.revoked_at === null) actions(root, button(scope.kind === 'instance' ? 'Issue instance recovery invitation' : 'Issue workspace recovery invitation', () => api.run(() => edit(api, client, 'recover'))));
    else root.append(el('p', 'This grant row is revoked. Select a current grant row for workspace recovery; recovery cannot restore revoked access.'));
    if (canManageInvitations(api.state.session, workspace) && !workspace.archived && client.revoked_at !== null) actions(root, button('Explicitly readmit client', () => api.run(() => edit(api, client, 'readmit'))));
    else if (workspace.archived && scope.kind === 'workspace') root.append(el('p', 'Archived: recovery of still-authorized readable context remains available; new readmission requires reactivation.'));
  }
  root.append(el('section', undefined, 'client-access-editor')); notice('Loaded client recovery history in the explicit scope.');
}

async function edit(api, client, action) {
  if (!api.mayLeave()) return;
  if (action === 'readmit') {
    const window = enrollmentStatus(await api.request('/api/v1/admin/enrollment/status', {workspace_id: api.state.workspace.id}), api.state.workspace.id);
    if (window.phase === 'open') return notice('Close initial group enrollment before targeted readmission. Return to workspace access to manage that window.');
  }
  const root = byId('content').querySelector('.client-access-editor'); root.replaceChildren();
  const form = el('form'); root.append(form); const scope = clientRecoveryScope(api.state.session, api.state.workspace);
  form.append(el('h3', action === 'readmit' ? 'Explicit targeted readmission' : 'Recover existing client credentials'), el('p', `Target client ${client.client_name} · ${client.client_id}`), el('p', scopeLabel(scope, api.state.workspace)));
  form.append(el('p', action === 'readmit'
    ? 'Creates a NEW grant with the chosen role for this same registered client. Every prior grant in this workspace must be revoked; the server rechecks complete history. Any later admission makes this pending invitation stale, even if that later grant is revoked. It preserves old revocations and unrelated access, and does not select the workspace. Issuing this targeted readmission invitation permanently closes an unused initial group phase, even if the invitation is never redeemed or is later revoked. Use a fresh client key.'
    : 'Recovery preserves this client UUID and captures only still-authorized grants in this scope at issuance. Later grants are not included; grants revoked before redemption are excluded. It never restores revoked access or bypasses a disabled client. Issuance alone does not rotate credentials. On redemption, older credential bindings for captured grants are replaced; unrelated workspace bindings remain. Use a fresh key and a separate empty recovery profile.', 'warning'));
  if (action === 'recover') form.append(el('p', 'Issuing another recovery invitation does not revoke earlier ones. Deliver the secret privately; revoke undelivered invitations deliberately.'));
  let role;
  if (action === 'readmit') { role = select(form, 'Readmission data access', [{id: 'read', name: 'Read only'}, {id: 'read_write', name: 'Read / write'}], () => {}); role.value = 'read'; }
  const hours = field(form, 'Client access invitation lifetime (hours)', {value: '24'}); hours.type = 'number'; hours.min = '1'; hours.max = '168'; hours.step = '1'; hours.required = true;
  const mark = () => api.draft(JSON.stringify({client_id: client.client_id, scope, action, role: role?.value, lifetime_hours: hours.value}, null, 2)); form.addEventListener('input', mark); form.addEventListener('change', mark);
  const submit = el('button', action === 'readmit' ? 'Create targeted readmission invitation' : 'Create recovery invitation', 'primary'); submit.type = 'submit';
  actions(form, submit, button('Discard client access draft', () => { if (!confirm('Discard this local client access draft?')) return; api.clean(); root.replaceChildren(); notice('Local client access draft discarded.'); }));
  form.addEventListener('submit', event => { event.preventDefault(); api.run(async () => {
    if (api.state.pending) throw new Error('Resolve the original client access action first.');
    const expires_at = invitationDeadline(hours.value);
    const attempt = new ClientAccessAttempt(api.state.session, api.state.workspace, client, {action, expires_at, role: role?.value});
    if (!confirm(`${action === 'readmit' ? 'Issue explicit new membership' : 'Issue credential recovery'} for ${client.client_id}? Scope: ${scopeLabel(scope, api.state.workspace)}.${role ? ' New role: ' + role.value + '.' : ''} Deadline: ${stamp(expires_at)}.`)) return;
    await change(api, attempt);
  }); }); hours.focus();
}
function freeze() { for (const node of byId('content').querySelectorAll('button,input,textarea,select')) { node.disabled = true; delete node.dataset.busy; } }
async function change(api, attempt) {
  await api.verify(); api.state.pending = attempt; let result;
  try {
    const request = attempt.request(api.state.session); result = await wire(attempt.path, request.body, request.proof);
    if (attempt.action === 'revoke_recovery' ? result?.completed !== true : typeof result?.invitation_id !== 'string' || !result.invitation_id || typeof result.secret !== 'string' || !result.secret) throw new Error('The client access response could not be verified.');
  } catch (error) { uncertain(api, attempt, error); return; }
  api.state.pending = null; api.clean(); freeze();
  const id = result.invitation_id || JSON.parse(attempt.requestBody).invitation_id;
  const confirmed = `${attempt.action === 'readmit' ? 'Readmission invitation creation' : attempt.action === 'recover' ? 'Recovery invitation creation' : 'Recovery invitation revocation'} confirmed: ${id} for client ${attempt.client_id}. Scope: ${scopeLabel(attempt.scope, api.state.workspace)}.`;
  try { await api.verify(); }
  catch { api.lock(confirmed + (attempt.action === 'readmit' ? ' Its secret is unavailable; inspect/revoke this targeted invitation in workspace inventory before deliberately issuing another.' : attempt.action === 'recover' ? ' Its secret is unavailable; inspect this exact client/scope recovery history and revoke the undelivered invitation before deliberately issuing another.' : ' Current metadata can be read after signing in again; existing recovered access was not revoked.')); return; }
  const root = byId('content'); root.replaceChildren(el('h2', 'Client access action acknowledged'), el('p', confirmed));
  if (result.secret) secret(root, attempt.action === 'readmit' ? 'Targeted readmission invitation · ' + id : 'Recovery invitation · ' + id, result.secret);
  actions(root, button('Read current workspace access', () => api.run(() => api.reloadWorkspace()))); notice(confirmed + (result.secret ? ' Copy the secret privately before leaving.' : ' Existing credentials and grants remain unchanged.'));
}
function uncertain(api, attempt, error) {
  freeze(); const box = el('section', undefined, 'warning client-access-pending');
  box.append(el('h3', 'Client access action outcome unconfirmed'), el('p', error.message), el('p', 'No operation ID or replay receipt is available; these controls do not retry. The action may have completed. Current metadata cannot prove which request created a row. Keep the original client, scope and request; secrets cannot be retrieved.'));
  field(box, 'Original client access action', {value: attempt.body, area: true, readOnly: true});
  actions(box, button('Copy original client access action', () => copy(attempt.body)), button('Read current client access observation', () => api.run(async () => {
    const original = JSON.parse(attempt.requestBody); let value;
    if (attempt.action === 'readmit') value = invitationPage(await api.request('/api/v1/admin/invitations/list', {workspace_id: attempt.workspace_id, limit: 16}));
    else if (attempt.action === 'revoke_recovery') value = recoveryStatus(await api.request(BASE + 'status', original), attempt.client_id, attempt.scope);
    else value = recoveryPage(await api.request(BASE + 'list', {client_id: attempt.client_id, scope: attempt.scope, limit: 16}), attempt.client_id, attempt.scope);
    box.querySelector('.client-access-observation')?.remove(); const section = el('section', undefined, 'client-access-observation'); section.append(el('p', 'Observation only, not attribution or cancellation. Lists are first pages only; use the full pager after local discard.'), el('pre', JSON.stringify(value, null, 2))); box.append(section);
  })), button('Forget client access action locally', () => {
    if (!confirm('Forget this action locally? It is not cancelled and may still complete. Keep a copy of the original.')) return;
    api.state.pending = null; api.draft(attempt.body); box.replaceChildren(el('h3', 'Original client access action retained as a local copy'), el('p', 'The server action remains uncancelled.'));
    field(box, 'Local client access action copy', {value: attempt.body, area: true, readOnly: true});
    actions(box, button('Copy client access action', () => copy(attempt.body)), button('Discard local client access copy', () => {
      if (!confirm('Discard this local copy without cancelling the server action?')) return;
      api.clean(); const root = byId('content'); root.replaceChildren(el('h2', 'Local client access work discarded'), el('p', 'No server cancellation or successful metadata read is implied.')); actions(root, button('Read current workspace access', () => api.run(() => api.reloadWorkspace())));
    }));
  })); byId('content').prepend(box); notice('Client access action unconfirmed. Original request retained; no retry sent.');
}
