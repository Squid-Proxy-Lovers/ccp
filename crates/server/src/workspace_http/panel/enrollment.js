// SPDX-License-Identifier: AGPL-3.0-or-later
import {canManageInvitations, enrollmentStatus, initialBounds, EnrollmentAttempt, wire} from './model.js';
import {byId, el, button, field, select, actions, notice, stamp, secret, copy} from './view.js';
const GET = '/api/v1/admin/enrollment/status';

export async function showEnrollment(api, parent) {
  if (!canManageInvitations(api.state.session, api.state.workspace)) return null;
  const root = el('section', undefined, 'enrollment'); parent.append(root);
  try {
    const state = enrollmentStatus(await api.request(GET, {workspace_id: api.state.workspace.id}), api.state.workspace.id);
    root.append(el('h3', 'Initial group enrollment'), el('p', `Last read window: ${state.phase}${state.closed_at === null ? '' : ' · Closed ' + stamp(state.closed_at)}.`));
    root.append(el('p', 'This workspace has one initial group phase. Closing it is terminal; existing client grants and individual invitations are preserved. Current observations are not an opening permission lease.', 'muted'));
    if (state.phase === 'unused') root.append(el('p', 'Issuing an individual invitation permanently closes this unused initial group phase, even before any client redeems it. Choose group enrollment first if you need this phase.', 'warning'));
    if (state.invitation) root.append(el('p', `Group invitation ${state.invitation.invitation_id} · ${state.invitation.uses} admissions${state.invitation.max_uses === null ? ' · No count limit' : ' / ' + state.invitation.max_uses}${state.invitation.expires_at === null ? ' · No deadline' : ' · Deadline ' + stamp(state.invitation.expires_at)}.`, 'metadata'));
    else if (state.phase === 'closed') root.append(el('p', 'The initial window was closed without a group invitation.'));
    if (state.phase === 'unused' && state.has_admission_history) root.append(el('p', 'Client admission history exists, including revoked grants. A new initial group phase cannot be opened.'));
    if (state.workspace_archived) root.append(el('p', 'Archived: new group issuance is paused. Authorized security closure remains available.'));
    if (state.can_open) actions(root, button('Configure initial group', () => api.run(() => configure(api, state))));
    if (state.phase !== 'closed') actions(root, button('Close initial enrollment permanently', () => api.run(async () => {
      if (!api.mayLeave() || !confirm(`Permanently close initial enrollment in ${api.state.workspace.name}? It cannot reopen. Existing client grants and individual invitations are preserved.`)) return;
      await change(api, new EnrollmentAttempt(api.state.session, state, 'close', {workspace_id: state.workspace_id}, true));
    }), 'danger'));
    return state;
  } catch (error) {
    if (api.state.locked) return null;
    root.replaceChildren(el('h3', 'Initial enrollment state unavailable'), el('p', error.message), el('p', 'No unused, open or closed state was assumed. Read current workspace again before an initial-group action.'));
    return null;
  }
}

function configure(api, state) {
  if (!api.mayLeave()) return;
  const root = byId('content'); root.replaceChildren(el('h2', 'Configure initial group enrollment'));
  root.append(el('p', `Workspace ${api.state.workspace.name} · ${state.workspace_id}`), el('p', 'Choose a positive admission capacity, a positive lifetime, or both. There is no numeric default. This phase cannot reopen after closure. Copy the one-time group secret privately; a lost secret cannot be retrieved or replaced by reopening the group.', 'warning'));
  const form = el('form'); root.append(form);
  const role = select(form, 'Group data access', [{id: 'read', name: 'Read only'}, {id: 'read_write', name: 'Read / write'}], () => {}); role.value = 'read';
  const capacity = field(form, 'Maximum client admissions (optional)'); capacity.type = 'number'; capacity.min = '1'; capacity.max = '4294967295'; capacity.step = '1';
  const hours = field(form, 'Group lifetime in hours (optional)'); hours.type = 'number'; hours.min = '1'; hours.step = '1';
  const label = el('label', undefined, 'confirm-row'), consent = el('input'); consent.type = 'checkbox'; consent.required = true;
  label.append(consent, el('span', 'I understand this is the workspace’s single initial group window and it cannot be reopened once closed.')); form.append(label);
  const mark = () => api.draft(JSON.stringify({workspace_id: state.workspace_id, role: role.value, max_clients: capacity.value, lifetime_hours: hours.value, confirmed: consent.checked}, null, 2));
  form.addEventListener('input', mark); form.addEventListener('change', mark);
  const submit = el('button', 'Create initial group invitation', 'primary'); submit.type = 'submit';
  actions(form, submit, button('Discard group draft', () => {
    if (!confirm('Discard this local group draft?')) return;
    api.clean(); localDiscard(api, 'Local group draft discarded.');
  }));
  form.addEventListener('submit', event => { event.preventDefault(); api.run(async () => {
    if (api.state.pending) throw new Error('Resolve the original group action first.');
    const bounds = initialBounds(capacity.value, hours.value);
    const attempt = new EnrollmentAttempt(api.state.session, state, 'initial', {workspace_id: state.workspace_id, role: role.value, ...bounds}, consent.checked);
    if (!confirm(`Open the single initial group in ${api.state.workspace.name}? Capacity: ${bounds.max_clients === null ? 'No count limit' : bounds.max_clients}. Deadline: ${bounds.expires_at === null ? 'None' : stamp(bounds.expires_at)}. It cannot reopen once closed.`)) return;
    await change(api, attempt);
  }); });
  capacity.focus();
}

