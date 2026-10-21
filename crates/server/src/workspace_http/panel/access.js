// SPDX-License-Identifier: AGPL-3.0-or-later
import {byId, el, button, field, select, actions, notice, secret} from './view.js';

export async function showAccess(api, after = null) {
  const workspace = api.state.workspace;
  const page = await api.request('/api/v1/admin/workspaces/grants', {workspace_id: workspace.id, after, limit: 32});
  const root = byId('content');
  root.replaceChildren(el('h2', 'Clients & access'));
  root.append(el('p', 'Registered client identities and their grants in this workspace. Agent names and recent changes do not prove that a client is online.', 'muted'));
  const table = el('table');
  const head = el('tr');
  for (const name of ['Client', 'Workspace access', 'Actions']) head.append(el('th', name));
  const thead = el('thead'); thead.append(head); table.append(thead);
  const tbody = el('tbody'); table.append(tbody);
  for (const grant of page.grants) {
    const row = el('tr');
    const client = el('td', grant.client_name);
    client.append(el('small', grant.client_id, 'identifier'));
    const access = el('td', grant.revoked_at ? 'Revoked' : grant.role === 'read_write' ? 'Read / write' : 'Read only');
    if (grant.client_disabled) access.append(el('small', 'Disabled instance-wide'));
    const controls = el('td');
    if (!grant.revoked_at && api.state.session.identity.scope.kind === 'workspace') {
      controls.append(button('Revoke workspace access', () => api.run(async () => {
        if (!confirm(`Revoke ${grant.client_name} in ${workspace.name}? Stored entries and other workspaces are preserved.`)) return;
        await api.request('/api/v1/admin/grants/revoke', {grant_id: grant.id});
        await showAccess(api, after);
        notice('Workspace access revoked. Stored entries and ownership are preserved.');
      }), 'danger'));
    }
    if (!grant.client_disabled && api.state.session.identity.scope.kind === 'instance') {
      controls.append(button('Disable instance-wide', () => api.run(async () => {
        if (!confirm(`Disable ${grant.client_name} across this entire CCP instance, including certificate renewal? Stored content is preserved.`)) return;
        await api.request('/api/v1/admin/clients/disable', {client_id: grant.client_id});
        await showAccess(api, after);
        notice('Client disabled instance-wide. Stored content is preserved.');
      }), 'danger'));
    }
    row.append(client, access, controls); tbody.append(row);
  }
  const scroll = el('div', undefined, 'table-scroll'); scroll.append(table); root.append(scroll);
  if (!page.grants.length) root.append(el('p', 'No client grants on this page.', 'empty'));
  if (page.next_cursor) actions(root, button('Next clients', () => api.run(() => showAccess(api, page.next_cursor))));
  if (after) actions(root, button('First clients', () => api.run(() => showAccess(api))));
  if (api.writable()) invitation(root, api);
  else root.append(el('p', workspace.archived ? 'New client admission is paused while archived.' : 'Use a workspace admin key to invite clients.', 'muted'));
}

function invitation(root, api) {
  const form = el('form');
  form.append(el('h3', 'Invite a client'), el('p', 'One invitation for this workspace. An existing registered client reuses its identity when redeeming it. This does not select the workspace for that client.'));
  const role = select(form, 'Workspace data access', [
    {id: 'read', name: 'Read only'}, {id: 'read_write', name: 'Read / write'},
  ], () => {});
  role.value = 'read';
  const hours = field(form, 'Invitation lifetime (hours)', {value: '24'});
  hours.type = 'number'; hours.min = '1'; hours.max = '168'; hours.required = true;
  const submit = el('button', 'Create invitation', 'primary'); submit.type = 'submit';
  actions(form, submit);
  form.addEventListener('submit', event => {
    event.preventDefault();
    api.run(async () => {
      const lifetime = Number(hours.value);
      if (!Number.isInteger(lifetime) || lifetime < 1 || lifetime > 168) throw new Error('Choose 1–168 hours.');
      notice('Creating invitation. If the response is lost, do not assume creation failed.');
      const result = await api.request('/api/v1/admin/invitations/individual', {
        workspace_id: api.state.workspace.id, role: role.value,
        expires_at: Math.floor(Date.now() / 1000) + lifetime * 3600,
      });
      secret(root, 'Client invitation · ' + result.invitation_id, result.secret);
      notice('Invitation created. Copy it privately before leaving.');
    });
  });
  root.append(form);
}
