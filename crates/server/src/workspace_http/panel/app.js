// SPDX-License-Identifier: AGPL-3.0-or-later
import {signature, takeFragment, wire, canWrite} from './model.js';
import {byId, el, button, notice, stamp, field, actions, copy, secret} from './view.js';
import {showAccess} from './access.js';
import {showContext} from './context.js';
import {showGuidance} from './guidance.js';

let fragmentKey = takeFragment(location, history);
const state = {session: null, workspace: null, generation: 0, dirty: false, pending: null,
  draft: '', locked: false, busy: false, tab: 'access', cursor: null};

function mayLeave() {
  if (state.dirty || state.pending) {
    notice('Save or explicitly discard your draft and resolve the pending write before leaving.');
    return false;
  }
  return true;
}

function lock(message) {
  state.locked = true;
  state.generation += 1;
  byId('workspace-app').hidden = true;
  byId('content').replaceChildren();
  byId('workspaces').replaceChildren();
  byId('workspace-heading').replaceChildren();
  byId('identity').textContent = 'Session unavailable';
  byId('identity').removeAttribute('title');
  const panel = byId('locked');
  panel.hidden = false;
  panel.replaceChildren(el('h1', 'Session changed or unavailable'), el('p', message));
  if (state.draft || state.pending) {
    const text = state.pending ? state.pending.body : state.draft;
    field(panel, 'Unsaved local work / original write request', {value: text, area: true, readOnly: true});
    actions(panel, button('Copy local work', () => copy(text)));
    panel.append(el('p', 'A pending write may already have completed. Discarding here does not cancel it on the server. Keep its operation ID to investigate.'));
  }
  actions(panel, button('Discard local work and return to sign in', () => {
    if (!confirm('Discard local work? Any uncertain server write remains uncertain.')) return;
    reset();
  }));
  notice(message);
}

function reset() {
  byId('login-link').replaceChildren();
  Object.assign(state, {session: null, workspace: null, dirty: false, pending: null,
    draft: '', locked: false, cursor: null});
  state.generation += 1;
  byId('locked').hidden = true;
  byId('locked').replaceChildren();
  byId('workspace-app').hidden = true;
  byId('content').replaceChildren();
  byId('workspaces').replaceChildren();
  byId('workspace-heading').replaceChildren();
  notice('');
  byId('workspace-tabs').hidden = true;
  byId('identity').textContent = '';
  byId('identity').removeAttribute('title');
  byId('login').hidden = false;
  byId('key').focus();
}

async function verify() {
  if (!state.session || state.locked) throw new Error('Sign in again before continuing.');
  const generation = state.generation;
  let current;
  try { current = await wire('/api/v1/admin/session'); }
  catch (error) { if (generation === state.generation) lock(error.message); throw error; }
  if (generation !== state.generation) throw new Error('The view changed.');
  if (signature(current) !== signature(state.session)) {
    lock('Another tab changed this browser session. Existing edits will not be sent under that key.');
    throw new Error('The browser session changed.');
  }
  return current;
}

async function request(path, body) {
  await verify();
  const generation = state.generation;
  try {
    const data = await wire(path, body === undefined ? undefined : JSON.stringify(body), state.session.csrf_token);
    try { await verify(); }
    catch (error) {
      const recovery = creationRecovery(path, data, body);
      if (recovery) { lock(recovery); throw new Error(recovery); }
      throw error;
    }
    if (state.locked || generation !== state.generation) throw new Error('The view changed. Refresh before continuing.');
    return data;
  } catch (error) {
    if (error.status === 401 || error.status === 403) lock(error.message);
    throw error;
  }
}

function creationRecovery(path, result, target = {}) {
  if (path === '/api/v1/admin/workspaces/create') {
    return `Workspace creation confirmed: ${result.workspace_id}. The owner key is no longer displayed because this session could not be checked. Ask the CCP host operator to recover this workspace's owner key using the local reset-owner-key command. This requires host access; an instance key cannot reset it remotely. Do not create a duplicate workspace.`;
  }
  if (path === '/api/v1/admin/invitations/individual') {
    return `Invitation creation confirmed: ${result.invitation_id} in workspace ${target.workspace_id}. Its secret is no longer displayed because this session could not be checked. After signing in with an authorized workspace key, revoke this invitation through /api/v1/admin/invitations/revoke before deliberately issuing another.`;
  }
  return '';
}

