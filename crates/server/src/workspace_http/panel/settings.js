// SPDX-License-Identifier: AGPL-3.0-or-later
import {LifecycleAttempt, RetentionIntent, canChangeLifecycle, retentionSeconds, retentionPolicy, workspaceSnapshot, wire} from './model.js';
import {showTeam} from './team.js';
import {canManageTeam} from './model.js';
import {byId, el, button, field, select, actions, notice, stamp, copy} from './view.js';

const GET = '/api/v1/admin/content/retention/get';
const CHANGE = '/api/v1/admin/content/retention/change';

export async function showSettings(api) {
  const root = byId('content'); root.replaceChildren(el('h2', 'Workspace settings'));
  lifecycle(api, root);
  const team = el('section'); root.append(team);
  team.append(el('h3', 'Administrators & keys'), el('p', 'Only this workspace’s owner can inspect its administrator roster, issue scoped keys or revoke its administrators and keys.'));
  if (canManageTeam(api.state.session, api.state.workspace)) actions(team, button('Manage administrators & keys', () => api.run(async () => {
    if (api.mayLeave()) await showTeam(api);
  })));
  const section = el('section', undefined, 'retention-settings');
  section.append(el('h3', 'Archived entry retention'), el('p', 'Loading current policy…', 'muted')); root.append(section);
  try {
    const policy = retentionPolicy(await api.request(GET, {workspace_id: api.state.workspace.id}), api.state.workspace.id);
    retention(api, section, policy);
    notice('Loaded workspace settings.');
    return policy;
  } catch (error) {
    if (!api.state.locked) {
      section.replaceChildren(el('h3', 'Retention policy unavailable'), el('p', error.message));
      section.append(el('p', 'No manual or timed policy was assumed. Lifecycle actions remain subject to their own permissions.'));
      actions(section, button('Read current settings', () => api.run(() => reload(api))));
      if (api.state.dirty && api.state.draft) {
        const work = el('section', undefined, 'local-settings-draft');
        field(work, 'Local configuration draft', {value: api.state.draft, area: true, readOnly: true});
        actions(work, button('Copy configuration draft', () => copy(api.state.draft)), button('Discard local configuration draft', () => {
          if (!confirm('Discard this local draft? Any forgotten server request remains uncancelled.')) return;
          api.clean(); work.remove(); notice('Local configuration draft discarded. Current policy is still unavailable.');
        }));
        section.append(work);
      }
      notice('Retention policy unavailable. ' + error.message);
    }
    return null;
  }
}

function lifecycle(api, root) {
  const workspace = api.state.workspace;
  const section = el('section', undefined, 'lifecycle-settings'); root.append(section);
  section.append(el('h3', 'Workspace lifecycle'), el('p', workspace.archived
    ? 'Archived: context remains readable. Content writes and new admission stop; timed entry cleanup is paused.'
    : 'Active: authorized content writes and admission are available. Explicitly enabled timed cleanup runs.'));
  if (!canChangeLifecycle(api.state.session, workspace)) {
    section.append(el('p', 'Only this workspace’s owner or an authorized instance key can archive or reactivate it.', 'muted'));
    return;
  }
  actions(section, button(workspace.archived ? 'Reactivate workspace' : 'Archive workspace', () => api.run(async () => {
    if (!api.mayLeave()) return;
    const message = workspace.archived
      ? `Reactivate ${workspace.name}? Content writes and admission resume, and timed cleanup continues from its remaining window.`
      : `Archive ${workspace.name}? Context stays readable; content writes and new admission stop, and timed cleanup pauses.`;
    if (!confirm(message)) return;
    await api.verify();
    const attempt = new LifecycleAttempt(api.state.session, workspace);
    api.state.pending = attempt;
    try {
      const request = attempt.request(api.state.session);
      const result = await wire(attempt.path, request.body, request.proof);
      if (result?.completed !== true) throw new Error('The lifecycle response could not be verified.');
      api.state.pending = null;
      freeze();
      api.workspaceAcknowledged(attempt.archived);
      const label = attempt.archived ? 'Archive' : 'Reactivation';
      section.replaceChildren(el('h3', 'Lifecycle action acknowledged'), el('p', `${label} action confirmed. Use Refresh to read the current workspace state before another action.`));
      notice(`${label} action confirmed.`);
      try {
        await api.reloadWorkspace();
        notice(`${label} action confirmed. Last read workspace state: ${api.state.workspace.archived ? 'Archived' : 'Active'}.`);
      } catch (error) { notice(`${label} action confirmed. Current state refresh failed: ${error.message}`); }
    } catch (error) { uncertain(api, attempt, error); }
  }), workspace.archived ? 'primary' : 'danger'));
}

