// SPDX-License-Identifier: AGPL-3.0-or-later
import {byId, el, button, actions, notice} from './view.js';

import {showClientAccess} from './client_access.js';
import {showEnrollment} from './enrollment.js';
import {showInvitations} from './invitations.js';

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
    controls.append(button('Manage client access', () => api.run(() => showClientAccess(api, grant))));
    if (!grant.revoked_at && api.state.session.identity.scope.kind === 'workspace') {
      controls.append(button('Revoke workspace access', () => api.run(async () => {
        if (!api.mayLeave()) return;
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
  if (page.next_cursor) actions(root, button('Next clients', () => api.run(async () => { if (api.mayLeave()) await showAccess(api, page.next_cursor); })));
  if (after) actions(root, button('First clients', () => api.run(async () => { if (api.mayLeave()) await showAccess(api); })));
  const enrollment = await showEnrollment(api, root);
  await showInvitations(api, root, null, enrollment?.phase === 'open');
}
