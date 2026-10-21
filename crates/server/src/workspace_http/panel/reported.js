// SPDX-License-Identifier: AGPL-3.0-or-later
import {reportedWorkPage} from './model.js';
import {byId, el, button, actions, stamp, notice} from './view.js';

export async function showReported(api, after = null) {
  if (!api.mayLeave()) return;
  const root = byId('content'), workspace = api.state.workspace;
  let page;
  try { page = reportedWorkPage(await api.request('/api/v1/work/status/list', {workspace_id: workspace.id, after, limit: 32}), workspace.id); }
  catch (error) {
    if (api.state.locked) return;
    root.replaceChildren(el('h2', 'Reported work unavailable'), el('p', error.message), el('p', 'No empty workspace or report freshness was assumed.'));
    actions(root, button('Read current status messages', () => api.run(() => showReported(api)))); return;
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
    row.append(client, message, age); body.append(row);
  }
  const scroll = el('div', undefined, 'table-scroll'); scroll.append(table); root.append(scroll);
  if (!page.items.length) root.append(el('p', 'No uncleared status messages on this page. Silence is not a completion or offline signal.', 'empty'));
  if (page.next_cursor) actions(root, button('Next status messages', () => api.run(() => showReported(api, page.next_cursor))));
  if (after) actions(root, button('First status messages', () => api.run(() => showReported(api))));
  root.append(el('p', 'Stale messages keep their last text and timestamp. Ordinary CCP traffic does not refresh them. This human view is read-only; only authenticated clients publish explicit updates through the status API.', 'muted'));
  notice('Loaded a bounded page of reported work. No contact tracking or task completion was inferred.');
}