function retention(api, root, policy) {
  root.replaceChildren(el('h3', 'Archived entry retention'));
  root.append(el('p', policy.retention_seconds === null ? 'Manual retention: keep individually archived entries until privileged manual purge.'
    : `Timed cleanup: individually archived entries become due after ${policy.retention_seconds} seconds.${api.state.workspace.archived ? ' Paused while this workspace is archived.' : ''}`));
  root.append(el('p', `Loaded policy revision ${policy.revision}${policy.changed_at === null ? ' · Default manual policy' : ' · Changed ' + stamp(policy.changed_at)}`, 'metadata'));
  root.append(el('p', 'Timed cleanup permanently removes due archived entries and their revision history, retaining body-free operation and audit proofs. Backup retention is separate. The policy belongs to this workspace; revoking an admin key does not disable it.', 'muted'));
  if (!api.writable()) {
    root.append(el('p', api.state.workspace.archived ? 'Reactivate the workspace before changing retention.' : 'Use a workspace owner/admin key to change this policy.', 'muted'));
    return;
  }
  if (policy.retention_seconds !== null) {
    root.append(el('p', 'To choose another duration, first disable timed cleanup, then explicitly enable it again. Existing individual archives receive fresh full windows on re-enable.'));
    actions(root, button('Disable timed cleanup', () => api.run(async () => {
      if (!api.mayLeave() || !confirm('Disable timed cleanup? Archive clocks are removed and entries return to manual retention.')) return;
      await change(api, new RetentionIntent(api.state.session, policy, null, false));
    })));
  } else enableForm(api, root, policy);
}

function enableForm(api, root, policy) {
  const form = el('form'); root.append(form);
  form.append(el('p', 'Enabling starts a fresh full window for every existing individually archived entry. Future entry archives get that same window. Workspace archive pauses the remaining time. No duration is selected by default.'));
  const row = el('div', undefined, 'duration-fields'); form.append(row);
  const amount = el('div'), units = el('div'); row.append(amount, units);
  const duration = field(amount, 'Retention duration'); duration.type = 'number'; duration.min = '1'; duration.step = '1'; duration.required = true;
  const unit = select(units, 'Duration unit', [{id: '86400', name: 'Days'}, {id: '3600', name: 'Hours'}, {id: '1', name: 'Seconds'}], () => {}); unit.value = '86400';
  const label = el('label', undefined, 'confirm-row');
  const check = el('input'); check.type = 'checkbox'; check.required = true;
  label.append(check, el('span', 'I confirm permanent timed deletion of due archived entries and their revision history.')); form.append(label);
  form.addEventListener('input', () => api.draft(JSON.stringify({workspace_id: policy.workspace_id, base_revision: policy.revision,
    duration: duration.value, unit: unit.value, confirm_timed_purge: check.checked}, null, 2)));
  const submit = el('button', 'Enable timed cleanup', 'primary'); submit.type = 'submit';
  actions(form, submit, button('Discard configuration / reload', () => api.run(() => reload(api))));
  form.addEventListener('submit', event => {
    event.preventDefault();
    api.run(async () => {
      if (api.state.pending) throw new Error('Resolve or explicitly forget the original request first.');
      const seconds = retentionSeconds(duration.value, unit.value);
      await change(api, new RetentionIntent(api.state.session, policy, seconds, check.checked));
    });
  });
}

async function reload(api, lifecycleOnly = false) {
  if (api.state.pending) return notice('Resolve or explicitly forget the original request before reloading.');
  if (api.state.dirty && !confirm('Discard this local configuration draft and read current settings? Copy any text you want to keep first.')) return;
  const policy = await api.reloadWorkspace();
  if (policy || lifecycleOnly && !api.state.locked) {
    api.clean(); byId('content').querySelector('.local-settings-draft')?.remove();
    notice(policy ? 'Loaded current workspace and retention settings.' : 'Loaded current workspace state. Retention policy remains unavailable.');
  }
}

