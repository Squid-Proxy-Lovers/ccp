// SPDX-License-Identifier: AGPL-3.0-or-later
import {canManageTeam, keyExpiry, SecurityAttempt, wire} from './model.js';
import {byId, el, button, field, select, actions, notice, stamp, copy, secret} from './view.js';
const BASE = '/api/v1/admin/';

export async function showTeam(api, adminAfter = null, keyAfter = null) {
  if (!canManageTeam(api.state.session, api.state.workspace)) throw new Error('Use this workspace’s owner key to manage administrators and keys.');
  const workspace = api.state.workspace;
  const roster = await api.request(BASE + 'team/list', {workspace_id: workspace.id, after: adminAfter, limit: 16});
  const keys = await api.request(BASE + 'keys/list', {workspace_id: workspace.id, after: keyAfter, limit: 16});
  const root = byId('content');
  root.replaceChildren(el('h2', 'Administrators & keys'));
  root.append(el('p', 'Owner-only security administration for this workspace. A principal ID identifies the registered administrator; names and shared keys do not prove which person is using them.'));
  actions(root, button('Back to settings', () => api.run(async () => { if (api.mayLeave()) await api.reloadWorkspace(); })),
    button('Read current administrators & keys', () => api.run(() => reload(api, adminAfter, keyAfter))));
  const table = grid(root, 'Administrators', ['Principal', 'Role', 'Actions']);
  for (const admin of roster.admins) {
    const cell = el('td', admin.name); cell.append(el('small', admin.id, 'identifier'));
    const controls = el('td');
    if (admin.enabled) controls.append(button('Issue key', () => api.run(() => edit(api, admin))));
    if (!admin.is_owner) controls.append(button('Revoke administrator', () => api.run(async () => {
      if (!api.mayLeave() || !confirm(`Revoke ${admin.name} (${admin.id}) in ${workspace.name}? All of this principal’s workspace keys are revoked; other workspaces and stored content are preserved.`)) return;
      await change(api, 'team/revoke', {workspace_id: workspace.id, principal_id: admin.id});
    }), 'danger'));
    const role = el('td', admin.is_owner ? 'Owner' : 'Administrator');
    if (!admin.enabled) role.append(el('small', 'Principal disabled'));
    const row = el('tr'); row.append(cell, role, controls); table.append(row);
  }
  if (!roster.admins.length) root.append(el('p', 'No administrators on this page.', 'empty'));
  pager(root, roster.next_cursor, adminAfter, 'administrators', next => api.run(async () => { if (api.mayLeave()) await showTeam(api, next, keyAfter); }));
  const keyTable = grid(root, 'Workspace keys', ['Key', 'Principal ID', 'Recorded state', 'Actions']);
  for (const key of keys.keys) {
    const name = el('td', key.name); name.append(el('small', key.id, 'identifier'));
    if (key.id === api.state.session.identity.key_id) name.append(el('small', 'Current login key'));
    const state = el('td', key.revoked_at === null ? 'Not revoked' : 'Revoked ' + stamp(key.revoked_at));
    state.append(el('small', key.expires_at === null ? 'No expiry' : 'Expires ' + stamp(key.expires_at)),
      el('small', key.role_active ? 'Principal’s scoped role active' : 'Principal’s scoped role inactive'));
    const controls = el('td');
    if (key.revoked_at === null) controls.append(button('Revoke key', () => api.run(async () => {
      if (!api.mayLeave() || !confirm(`Revoke key ${key.name} (${key.id}) in ${workspace.name}?${key.id === api.state.session.identity.key_id ? ' This is your current login key. This browser session will lose authority; have another owner key available.' : ''} The principal’s other keys and role remain unchanged.`)) return;
      await change(api, 'keys/revoke', {workspace_id: workspace.id, key_id: key.id});
    }), 'danger'));
    const row = el('tr'); row.append(name, el('td', key.principal_id, 'identifier'), state, controls); keyTable.append(row);
  }
  if (!keys.keys.length) root.append(el('p', 'No workspace keys on this page.', 'empty'));
  pager(root, keys.next_cursor, keyAfter, 'keys', next => api.run(async () => { if (api.mayLeave()) await showTeam(api, adminAfter, next); }));
  root.append(el('p', 'Pages are separate current observations, ordered by ID. Metadata contains no secret. Role activity does not mean an expired or revoked key works. Creating a key does not revoke older keys; deliver the new key privately before deliberately revoking an old one.', 'muted'));
  if (workspace.archived) root.append(el('p', 'Archived: key issuance/revocation and administrator revocation remain available. New administrator appointment requires reactivation.'));
  else actions(root, button('Appoint administrator', () => api.run(() => edit(api))));
  root.append(el('section', undefined, 'team-editor'));
  notice('Loaded administrator and key metadata.');
}

