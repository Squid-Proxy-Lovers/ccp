// SPDX-License-Identifier: AGPL-3.0-or-later
import {AdmissionAttempt, canManageInvitations, invitationDeadline, invitationPage, invitationStatus, wire} from './model.js';
import {byId, el, button, field, select, actions, notice, stamp, copy, secret} from './view.js';
const BASE = '/api/v1/admin/invitations/';

export async function showInvitations(api, parent, after = null, initialOpen = false) {
  let root = parent.querySelector('.invitations');
  if (!root) { root = el('section', undefined, 'invitations'); parent.append(root); }
  if (!canManageInvitations(api.state.session, api.state.workspace)) {
    root.replaceChildren(el('h3', 'Invitations'), el('p', 'Use this workspace’s owner/admin key to inspect or manage invitations. Instance read access does not include this security inventory.')); return;
  }
  try {
    const page = invitationPage(await api.request(BASE + 'list', {workspace_id: api.state.workspace.id, after, limit: 16}));
    root.replaceChildren(el('h3', 'Invitations'));
    root.append(el('p', 'Recorded invitation metadata, not a permission lease or proof that a particular request created a row. Secrets are shown only when created. Empty inventory does not mean initial group enrollment is available.', 'muted'));
    const table = el('table'), head = el('thead'), row = el('tr'), body = el('tbody');
    for (const title of ['Invitation / requested access', 'Recorded state', 'Actions']) row.append(el('th', title));
    head.append(row); table.append(head, body);
    for (const item of page.invitations) {
      const status = item.status, cell = el('td', item.kind === 'initial_group' ? 'Initial group' : status.target_client_id ? 'Readmission' : 'Individual');
      cell.append(el('small', status.invitation_id, 'identifier'), el('small', item.role === 'read_write' ? 'Read / write requested' : 'Read only requested'));
      if (status.target_client_id) cell.append(el('small', 'Target client ' + status.target_client_id, 'identifier'));
      const state = el('td', `${status.uses} uses / ${status.max_uses === null ? 'No count limit' : status.max_uses}`);
      state.append(el('small', status.expires_at === null ? 'No deadline' : 'Deadline ' + stamp(status.expires_at)),
        el('small', `${status.revoked ? 'Revoked' : 'Not revoked'} · ${status.expired ? 'Expiry observed' : 'Expiry not observed'}`),
        el('small', status.issuer_authorized ? 'Issuer currently authorized' : 'Issuer currently not authorized'));
      if (item.kind === 'initial_group') state.append(el('small', status.initial_closed ? 'Initial window closed' : 'Initial window not observed closed'));
      const controls = el('td');
      if (!status.revoked) controls.append(button('Revoke invitation', () => api.run(async () => {
        if (!api.mayLeave() || !confirm(`Revoke invitation ${status.invitation_id} in ${api.state.workspace.name}? Existing client grants and content are preserved. Revoking an initial-group invitation also closes its initial window.`)) return;
        await change(api, 'revoke', {workspace_id: api.state.workspace.id, invitation_id: status.invitation_id});
      }), 'danger'));
      const entry = el('tr'); entry.append(cell, state, controls); body.append(entry);
    }
    const scroll = el('div', undefined, 'table-scroll'); scroll.append(table); root.append(scroll);
    if (!page.invitations.length) root.append(el('p', 'No invitations on this page.', 'empty'));
    const read = cursor => api.run(async () => { if (api.mayLeave()) await showInvitations(api, parent, cursor, initialOpen); });
    actions(root, button('Read current invitations', () => read(null)));
    if (page.next_cursor) actions(root, button('Next invitations', () => read(page.next_cursor)));
    if (after) actions(root, button('First invitations', () => read(null)));
    root.append(el('p', 'Issuer authority is a current check: it is false while archived or after issuer permission/key loss. It is separate from permanent invitation revocation. Pages are separate current observations, ordered by UUID.', 'muted'));
  } catch (error) {
    if (api.state.locked) return;
    const failure = el('section', undefined, 'invitation-read-error');
    failure.append(el('h3', 'Invitation metadata unavailable'), el('p', error.message), el('p', 'No empty inventory or usable invitation was assumed. Any earlier table is a previous observation.'));
    actions(failure, button('Read invitation metadata', () => api.run(async () => { if (api.mayLeave()) await showInvitations(api, parent, after, initialOpen); })));
    root.querySelector('.invitation-read-error')?.remove(); root.append(failure);
  }
  if (!root.querySelector('.invitation-form')) {
    if (api.writable() && !initialOpen) issueForm(api, root);
    else root.append(el('p', initialOpen ? 'Close initial group enrollment before issuing individual invitations.' : 'New client admission is paused while archived.', 'muted'));
  }
}

