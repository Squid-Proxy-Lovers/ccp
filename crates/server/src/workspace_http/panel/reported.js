// SPDX-License-Identifier: AGPL-3.0-or-later
import {reportedWorkPage, reportedWorkStatus, StatusClearIntent, wire, canManageInvitations} from './model.js';
import {byId, el, button, field, actions, stamp, notice, copy} from './view.js';

export async function showReported(api, after = null, confirmed = null) {
  if (!api.mayLeave()) return;
  const root = byId('content'), workspace = api.state.workspace;
  let page;
  try { page = reportedWorkPage(await api.request('/api/v1/work/status/list', {workspace_id: workspace.id, after, limit: 32}), workspace.id); }
  catch (error) {
    if (api.state.locked) { if (confirmed) api.lock(confirmed + ' Current status messages could not be read.'); return; }
    root.replaceChildren(el('h2', 'Reported work unavailable'), ...(confirmed ? [el('p', confirmed)] : []), el('p', error.message), el('p', 'No empty workspace or report freshness was assumed.'));
    actions(root, button('Read current status messages', () => api.run(() => showReported(api, null, confirmed)))); return;
  }
  root.replaceChildren(el('h2', 'Reported work'), el('p', 'Explicit client status messages, with optional agent labels supplied by the client. They do not prove an agent is online, working or finished. Recorded entry revisions remain separate in Context & history.'));
  root.append(el('p', `Server read: ${stamp(page.observed_at)} · Stale threshold: ${page.stale_after_seconds} seconds. Freshness labels refer to this read, not the present moment. Read again to observe current age.`, 'metadata'));
  actions(root, button('Read current status messages', () => api.run(() => showReported(api))));
  const table = el('table', undefined, 'reported-table'), head = el('thead'), columns = el('tr'), body = el('tbody');
  for (const name of ['Registered client & agent label', 'Work message', 'Report time & age']) columns.append(el('th', name));
  head.append(columns); table.append(head, body);
  for (const report of page.items) {
    const row = el('tr'), client = el('td', report.client_name), message = el('td'), age = el('td', report.stale ? 'Stale at last read' : 'Fresh at last read');
    client.append(el('small', report.client_id, 'identifier'));
    if (report.client_disabled) client.append(el('small', 'Client disabled instance-wide'));
    client.append(el('small', 'Agent label (self-reported): ' + (report.agent === null ? 'Not supplied' : report.agent)));
    message.append(el('pre', report.message)); age.append(el('small', stamp(report.updated_at)), el('small', `Revision ${report.revision}`), el('small', report.status_id, 'identifier'));
    if (canManageInvitations(api.state.session, workspace) && !workspace.archived) actions(age, button('Clear status message', () => api.run(async () => {
      if (!api.mayLeave()) return;
      const intent = new StatusClearIntent(api.state.session, workspace, report);
      if (!confirm(`Clear status ${report.status_id} from client ${report.client_id} in workspace ${workspace.id} at revision ${report.revision}? Current text and agent label are removed. Client ownership and operation receipts remain.`)) return;
      await sendClear(api, root, intent);
    }), 'danger'));
    row.append(client, message, age); body.append(row);
  }
  const scroll = el('div', undefined, 'table-scroll'); scroll.append(table); root.append(scroll);
  if (!page.items.length) root.append(el('p', 'No uncleared status messages on this page. Silence is not a completion or offline signal.', 'empty'));
  if (page.next_cursor) actions(root, button('Next status messages', () => api.run(() => showReported(api, page.next_cursor))));
  if (after) actions(root, button('First status messages', () => api.run(() => showReported(api))));
  root.append(el('p', 'Stale messages keep their last text and timestamp. Ordinary CCP traffic does not refresh them. Only authenticated clients publish status updates. Workspace owners/admins can explicitly clear messages in an active workspace; archived workspaces and instance keys provide read-only status access.', 'muted'));
  notice('Loaded a bounded page of reported work. No contact tracking or task completion was inferred.');
}


function freeze(root) {
  for (const node of root.querySelectorAll('button,input,textarea,select')) { node.disabled = true; delete node.dataset.busy; }
}
async function sendClear(api, root, intent) {
  await api.verify();
  api.state.pending = intent; freeze(root); let receipt;
  try { const request = intent.request(api.state.session); receipt = intent.receipt(await wire(intent.path, request.body, request.proof)); }
  catch (error) {
    if (error.status === 401 || error.status === 403) { api.lock('Status clear unconfirmed. ' + error.message); return; }
    if (!api.state.locked) pendingClear(api, root, intent, error); return;
  }
  api.state.pending = null; api.clean();
  const confirmed = 'Status clear confirmed. Historical receipt: ' + JSON.stringify(receipt) + '.';
  root.replaceChildren(el('h2', 'Status clear acknowledged'), el('p', confirmed), el('p', 'The original client still owns this slot. This receipt does not prove the current message is still cleared; the client may publish a later update.'));
  try { await api.verify(); } catch { api.lock(confirmed + ' Sign in again to read current messages; do not create a replacement operation.'); return; }
  actions(root, button('Read current status messages', () => api.run(async () => {
    try { await showReported(api, null, confirmed); }
    catch (error) { if (api.state.locked) api.lock(confirmed + ' Current messages are unavailable.'); else notice(confirmed + ' Current read failed: ' + error.message); }
  })));
  notice(confirmed);
}
function pendingClear(api, root, intent, error) {
  root.querySelector('.status-pending')?.remove();
  const box = el('section', undefined, 'warning status-pending');
  box.append(el('h3', 'Status clear outcome unconfirmed'), el('p', error.message), el('p', 'The exact original request is retained. No automatic retry was sent. A current read cannot attribute or cancel it. A conflict requires a separate decision; do not replace its UUID or revision silently.'));
  field(box, 'Original status clear', {value: intent.body, area: true, readOnly: true});
  actions(box, button('Check / retry exact status clear', () => api.run(() => sendClear(api, root, intent))), button('Copy original status clear', () => copy(intent.body)), button('Read separate status observation', () => api.run(async () => {
    const data = await api.request('/api/v1/work/status/get', {workspace_id: intent.workspace_id, status_id: intent.status_id});
    reportedWorkStatus(data, intent.workspace_id);
    if (data.workspace_id !== intent.workspace_id || data.status_id !== intent.status_id || data.client_id !== intent.client_id) throw new Error('The separate status observation did not match this slot.');
    box.querySelector('.status-observation')?.remove(); const node = el('div', undefined, 'status-observation');
    node.append(el('p', 'Separate current observation only; not proof of this operation or cancellation.'), el('pre', JSON.stringify(data, null, 2))); box.append(node);
  })), button('Forget status clear locally', () => {
    if (!confirm('Forget this local status-clear intent? It is not cancelled and may already have completed. Keep a copy.')) return;
    api.state.pending = null; api.draft(intent.body); root.replaceChildren(el('h2', 'Status clear retained as a local copy'));
    field(root, 'Local status-clear copy', {value: intent.body, area: true, readOnly: true});
    actions(root, button('Copy status-clear request', () => copy(intent.body)), button('Discard local status-clear copy', () => {
      if (!confirm('Discard this local copy without cancelling any server operation?')) return;
      api.clean(); root.replaceChildren(el('h2', 'Local status-clear work discarded'), el('p', 'No server cancellation or successful read is implied.'));
      actions(root, button('Return to reported work', () => api.run(() => showReported(api))));
    }));
  })); root.prepend(box); notice('Status clear unconfirmed. Keep the original request.');
}