function grid(root, title, columns) {
  root.append(el('h3', title));
  const table = el('table'), row = el('tr'), head = el('thead'), body = el('tbody');
  for (const name of columns) row.append(el('th', name));
  head.append(row); table.append(head, body);
  const scroll = el('div', undefined, 'table-scroll'); scroll.append(table); root.append(scroll);
  return body;
}
function pager(root, next, after, title, choose) {
  if (next) actions(root, button('Next ' + title, () => choose(next)));
  if (after) actions(root, button('First ' + title, () => choose(null)));
}

function edit(api, admin = null) {
  if (!api.mayLeave()) return;
  const root = byId('content').querySelector('.team-editor'); root.replaceChildren();
  const form = el('form'); root.append(form);
  form.append(el('h3', admin ? 'Issue additional workspace key' : 'Appoint workspace administrator'));
  if (admin) form.append(el('p', `${admin.name} · ${admin.id}. Issuing does not revoke existing keys.`));
  let mode, principal, name;
  if (!admin) {
    mode = select(form, 'Administrator identity', [{id: 'new_principal', name: 'Create new principal'}, {id: 'existing', name: 'Existing principal ID'}], () => { principal.value = ''; mark(); });
    mode.value = 'new_principal';
    principal = field(form, 'Principal name or existing UUID'); principal.required = true;
    form.append(el('p', 'Create a new principal by label, or explicitly enter an existing principal UUID. Matching names never select an existing identity.', 'muted'));
  } else { name = field(form, 'New key label'); name.required = true; }
  const expires = field(form, 'Key expiry (optional, local time)'); expires.type = 'datetime-local'; expires.step = '1';
  form.append(el('p', 'Leave expiry empty for valid until revoked. The secret is returned once; deliver it privately. A lost response cannot retrieve the secret.'));
  const mark = () => api.draft(JSON.stringify({workspace_id: api.state.workspace.id, action: admin ? 'keys/issue' : 'team/appoint',
    principal_id: admin?.id, identity_kind: mode?.value, principal: principal?.value, name: name?.value, local_expiry: expires.value}, null, 2));
  form.addEventListener('input', mark); form.addEventListener('change', mark);
  const submit = el('button', admin ? 'Create workspace key' : 'Appoint and create key', 'primary'); submit.type = 'submit';
  actions(form, submit, button('Discard security draft', () => {
    if (!confirm('Discard this local security draft?')) return;
    api.clean(); root.replaceChildren(); notice('Local security draft discarded.');
  }));
  form.addEventListener('submit', event => {
    event.preventDefault(); api.run(async () => {
      if (api.state.pending) throw new Error('Resolve the original security action first.');
      const expiry = keyExpiry(expires.value);
      let body;
      if (admin) body = {workspace_id: api.state.workspace.id, principal_id: admin.id, name: name.value, expires_at: expiry};
      else {
        if (!['new_principal', 'existing'].includes(mode.value) || !principal.value.trim()) throw new Error('Choose an identity and enter its name or UUID.');
        if (mode.value === 'existing' && !/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(principal.value.trim())) throw new Error('Enter the existing principal’s complete UUID.');
        body = {workspace_id: api.state.workspace.id, principal: mode.value === 'existing'
          ? {kind: 'existing', principal_id: principal.value.trim()} : {kind: 'new_principal', name: principal.value.trim()}, key_expires_at: expiry};
      }
      if (!confirm(admin ? `Create an additional key for ${admin.name} (${admin.id})? Existing keys remain valid.` : 'Appoint this principal as a workspace administrator and create its first key? This permits client admission and data management in this workspace.')) return;
      await change(api, admin ? 'keys/issue' : 'team/appoint', body);
    });
  });
  (principal || name).focus();
}

async function reload(api, adminAfter = null, keyAfter = null) {
  if (api.state.pending) return notice('Read the current metadata in the pending action, then explicitly forget it before reloading.');
  if (api.state.dirty && !confirm('Discard local security work and read current administrator/key metadata? Forgotten server requests are not cancelled.')) return;
  await showTeam(api, adminAfter, keyAfter);
  api.clean();
}
function freeze() {
  for (const node of byId('content').querySelectorAll('button,input,textarea,select')) { node.disabled = true; delete node.dataset.busy; }
}