async function change(api, intent) {
  if (api.state.pending && api.state.pending !== intent) throw new Error('Resolve the original request first.');
  await api.verify();
  api.state.pending = intent;
  let saved;
  try {
    const request = intent.request(api.state.session);
    saved = intent.receipt(await wire(CHANGE, request.body, request.proof));
    api.state.pending = null; api.clean();
    byId('content').querySelector('.settings-pending')?.remove();
    freeze();
    const section = byId('content').querySelector('.retention-settings');
    if (section) section.replaceChildren(el('h3', 'Retention change acknowledged'), el('p', `Saved revision ${saved.revision}: ${saved.retention_seconds === null ? 'Manual retention' : saved.retention_seconds + ' seconds'}. This is a historical receipt; use Refresh for the current policy.`));
    notice(`Retention change confirmed at revision ${saved.revision}.`);
  } catch (error) { uncertain(api, intent, error); return; }
  try {
    const latest = await api.reloadWorkspace();
    notice(`Retention change confirmed at revision ${saved.revision}.${latest ? ' Showing current revision ' + latest.revision + '.' : ' Current policy is unavailable.'}`);
  } catch (error) { notice(`Retention change confirmed at revision ${saved.revision}. Refresh failed: ${error.message}`); }
}

function freeze() {
  for (const node of byId('content').querySelectorAll('input,select,button')) { node.disabled = true; delete node.dataset.busy; }
}

function uncertain(api, attempt, error) {
  if (api.state.locked) return;
  if (error.status === 401 || error.status === 403) { api.lock(error.message); return; }
  byId('content').querySelector('.settings-pending')?.remove();
  freeze();
  const box = el('section', undefined, 'settings-pending warning');
  box.append(el('h3', 'Settings request not confirmed'), el('p', attempt.kind === 'lifecycle'
    ? 'This endpoint has no operation ID or safe retry. Nothing will resend it automatically. Reading current state cannot prove which request changed it. The original request may still complete later.'
    : 'The original policy change, UUID, base revision and proof are retained. Nothing retries automatically. Compare the current policy before deliberately reconciling it.'));
  field(box, 'Original settings request', {value: attempt.body, area: true, readOnly: true});
  const compare = async () => {
    let text;
    if (attempt.kind === 'lifecycle') {
      const current = workspaceSnapshot(await api.request('/api/v1/admin/workspaces/inspect', {workspace_id: attempt.workspace_id}), attempt.workspace_id);
      api.workspaceObserved(current.archived);
      const summary = byId('content').querySelector('.lifecycle-settings > p');
      if (summary) summary.textContent = `Last read: ${current.archived ? 'Archived' : 'Active'}. Original request outcome remains unattributed.`;
      text = `Last read workspace state: ${current.archived ? 'Archived' : 'Active'}. This is a state observation, not proof of this request’s outcome.`;
    } else {
      const policy = retentionPolicy(await api.request(GET, {workspace_id: attempt.workspace_id}), attempt.workspace_id);
      text = `Last read policy revision ${policy.revision}: ${policy.retention_seconds === null ? 'Manual retention' : policy.retention_seconds + ' seconds'}. The original request and base are unchanged.`;
    }
    box.querySelector('.settings-comparison')?.remove(); box.append(el('p', text, 'settings-comparison'));
  };
  const controls = actions(box, button('Copy original settings request', () => copy(attempt.body)),
    button('Read current state', () => api.run(compare)),
    button('Forget local settings attempt', () => api.run(async () => {
      if (!confirm('Forget this local request? It may already have completed or still complete later. This does not cancel it. Keep a copy before continuing.')) return;
      await api.verify();
      api.state.pending = null; api.state.dirty = true; api.state.draft = attempt.body;
      box.replaceChildren(el('h3', 'Local request tracking forgotten'), el('p', 'The original may still execute. Copy it if needed, then explicitly discard/reload current settings before another action.'));
      field(box, 'Forgotten settings request', {value: attempt.body, area: true, readOnly: true});
      actions(box, button('Copy original settings request', () => copy(attempt.body)), button('Discard configuration / reload', () => api.run(() => reload(api, attempt.kind === 'lifecycle'))));
    })));
  if (attempt.kind === 'retention') controls.prepend(button('Check / retry exact policy change', () => api.run(() => change(api, attempt))));
  byId('content').prepend(box); notice(error.message + ' The original settings request is retained.');
}