function freeze() {
  for (const node of byId('content').querySelectorAll('button,input,textarea,select')) { node.disabled = true; delete node.dataset.busy; }
}
async function change(api, attempt) {
  await api.verify(); api.state.pending = attempt;
  let result; const opening = attempt.path.endsWith('/initial');
  try {
    const request = attempt.request(api.state.session); result = await wire(attempt.path, request.body, request.proof);
    if (opening ? typeof result?.invitation_id !== 'string' || !result.invitation_id || typeof result.secret !== 'string' || !result.secret : result?.completed !== true) throw new Error('The initial-window response could not be verified.');
  } catch (error) { uncertain(api, attempt, error); return; }
  api.state.pending = null; api.clean(); freeze();
  const confirmed = opening ? `Initial group creation confirmed: ${result.invitation_id} in workspace ${attempt.workspace_id}.` : `Initial enrollment closure confirmed in workspace ${attempt.workspace_id}.`;
  try { await api.verify(); }
  catch { api.lock(confirmed + (opening ? ' Its secret is unavailable because the session could not be checked. Inspect/revoke this group invitation or close the window, then use individual invitations. The group cannot be reopened.' : ' Sign in with a valid workspace key to inspect current state.')); return; }
  const root = byId('content'); root.replaceChildren(el('h2', 'Initial-window action acknowledged'), el('p', confirmed));
  if (opening) secret(root, 'Initial group invitation · ' + result.invitation_id, result.secret);
  actions(root, button('Read current client access and invitations', () => api.run(() => api.reloadWorkspace())));
  notice(confirmed + (opening ? ' Copy the secret privately before leaving.' : ' Existing client grants and individual invitations are preserved.'));
}
function localDiscard(api, message) {
  const root = byId('content'); root.replaceChildren(el('h2', 'Local group work discarded'), el('p', 'This does not cancel a server action or imply a current state read.'));
  actions(root, button('Read current client access and invitations', () => api.run(() => api.reloadWorkspace()))); notice(message);
}
function uncertain(api, attempt, error) {
  freeze(); const box = el('section', undefined, 'warning enrollment-pending');
  box.append(el('h3', 'Initial-window action outcome unconfirmed'), el('p', error.message), el('p', 'No operation ID or replay receipt is available. These controls do not retry; the action may have completed. A window observation is not causal attribution. If the group secret is lost, inspect/revoke its invitation or close the window and use individual invitations; do not attempt to reopen it.'));
  field(box, 'Original initial-window action', {value: attempt.body, area: true, readOnly: true});
  actions(box, button('Copy original group action', () => copy(attempt.body)), button('Read current initial-window observation', () => api.run(async () => {
    const value = enrollmentStatus(await api.request(GET, {workspace_id: attempt.workspace_id}), attempt.workspace_id);
    box.querySelector('.enrollment-observation')?.remove(); const section = el('section', undefined, 'enrollment-observation');
    section.append(el('p', 'Current observation only; not causal attribution or cancellation.'), el('pre', JSON.stringify(value, null, 2))); box.append(section);
  })), button('Forget group action locally', () => {
    if (!confirm('Forget this group action locally? It is not cancelled and may still complete. Keep the original request.')) return;
    api.state.pending = null; api.draft(attempt.body);
    box.replaceChildren(el('h3', 'Original group action retained as a local copy'), el('p', 'The server action remains uncancelled.'));
    field(box, 'Local group action copy', {value: attempt.body, area: true, readOnly: true});
    actions(box, button('Copy group action', () => copy(attempt.body)), button('Discard local group copy', () => {
      if (!confirm('Discard the local copy without cancelling the server action?')) return;
      api.clean(); localDiscard(api, 'Local group copy discarded. Read current window state before further action.');
    }));
  }));
  byId('content').prepend(box); notice('Initial-window action unconfirmed. No retry was sent; original request retained.');
}
