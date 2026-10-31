// SPDX-License-Identifier: AGPL-3.0-or-later
import {canChangeLifecycle, canDeleteWorkspace, deletionPreview, WorkspaceDeletionIntent,
  deletionLookup, deletionStatusRequest, deletionReceipt, deletionTracking, wire} from './model.js';
import {byId, el, button, field, actions, notice, stamp, copy} from './view.js';

const BASE = '/api/v1/admin/workspaces/deletion/';

export function showDeletionSettings(api, root) {
  const workspace = api.state.workspace;
  const section = el('section', undefined, 'deletion-settings'); root.append(section);
  section.append(el('h3', 'Permanent workspace deletion'), el('p', 'Remove this workspace’s entries, archives, revision history, guidance and status messages. This cannot be undone. Backup retention is separate.'));
  if (!canChangeLifecycle(api.state.session, workspace)) {
    section.append(el('p', 'Only this workspace’s owner or an instance administrator can permanently delete it.', 'muted')); return;
  }
  if (!canDeleteWorkspace(api.state.session, workspace)) {
    section.append(el('p', 'Archive this workspace first. Archiving preserves readable context and can be reversed.', 'muted')); return;
  }
  actions(section, button('Prepare permanent deletion', () => api.run(async () => {
    if (!api.mayLeave()) return;
    if (api.state.deletion) { renderDeletionTracker(api); return notice('Copy or explicitly forget the existing deletion tracking before preparing another deletion.'); }
    const preview = deletionPreview(await api.request(BASE + 'preview', {workspace_id: workspace.id}), workspace);
    freezeContent();
    confirmation(api, section, workspace, preview);
  }), 'danger'));
}

function confirmation(api, root, workspace, preview) {
  root.replaceChildren(el('h3', 'Confirm permanent workspace deletion'));
  root.append(el('p', `Delete ${preview.name} (${preview.workspace_id})? All access ends immediately when deletion starts.`));
  root.append(el('p', 'Shared client identities and access to other workspaces remain. Minimal deletion and security records remain. Large cleanups can take hours; “started” does not mean “complete”.'));
  root.append(el('p', 'Workspace keys and their browser sessions lose access at start. Save the receipt; a current instance administrator can check completion or investigate a lost response.', 'warning'));
  const form = el('form'); root.append(form);
  const typed = field(form, 'Type the exact workspace UUID'); typed.required = true; typed.autocomplete = 'off'; typed.spellcheck = false;
  const label = el('label', undefined, 'confirm-row');
  const check = el('input'); check.type = 'checkbox'; check.required = true;
  label.append(check, el('span', 'I confirm permanent deletion of this archived workspace and its contents and history.')); form.append(label);
  const draft = () => api.draft(JSON.stringify({unsubmitted: true, workspace_id: preview.workspace_id,
    lifecycle_revision: preview.lifecycle_revision, confirmation: typed.value, confirmed: check.checked}, null, 2));
  draft(); form.addEventListener('input', draft);
  const submit = el('button', 'Permanently delete workspace', 'danger'); submit.type = 'submit';
  actions(form, submit, button('Cancel deletion preparation', () => api.run(async () => {
    api.clean(); await api.reloadWorkspace();
  })));
  form.addEventListener('submit', event => {
    event.preventDefault(); api.run(async () => {
      if (api.state.pending || api.state.deletion) throw new Error('Resolve or explicitly forget the existing deletion tracking first.');
      await api.verify();
      const intent = new WorkspaceDeletionIntent(api.state.session, workspace, preview, typed.value, check.checked);
      await start(api, intent);
    });
  });
  typed.focus();
}

async function start(api, intent) {
  api.state.pending = intent;
  api.state.deletion = {reference: intent, receipt: null};
  try {
    const request = intent.request(api.state.session);
    const value = deletionReceipt(await wire(BASE + 'start', request.body, request.proof), intent);
    // Commit the body-free acknowledgment before any post-write session check.
    api.state.deletion.receipt = value;
    api.state.pending = null; api.clean();
  } catch (error) {
    clearWorkspace(api); renderDeletionTracker(api);
    if (error.status === 401 || error.status === 403) api.lock('Deletion outcome is unconfirmed. Keep the original tracking details; permission loss does not prove it failed.');
    notice('Deletion outcome unconfirmed. ' + error.message); return;
  }
  clearWorkspace(api);
  renderDeletionTracker(api);
  notice('Deletion started. Cleanup completion is tracked separately.');
  if (api.state.session.identity.scope.kind === 'workspace') {
    api.lock('Deletion started. This workspace key has lost its role. Save the receipt, then use an instance-admin key to check completion.');
  } else {
    try { await api.refreshWorkspaces(); }
    catch (error) { notice('Deletion started. The receipt is retained. Workspace refresh failed: ' + error.message); }
  }
}