async function run(action) {
  if (state.busy) return;
  state.busy = true;
  document.body.setAttribute('aria-busy', 'true');
  for (const node of document.querySelectorAll('button,input,textarea,select')) {
    if (!node.disabled) { node.dataset.busy = 'true'; node.disabled = true; }
  }
  try { await action(); }
  catch (error) { notice(error.message); }
  finally {
    state.busy = false;
    document.body.removeAttribute('aria-busy');
    for (const node of document.querySelectorAll('[data-busy]')) { node.disabled = false; delete node.dataset.busy; }
  }
}

const api = {
  state, request, verify, run, mayLeave, lock,
  writable: () => canWrite(state.session, state.workspace),
  draft(text) { state.dirty = true; state.draft = text; },
  clean() { state.dirty = false; state.draft = ''; },
};

async function workspaces(after = null) {
  const query = new URLSearchParams({limit: '32'});
  if (after) query.set('after', after);
  const data = await request(`/api/v1/admin/workspaces?${query}`);
  const nav = byId('workspaces');
  nav.replaceChildren();
  if (!data.workspaces.length) nav.append(el('p', 'No accessible workspaces.', 'empty'));
  for (const workspace of data.workspaces) {
    const item = button(workspace.name + (workspace.archived ? ' · Archived' : ''), () => run(async () => {
      if (!mayLeave()) return;
      state.generation += 1;
      state.workspace = workspace;
      state.tab = 'access';
      for (const node of nav.querySelectorAll('button')) node.removeAttribute('aria-current');
      item.setAttribute('aria-current', 'true');
      await renderWorkspace();
    }));
    nav.append(item);
  }
  if (data.next_cursor) nav.append(button('Next workspaces', () => run(async () => {
    if (mayLeave()) await workspaces(data.next_cursor);
  })));
  if (after) nav.append(button('First workspaces', () => run(async () => { if (mayLeave()) await workspaces(); })));
  if (state.session.identity.scope.kind === 'instance') createWorkspace(nav);
}

function createWorkspace(root) {
  const details = el('details'); details.append(el('summary', 'Create workspace'));
  const form = el('form');
  const name = field(form, 'Workspace name'); name.required = true;
  const owner = field(form, 'New owner principal name'); owner.required = true;
  form.append(el('p', 'Creates a new owner principal and delivers its workspace key once. Your instance key remains separate.', 'muted'));
  const submit = el('button', 'Create workspace'); submit.type = 'submit'; actions(form, submit);
  form.addEventListener('submit', event => {
    event.preventDefault();
    run(async () => {
      if (!mayLeave()) return;
      notice('Creating workspace. If the response is lost, refresh the list before repeating.');
      const result = await request('/api/v1/admin/workspaces/create', {
        name: name.value, owner: {kind: 'new_principal', name: owner.value}, key_expires_at: null,
      });
      secret(byId('workspace-heading'), 'Workspace owner key · ' + result.workspace_id, result.owner_key);
      notice('Workspace created. Copy its owner key privately before navigating away.');
      try { await workspaces(); }
      catch (error) {
        const message = state.locked ? creationRecovery('/api/v1/admin/workspaces/create', result)
          : `Workspace created. Copy its owner key privately before navigating away. Workspace list refresh failed: ${error.message}`;
        if (state.locked) lock(message);
        notice(message);
      }
    });
  });
  details.append(form); root.append(details);
}

async function renderWorkspace() {
  const info = await request('/api/v1/admin/workspaces/inspect', {workspace_id: state.workspace.id});
  state.workspace = info.workspace;
  byId('workspace-tabs').hidden = false;
  const heading = byId('workspace-heading');
  heading.replaceChildren(el('h2', info.workspace.name), el('p', info.workspace.archived
    ? 'Archived · Context is readable; content writes and new admission are paused.'
    : 'Active workspace'));
  heading.append(el('p', `Workspace ${info.workspace.id} · Owner ${info.owner_id || 'Not appointed'}`, 'metadata'));
  if (state.session.identity.scope.kind === 'instance') {
    heading.append(el('p', 'Instance key: workspace context is read-only. Sign in with a workspace key to edit.', 'muted'));
  }
  byId('global-guidance').setAttribute('aria-current', 'false');
  for (const tab of ['access', 'context', 'guidance']) byId(`${tab}-tab`).setAttribute('aria-current', String(state.tab === tab));
  byId('content').replaceChildren();
  if (state.tab === 'guidance') await showGuidance(api);
  else await (state.tab === 'access' ? showAccess(api) : showContext(api));
}

