// SPDX-License-Identifier: AGPL-3.0-or-later
import {el, button, actions, copy, stamp, field} from './view.js';
import {nativeHistoryPage, nativeScalarText} from './native_values.js';

const labels = {
  message_packs: 'Original entry row', message_history: 'Original append row',
  deleted_message_packs: 'Original archived-entry row', deleted_message_history: 'Original archived append row',
};

export async function showNativeHistory(api, root, entry, after = null) {
  const workspace = api.state.workspace.id;
  root.querySelector('.source-history')?.remove();
  const section = el('section', undefined, 'source-history');
  const heading = el('h3', 'Source history'); heading.tabIndex = -1;
  section.append(heading, el('p', 'Original records retained at conversion. Read-only and separate from recorded workspace revisions. Old names and timestamps do not establish ownership or permissions.', 'muted'));
  const body = el('div'); body.setAttribute('aria-live', 'polite');
  body.append(el('p', 'Loading source records…')); section.append(body); root.append(section);
  try {
    const result = await api.request('/api/v1/content/entries/native-history', {workspace_id: workspace, entry_id: entry, after, limit: 8});
    if (!section.isConnected || api.state.locked) return;
    const page = nativeHistoryPage(result, workspace, entry, after);
    body.replaceChildren();
    if (page.source === null) body.append(el('p', 'No original native source records are attached to this entry.', 'empty'));
    else renderPage(api, root, section, body, page, after);
    heading.focus();
  } catch {
    if (!section.isConnected || api.state.locked) return;
    body.replaceChildren(el('p', 'Source history is unavailable or could not be verified. Your entry edits are unchanged. Retry, or use the CLI to inspect the original records.', 'error'));
    actions(body, button('Retry source history', () => api.run(() => showNativeHistory(api, root, entry, after))));
  }
}

function renderPage(api, root, section, body, page, after) {
  body.append(el('p', `Converted ${stamp(page.source.converted_at)} · Source schema ${page.source.schema}`, 'metadata'));
  body.append(el('p', 'Source database SHA-256: ' + page.source.database_sha256, 'metadata identifier'));
  body.append(el('p', `Showing ${page.items.length} original records.`));
  if (!page.items.length) body.append(el('p', 'End of source history.', 'empty'));
  for (const item of page.items) body.append(record(item));
  const controls = [button('Copy source page as JSON', () => copyPage(body, page))];
  if (page.next_cursor !== null) controls.push(button('Next source records', () => api.run(() => showNativeHistory(api, root, page.entry_id, page.next_cursor))));
  if (after) controls.push(button('First source records', () => api.run(() => showNativeHistory(api, root, page.entry_id))));
  actions(body, ...controls);
  section.dataset.entry = page.entry_id;
}

function record(item) {
  const details = el('details', undefined, 'source-record history-item');
  details.append(el('summary', `Record ${item.ordinal} · ${labels[item.source_table]}`));
  details.addEventListener('toggle', () => {
    if (!details.open || details.dataset.loaded) return;
    details.dataset.loaded = 'true';
    details.append(el('p', `Source table ${item.source_table} · Original row key ${item.source_key}`, 'metadata identifier'));
    const table = el('table');
    const head = el('tr'); head.append(el('th', 'Original column'), el('th', 'Typed value'));
    const header = el('thead'); header.append(head); table.append(header);
    const body = el('tbody');
    item.row.columns.forEach((name, index) => {
      const row = el('tr'), value = item.row.values[index], cell = el('td');
      cell.append(el('span', value.type.replace('_', ' ').toUpperCase(), 'metadata'));
      const content = el('pre', nativeScalarText(value)); content.tabIndex = 0; cell.append(content);
      row.append(el('td', name), cell); body.append(row);
    });
    table.append(body); const scroll = el('div', undefined, 'table-scroll'); scroll.append(table); details.append(scroll);
  });
  return details;
}

async function copyPage(body, page) {
  const json = JSON.stringify(page, null, 2);
  body.querySelector('.source-export')?.remove();
  if (await copy(json)) return;
  const fallback = el('div', undefined, 'source-export');
  const text = field(fallback, 'Source page JSON (select and copy)', {value: json, area: true, readOnly: true});
  body.append(fallback); text.focus(); text.select();
}
