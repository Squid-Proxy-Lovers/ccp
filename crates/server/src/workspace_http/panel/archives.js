// SPDX-License-Identifier: AGPL-3.0-or-later
import {ContentAdminIntent, canManageInvitations, shelfState, archivedShelfPage, wire} from './model.js';
import {el, button, field, actions, notice, copy} from './view.js';
const content = '/api/v1/content/';
const scoped = api => canManageInvitations(api.state.session, api.state.workspace);

export async function showArchivedShelves(api, root, refresh, after = null) {
  if (!api.mayLeave()) return;
  const workspace = api.state.workspace;
  const page = archivedShelfPage(await api.request(content + 'shelves/archived', {workspace_id: workspace.id, after, limit: 32}), workspace.id);
  root.replaceChildren(el('h2', 'Archived shelves'), el('p', 'Summary metadata only. Books, entry bodies and history stay hidden until shelf restoration. No shelf payloads are purged here.'));
  actions(root, button('Return to context browser', () => api.run(async () => { if (api.mayLeave()) await refresh(); })));
  for (const shelf of page.items) actions(root, button(`${shelf.name} · Inspect archived shelf`, () => api.run(async () => { if (api.mayLeave()) await showShelf(api, root, shelf.id, refresh); })));
  if (!page.items.length) root.append(el('p', 'No archived shelves on this page.', 'empty'));
  if (page.next_cursor) actions(root, button('Next archived shelves', () => api.run(() => showArchivedShelves(api, root, refresh, page.next_cursor))));
  if (after) actions(root, button('First archived shelves', () => api.run(() => showArchivedShelves(api, root, refresh))));
}
export async function showShelf(api, root, shelfId, refresh) {
  const workspace = api.state.workspace;
  const state = shelfState(await api.request(content + 'shelves/state', {workspace_id: workspace.id, node_id: shelfId}), workspace.id, shelfId);
  root.replaceChildren(el('h2', 'Shelf archive state'), el('p', `Shelf ${shelfId} · Workspace ${workspace.id}`, 'identifier'), el('p', `Last read lifecycle revision ${state.revision} · ${state.archived ? 'Archived' : 'Active'}. This is an observation, not future authority.`));
  root.append(el('p', 'Archiving hides all descendants together while preserving their IDs, ownership and history. Restoring keeps individually archived entries archived. A same-name collision is refused without renaming or overwriting. Shelf archive does not purge payloads or start entry cleanup.'));
  actions(root, button('Return to context browser', () => api.run(async () => { if (api.mayLeave()) await refresh(); })), button('Browse archived shelves', () => api.run(() => showArchivedShelves(api, root, refresh))));
  if (scoped(api) && !workspace.archived) actions(root, button(state.archived ? 'Restore shelf' : 'Archive shelf', () => api.run(async () => {
    if (!api.mayLeave() || !confirm(`${state.archived ? 'Restore' : 'Archive'} shelf ${shelfId} in workspace ${workspace.id} at lifecycle revision ${state.revision}? Descendant IDs and individual archive state are preserved.`)) return;
    const intent = new ContentAdminIntent(api.state.session, workspace, state, 'shelf');
    await send(api, root, intent, refresh);
  }), state.archived ? 'primary' : 'danger'));
  else root.append(el('p', workspace.archived ? 'Shelf transitions require workspace reactivation. Read-only metadata remains available.' : 'Read-only: use this workspace’s owner/admin key for shelf management.', 'muted'));
}
export function purgeForm(api, root, entry, refresh) {
  if (!scoped(api) || entry.archived !== true) return;
  const details = el('details'); details.append(el('summary', 'Permanently purge archived entry'));
  const form = el('form'); details.append(form); root.append(details);
  form.append(el('p', `Permanently remove entry ${entry.id} at revision ${entry.revision}, including all payload and revision rows. This cannot be restored. Retained operation proofs and audit remain. Separate backups, SQLite free pages and journals are not securely erased. Explicit purge is allowed while this workspace is archived.`, 'warning'));
  const typed = field(form, 'Type archived entry ID to confirm purge'); typed.required = true; typed.autocomplete = 'off';
  const label = el('label', undefined, 'confirm-row'), consent = el('input'); consent.type = 'checkbox'; consent.required = true;
  label.append(consent, el('span', 'I understand that this entry and all its revisions cannot be restored.')); form.append(label);
  form.addEventListener('input', () => api.draft(JSON.stringify({entry_id: entry.id, expected_revision: entry.revision, typed_id: typed.value, confirm_purge: consent.checked})));
  const submit = el('button', 'Permanently purge entry', 'danger'); submit.type = 'submit';
  actions(form, submit, button('Discard purge draft', () => {
    if (!confirm('Discard this local purge draft?')) return; api.clean(); typed.value = ''; consent.checked = false; details.open = false;
  }));
  form.addEventListener('submit', event => { event.preventDefault(); api.run(async () => {
    if (api.state.pending) throw new Error('Resolve the original archive operation first.');
    if (!consent.checked) throw new Error('Explicitly confirm irreversible entry purge.');
    const intent = new ContentAdminIntent(api.state.session, api.state.workspace, entry, 'purge', typed.value);
    if (!confirm(`Permanently purge entry ${entry.id} in workspace ${entry.workspace_id} at revision ${entry.revision}? All payload and revision rows are removed; backups are separate.`)) return;
    await send(api, root, intent, refresh);
  }); });
}
function freeze(root) { for (const node of root.querySelectorAll('button,input,textarea,select')) { node.disabled = true; delete node.dataset.busy; } }
async function send(api, root, intent, refresh) {
  await api.verify(); api.state.pending = intent; freeze(root); let receipt;
  try { const request = intent.request(api.state.session); receipt = intent.receipt(await wire(intent.path, request.body, request.proof)); }
  catch (error) { if (!api.state.locked) pending(api, root, intent, refresh, error); return; }
  api.state.pending = null; api.clean();
  const confirmed = `${intent.action === 'purge' ? 'Entry purge' : 'Shelf transition'} confirmed. Historical receipt: ${JSON.stringify(receipt)}.`;
  root.replaceChildren(el('h2', 'Archive operation acknowledged'), el('p', confirmed), el('p', 'This receipt records the original operation, not current visibility or a forensic erasure guarantee.'));
  try { await api.verify(); } catch { api.lock(confirmed + ' Sign in again to inspect current metadata; do not create a replacement operation.'); return; }
  actions(root, button('Read current context metadata', () => api.run(async () => {
    try { if (intent.action === 'shelf') await showShelf(api, root, intent.target_id, refresh); else await refresh(); }
    catch (error) { if (api.state.locked) api.lock(confirmed + ' Current metadata is unavailable.'); else notice(confirmed + ' Current metadata read failed: ' + error.message); }
  })));
  notice(confirmed);
}
function pending(api, root, intent, refresh, error) {
  root.querySelector('.archive-pending')?.remove();
  const box = el('section', undefined, 'warning archive-pending');
  box.append(el('h3', 'Archive operation outcome unconfirmed'), el('p', error.message), el('p', 'Keep the exact original operation and key. No automatic retry was sent. An observation cannot attribute this request. A deliberate exact retry returns its historical receipt without reapplying. A conflict needs a separate decision after discarding this original intent; never silently replace its UUID or revision.'));
  field(box, 'Original archive operation', {value: intent.body, area: true, readOnly: true});
  actions(box, button('Check / retry exact archive operation', () => api.run(() => send(api, root, intent, refresh))), button('Copy original archive operation', () => copy(intent.body)), button('Read separate archive observation', () => api.run(async () => {
    const data = await api.request(content + (intent.action === 'shelf' ? 'shelves/state' : 'entries/list'), intent.action === 'shelf'
      ? {workspace_id: intent.workspace_id, node_id: intent.target_id} : {workspace_id: intent.workspace_id, book_id: intent.book_id, archived: true, limit: 32});
    if (intent.action === 'shelf') shelfState(data, intent.workspace_id, intent.target_id);
    box.querySelector('.archive-observation')?.remove(); const node = el('div', undefined, 'archive-observation'); node.append(el('p', 'Separate current observation only; not attribution or cancellation. Entry lists show one page only.'), el('pre', JSON.stringify(data, null, 2))); box.append(node);
  })), button('Forget archive operation locally', () => {
    if (!confirm('Forget this original operation locally? It is not cancelled and may have completed. Keep a copy.')) return;
    api.state.pending = null; api.draft(intent.body); root.replaceChildren(el('h2', 'Original archive operation retained as a local copy'));
    field(root, 'Local archive operation copy', {value: intent.body, area: true, readOnly: true});
    actions(root, button('Copy archive operation', () => copy(intent.body)), button('Discard local archive copy', () => {
      if (!confirm('Discard this local copy without cancelling any server operation?')) return;
      api.clean(); root.replaceChildren(el('h2', 'Local archive work discarded'), el('p', 'No server cancellation or successful read is implied.')); actions(root, button('Return to context browser', () => api.run(refresh)));
    }));
  })); root.prepend(box); notice('Archive action unconfirmed. The original operation is retained.');
}