export function renderDeletionTracker(api) {
  const root = byId('deletion-tracker'); root.replaceChildren();
  const tracked = api.state.deletion;
  root.hidden = !tracked;
  if (!tracked) return;
  const {reference, receipt} = tracked;
  root.append(el('h2', receipt ? receipt.completed_at === null ? 'Workspace deletion started' : 'Workspace cleanup complete' : 'Workspace deletion outcome unconfirmed'));
  root.append(el('p', receipt ? receipt.completed_at === null
    ? `Access ended at ${stamp(receipt.started_at)}. Cleanup is pending and can take hours for a large workspace. Check manually for completion.`
    : `Live workspace content and history were removed by ${stamp(receipt.completed_at)}. Minimal security/deletion records remain; backup retention is separate.`
    : 'The original request may already have committed or may still arrive. No request retries automatically. Missing or unavailable status does not prove failure.'));
  field(root, 'Deletion tracking details', {value: deletionTracking(reference, receipt), area: true, readOnly: true});
  root.append(el('p', 'Tracking stays in this page’s memory across sign-in changes. Copy it before reloading or closing the page. It contains no admin key or session proof.', 'muted'));
  const controls = actions(root, button('Copy deletion tracking', () => copy(deletionTracking(reference, receipt))));
  if (api.state.session && !api.state.locked && api.state.session.identity.scope.kind === 'instance'
    && api.state.session.instance_id === reference.instance_id) {
    controls.append(button('Check deletion completion', () => api.run(() => inspect(api, tracked))));
  } else {
    root.append(el('p', 'Sign in with a current instance-admin key on this instance to inspect this operation. This never resends the deletion.'));
    if (api.state.session && !api.state.locked) controls.append(button('Change key to inspect deletion', () => {
      api.lock('Keep the original deletion tracking, then sign in with an instance-admin key. The server request is not cancelled.');
    }));
  }
  controls.append(button('Forget local deletion tracking', () => {
    if (!confirm('Forget this local tracking? This does not cancel deletion or a delayed request. Copy the details first.')) return;
    if (api.state.pending === reference) { api.state.pending = null; api.clean(); }
    api.state.deletion = null; renderDeletionTracker(api);
    notice('Local deletion tracking forgotten. Any server deletion continues. Read current state before another action.');
  }));
}

async function inspect(api, tracked) {
  try {
    const request = deletionStatusRequest(api.state.session, tracked.reference);
    const value = await api.request(BASE + 'status', request);
    const receipt = deletionReceipt(value, tracked.reference, tracked.receipt);
    tracked.receipt = receipt;
    if (api.state.pending === tracked.reference) { api.state.pending = null; api.clean(); }
    renderDeletionTracker(api);
    notice(receipt.completed_at === null ? 'Deletion start confirmed; cleanup is still pending.' : 'Workspace cleanup completion confirmed.');
  } catch (error) {
    renderDeletionTracker(api);
    notice('Deletion status could not be verified. The original details and last confirmed receipt are retained. ' + error.message);
  }
}

export function lookupDeletion(api) {
  if (api.state.deletion) { renderDeletionTracker(api); return; }
  const root = byId('deletion-tracker'); root.hidden = false; root.replaceChildren(el('h2', 'Inspect a workspace deletion'));
  root.append(el('p', 'Use the workspace and operation UUIDs from the original deletion request. This only reads status; it does not start or repeat deletion.'));
  const form = el('form'); root.append(form);
  const workspace = field(form, 'Deleted workspace UUID'), operation = field(form, 'Deletion operation UUID');
  workspace.required = true; operation.required = true;
  const submit = el('button', 'Inspect deletion'); submit.type = 'submit'; actions(form, submit);
  form.addEventListener('submit', event => {
    event.preventDefault(); api.run(async () => {
      await api.verify();
      api.state.deletion = {reference: deletionLookup(api.state.session, workspace.value, operation.value), receipt: null};
      await inspect(api, api.state.deletion);
    });
  });
}

function freezeContent() {
  for (const node of byId('content').querySelectorAll('input,textarea,select,button')) { node.disabled = true; delete node.dataset.busy; }
}

function clearWorkspace(api) {
  api.state.workspace = null;
  byId('workspace-tabs').hidden = true;
  for (const id of ['workspace-heading', 'content', 'workspaces']) byId(id).replaceChildren();
}