async function globalGuidance() {
  state.workspace = null;
  state.tab = 'global-guidance';
  byId('workspace-tabs').hidden = true;
  byId('workspace-heading').replaceChildren(el('h2', 'Global guidance'));
  for (const node of byId('workspaces').querySelectorAll('[aria-current]')) node.removeAttribute('aria-current');
  for (const tab of ['access', 'context', 'guidance']) byId(`${tab}-tab`).setAttribute('aria-current', 'false');
  byId('global-guidance').setAttribute('aria-current', 'true');
  await showGuidance(api);
}

async function activate(session) {
  byId('login-link').replaceChildren();
  state.generation += 1;
  state.session = session;
  state.locked = false;
  state.workspace = null;
  state.tab = 'access';
  byId('workspace-tabs').hidden = true;
  byId('global-guidance').setAttribute('aria-current', 'false');
  for (const tab of ['access', 'context', 'guidance']) byId(`${tab}-tab`).setAttribute('aria-current', 'false');
  byId('content').replaceChildren();
  byId('workspaces').replaceChildren();
  byId('login').hidden = true;
  byId('workspace-app').hidden = false;
  byId('locked').hidden = true;
  const scope = session.identity.scope;
  byId('identity').textContent = `${session.identity.principal_name} · ${session.identity.key_name} · ${scope.kind === 'instance' ? 'Instance admin' : 'Workspace ' + scope.role} · Session ends ${stamp(session.expires_at)}`;
  byId('identity').title = `Key ${session.identity.key_id} · Instance ${session.instance_id}. Key/principal labels do not identify the individual using a shared key.`;
  byId('workspace-heading').replaceChildren(el('p', 'Select a workspace to manage access or read shared context.'));
  await workspaces();
  notice('Signed in. Each action checks your current key permissions.');
}

byId('login-form').addEventListener('submit', event => {
  event.preventDefault();
  run(async () => {
    const key = byId('key').value;
    byId('key').value = '';
    byId('login-link').replaceChildren();
    await activate(await wire('/api/v1/admin/session/login', JSON.stringify({key}), 'login'));
  });
});
byId('key').addEventListener('input', () => byId('login-link').replaceChildren());
byId('copy-link').addEventListener('click', async () => {
  const key = byId('key').value;
  if (!key) return notice('Enter your key first, then copy the link before signing in.');
  const generation = state.generation;
  const link = `${location.origin}/admin#key=${encodeURIComponent(key)}`;
  if (!await copy(link) && generation === state.generation && !byId('login').hidden && byId('key').value === key) {
    const root = byId('login-link'); root.replaceChildren();
    const input = field(root, 'Reusable login link (keep private)', {value: link, readOnly: true});
    input.focus(); input.select();
  }
});
byId('refresh').addEventListener('click', () => run(async () => {
  if (!mayLeave()) return;
  if (state.tab === 'global-guidance') await globalGuidance();
  else if (state.workspace) await renderWorkspace();
  else await workspaces();
}));
for (const tab of ['access', 'context', 'guidance']) byId(`${tab}-tab`).addEventListener('click', () => run(async () => {
  if (!state.workspace || !mayLeave()) return;
  state.generation += 1;
  state.tab = tab;
  await renderWorkspace();
}));
byId('global-guidance').addEventListener('click', () => run(async () => {
  if (!mayLeave()) return;
  state.generation += 1;
  await globalGuidance();
}));
byId('signout').addEventListener('click', () => run(async () => {
  if (!mayLeave()) return;
  await verify();
  await wire('/api/v1/admin/session/logout', '{}', state.session.csrf_token);
  reset();
  notice('Signed out of this browser session. Your underlying key remains valid until revoked or expired.');
}));
window.addEventListener('beforeunload', event => {
  if (state.dirty || state.pending) { event.preventDefault(); event.returnValue = ''; }
});
window.addEventListener('focus', () => {
  if (state.session && !state.locked && !state.busy) run(verify);
});
window.addEventListener('hashchange', () => {
  if (!new URLSearchParams(location.hash.slice(1)).has('key')) return;
  const key = takeFragment(location, history);
  if (!key) return notice('Invalid login fragment. Enter your key manually.');
  if (state.busy || state.locked || !mayLeave()) {
    notice('The login link was removed from this URL. Finish or discard local work, then open your saved link again.');
    return;
  }
  run(async () => activate(await wire('/api/v1/admin/session/login', JSON.stringify({key}), 'login')));
});

run(async () => {
  if (fragmentKey) {
    const body = JSON.stringify({key: fragmentKey});
    fragmentKey = '';
    await activate(await wire('/api/v1/admin/session/login', body, 'login'));
  } else {
    try { await activate(await wire('/api/v1/admin/session')); }
    catch (error) { if (error.status !== 401) notice(error.message); }
  }
});