function issueForm(api, root) {
  const form = el('form', undefined, 'invitation-form'); root.append(form);
  form.append(el('h3', 'Invite a client'), el('p', 'One invitation for this workspace. An existing registered client reuses its identity when redeeming it. This does not select the workspace for that client.'));
  const role = select(form, 'Workspace data access', [{id: 'read', name: 'Read only'}, {id: 'read_write', name: 'Read / write'}], () => {}); role.value = 'read';
  const hours = field(form, 'Invitation lifetime (hours)', {value: '24'}); hours.type = 'number'; hours.min = '1'; hours.max = '168'; hours.step = '1'; hours.required = true;
  const mark = () => api.draft(JSON.stringify({workspace_id: api.state.workspace.id, role: role.value, lifetime_hours: hours.value}, null, 2));
  form.addEventListener('input', mark); form.addEventListener('change', mark);
  const submit = el('button', 'Create invitation', 'primary'); submit.type = 'submit';
  actions(form, submit, button('Discard invitation draft', () => {
    if (!confirm('Discard this local invitation draft?')) return;
    role.value = 'read'; hours.value = '24'; api.clean(); notice('Local invitation draft discarded.');
  }));
  form.addEventListener('submit', event => { event.preventDefault(); api.run(async () => {
    if (api.state.pending) throw new Error('Resolve the original invitation action first.');
    await change(api, 'individual', {workspace_id: api.state.workspace.id, role: role.value, expires_at: invitationDeadline(hours.value)});
  }); });
}

function freeze() {
  for (const node of byId('content').querySelectorAll('button,input,textarea,select')) { node.disabled = true; delete node.dataset.busy; }
}
async function change(api, action, body) {
  const attempt = new AdmissionAttempt(api.state.session, api.state.workspace, action, body);
  await api.verify(); api.state.pending = attempt;
  let result;
  try {
    const request = attempt.request(api.state.session); result = await wire(attempt.path, request.body, request.proof);
    if (action === 'individual' ? typeof result?.invitation_id !== 'string' || !result.invitation_id || typeof result.secret !== 'string' || !result.secret : result?.completed !== true) throw new Error('The invitation response could not be verified.');
  } catch (error) { uncertain(api, attempt, error); return; }
  api.state.pending = null; api.clean(); freeze();
  const confirmed = action === 'individual' ? `Invitation creation confirmed: ${result.invitation_id} in workspace ${body.workspace_id}.` : `Invitation revocation confirmed: ${body.invitation_id} in workspace ${body.workspace_id}.`;
  try { await api.verify(); }
  catch { api.lock(confirmed + (action === 'individual'
    ? ' Its secret is unavailable because this session could not be checked. Sign in with an authorized workspace key to inspect/revoke this invitation before deliberately issuing another.'
    : ' Sign in with an authorized workspace key to inspect current status; revocation is already confirmed.')); return; }
  const root = byId('content'); root.replaceChildren(el('h2', 'Invitation action acknowledged'), el('p', confirmed));
  if (action === 'individual') secret(root, 'Client invitation · ' + result.invitation_id, result.secret);
  actions(root, button('Read current client access and invitations', () => api.run(() => api.reloadWorkspace())));
  notice(confirmed + (action === 'individual' ? ' Copy the secret privately before leaving.' : ' Existing client grants and content are preserved.'));
}

function uncertain(api, attempt, error) {
  freeze(); const box = el('section', undefined, 'warning admission-pending');
  box.append(el('h3', 'Invitation action outcome unconfirmed'), el('p', error.message), el('p', 'This endpoint has no operation ID or safe retry. The action may have completed. Read metadata separately; a newly observed row does not prove which request created it. Secrets cannot be retrieved. Forgetting locally does not cancel delayed execution.'));
  field(box, 'Original invitation action', {value: attempt.body, area: true, readOnly: true});
  actions(box, button('Copy original invitation action', () => copy(attempt.body)), button('Read current invitation observation', () => api.run(async () => {
    const original = JSON.parse(attempt.requestBody);
    const observation = original.invitation_id ? invitationStatus(await api.request(BASE + 'status', original), original.invitation_id)
      : invitationPage(await api.request(BASE + 'list', {workspace_id: attempt.workspace_id, limit: 16}));
    box.querySelector('.invitation-observation')?.remove(); const section = el('section', undefined, 'invitation-observation');
    section.append(el('p', original.invitation_id ? 'Current status observation; no causal attribution.' : 'First page only; no causal attribution. After investigating and discarding locally, use the full inventory pager.'), el('pre', JSON.stringify(observation, null, 2))); box.append(section);
  })), button('Forget invitation action locally', () => {
    if (!confirm('Forget this invitation action locally? It is not cancelled and may still complete. Keep the original endpoint and target.')) return;
    api.state.pending = null; api.draft(attempt.body);
    box.replaceChildren(el('h3', 'Original invitation action retained as a local copy'), el('p', 'The server action remains uncancelled.'));
    field(box, 'Local invitation action copy', {value: attempt.body, area: true, readOnly: true});
    actions(box, button('Copy invitation action', () => copy(attempt.body)), button('Discard local invitation copy', () => {
      if (!confirm('Discard this local copy without cancelling the server request?')) return;
      api.clean(); const root = byId('content'); root.replaceChildren(el('h2', 'Local invitation copy discarded'), el('p', 'No server cancellation or current status read is implied.'));
      actions(root, button('Read current client access and invitations', () => api.run(() => api.reloadWorkspace())));
      notice('Local invitation copy discarded. Inspect current inventory before another action.');
    }));
  }));
  byId('content').prepend(box); notice('Invitation action unconfirmed. Original request retained; no retry sent.');
}