async function change(api, action, body) {
  await api.verify();
  const attempt = new SecurityAttempt(api.state.session, action, body);
  api.state.pending = attempt;
  let result;
  try {
    const request = attempt.request(api.state.session);
    result = await wire(attempt.path, request.body, request.proof);
    if (action === 'team/appoint' || action === 'keys/issue') {
      if (typeof result?.key_id !== 'string' || typeof result.principal_id !== 'string' || typeof result.secret !== 'string'
        || !result.secret || action === 'keys/issue' && result.principal_id !== body.principal_id
        || action === 'team/appoint' && body.principal.kind === 'existing' && result.principal_id !== body.principal.principal_id) throw new Error('The key creation response could not be verified.');
    } else if (result?.completed !== true) throw new Error('The revocation response could not be verified.');
  } catch (error) { uncertain(api, attempt, error); return; }
  api.state.pending = null; api.clean(); freeze();
  const confirmed = `Confirmed ${action} in workspace ${body.workspace_id}. ` + (result.key_id
    ? `Principal ${result.principal_id}; new key ${result.key_id}. ` : `Target ${body.key_id || body.principal_id}. `);
  if (body.key_id === api.state.session.identity.key_id) { api.lock(confirmed + 'Your current login key was revoked. Sign in using another owner key, or ask the host operator for local owner-key recovery.'); return; }
  try { await api.verify(); }
  catch { api.lock(confirmed + (result.key_id ? 'Its secret is unavailable because the session could not be checked. Sign in using a valid workspace owner key, inspect keys, issue a replacement for this principal and revoke the undelivered key. Do not repeat appointment by name.' : 'Read current metadata after signing in with a valid owner key.')); return; }
  const root = byId('content'); root.replaceChildren(el('h2', 'Security action acknowledged'), el('p', confirmed));
  if (result.key_id) secret(root, 'Workspace admin key · ' + result.key_id, result.secret);
  root.append(el('p', 'This acknowledges the action; current roles and key state may have changed since. No secret is stored by this panel.'));
  actions(root, button('Read current administrators & keys', () => api.run(() => reload(api))));
  notice(confirmed + (result.key_id ? 'Copy the secret privately before leaving.' : ''));
}

function uncertain(api, attempt, error) {
  freeze();
  const box = el('section', undefined, 'warning security-pending');
  box.append(el('h3', 'Security action outcome unconfirmed'), el('p', error.message), el('p', 'This endpoint has no operation ID or safe retry. The action may have completed. A roster/key observation cannot attribute changes to this request. No secret can be retrieved. Inspect metadata before deliberately issuing another key; do not repeat a new-principal appointment by name.'));
  field(box, 'Original security action', {value: attempt.body, area: true, readOnly: true});
  actions(box, button('Copy original security action', () => copy(attempt.body)), button('Read current security metadata', () => api.run(async () => {
    const admins = await api.request(BASE + 'team/list', {workspace_id: attempt.workspace_id, limit: 16});
    const keys = await api.request(BASE + 'keys/list', {workspace_id: attempt.workspace_id, limit: 16});
    const previous = box.querySelector('.security-observation'); previous?.remove();
    const observed = el('section', undefined, 'security-observation');
    observed.append(el('p', 'Observed first pages only; this does not confirm this request’s outcome. Use the full paginated view after forgetting locally.'), el('pre', JSON.stringify({admins, keys}, null, 2))); box.append(observed);
  })), button('Forget security action locally', () => api.run(async () => {
    if (!confirm('Forget this action locally? It is not cancelled and may still complete. Keep a copy of the original endpoint and target.')) return;
    api.state.pending = null; api.draft(attempt.body);
    box.replaceChildren(el('h3', 'Original action retained as a local copy'), el('p', 'The server action remains uncancelled.'));
    field(box, 'Local security action copy', {value: attempt.body, area: true, readOnly: true});
    actions(box, button('Copy security action', () => copy(attempt.body)), button('Discard local copy / read current team', () => api.run(() => reload(api))), button('Discard local security copy', () => {
      if (!confirm('Discard the local copy without reading the server? The original action remains uncancelled.')) return;
      api.clean();
      const root = byId('content'); root.replaceChildren(el('h2', 'Local security copy discarded'), el('p', 'The server action remains uncancelled. Read current metadata before another security action.'));
      actions(root, button('Read current administrators & keys', () => api.run(() => reload(api))));
      notice('Local security copy discarded. Current server state has not been read.');
    }));
  })));
  byId('content').prepend(box); notice('Security action outcome unconfirmed. Original endpoint and target retained; no retry was sent.');
}
