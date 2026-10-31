// SPDX-License-Identifier: AGPL-3.0-or-later
import {Intent, wire, exactRevision} from './model.js';
import {showNativeHistory} from './native_history.js';
import {showArchivedShelves, showShelf, purgeForm} from './archives.js';
import {byId, el, button, field, select, actions, notice, stamp, copy} from './view.js';

const path = name => '/api/v1/content/' + name;
const target = api => ({workspace_id: api.state.workspace.id});

export async function showContext(api) {
  const root = byId('content');
  root.replaceChildren();
  const layout = el('div', undefined, 'context-layout');
  const browser = el('aside', undefined, 'browser');
  const editor = el('article', undefined, 'editor'); editor.id = 'editor';
  layout.append(browser, editor); root.append(layout);
  browser.append(el('h2', 'Shared context'));
  editor.append(el('p', 'Choose a shelf, book and entry to read context or inspect its revisions.', 'empty'));
  await shelves(api, browser);
}

async function shelves(api, root, after = null) {
  const page = await api.request(path('shelves/list'), {...target(api), after, limit: 32});
  root.replaceChildren(el('h2', 'Shared context'));
  actions(root, button('Browse archived shelves', () => api.run(() => showArchivedShelves(api, byId('editor'), () => showContext(api)))));
  const shelf = select(root, 'Shelf', page.items, id => api.run(async () => {
    if (!api.mayLeave()) { shelf.value = shelf.dataset.previous || ''; return; }
    shelf.dataset.previous = id;
    below.replaceChildren(); byId('editor').replaceChildren();
    if (id) await books(api, below, id);
  }));
  if (!page.items.length) root.append(el('p', 'No active shelves on this page.', 'empty'));
  if (page.next_cursor) actions(root, button('Next shelves', () => api.run(async () => {
    if (api.mayLeave()) { byId('editor').replaceChildren(); await shelves(api, root, page.next_cursor); }
  })));
  if (after) actions(root, button('First shelves', () => api.run(async () => { if (api.mayLeave()) await shelves(api, root); })));
  if (api.writable()) containerForm(root, api, 'shelf', null, () => shelves(api, root));
  const below = el('div'); root.append(below);
}

async function books(api, root, shelfId, after = null) {
  const page = await api.request(path('books/list'), {...target(api), shelf_id: shelfId, after, limit: 32});
  root.replaceChildren();
  actions(root, button('Inspect shelf archive state', () => api.run(async () => { if (api.mayLeave()) await showShelf(api, byId('editor'), shelfId, () => showContext(api)); })));
  const book = select(root, 'Book', page.items, id => api.run(async () => {
    if (!api.mayLeave()) { book.value = book.dataset.previous || ''; return; }
    book.dataset.previous = id;
    below.replaceChildren(); byId('editor').replaceChildren();
    if (id) await entries(api, below, id);
  }));
  if (!page.items.length) root.append(el('p', 'No books on this page.', 'empty'));
  if (page.next_cursor) actions(root, button('Next books', () => api.run(async () => {
    if (api.mayLeave()) await books(api, root, shelfId, page.next_cursor);
  })));
  if (after) actions(root, button('First books', () => api.run(async () => { if (api.mayLeave()) await books(api, root, shelfId); })));
  if (api.writable()) containerForm(root, api, 'book', shelfId, () => books(api, root, shelfId));
  const below = el('div'); root.append(below);
}

function containerForm(root, api, kind, shelfId, refresh) {
  const details = el('details'); details.append(el('summary', 'Create ' + kind));
  const form = el('form');
  const name = field(form, 'Name'); name.required = true;
  const description = field(form, 'Description');
  const submit = el('button', 'Create ' + kind); submit.type = 'submit'; actions(form, submit);
  form.addEventListener('submit', event => {
    event.preventDefault();
    api.run(async () => {
      if (!api.mayLeave()) return;
      const body = {...target(api), name: name.value, description: description.value};
      if (shelfId) body.shelf_id = shelfId;
      notice('Creating ' + kind + '. If the response is lost, refresh the list before trying again.');
      await api.request(path(kind === 'shelf' ? 'shelves/create' : 'books/create'), body);
      await refresh(); notice(kind + ' created.');
    });
  });
  details.append(form); root.append(details);
}

async function entries(api, root, bookId, archived = false, after = null) {
  const page = await api.request(path('entries/list'), {...target(api), book_id: bookId, archived, after, limit: 32});
  root.replaceChildren(el('h3', 'Entries'));
  actions(root, button(archived ? 'Show active entries' : 'Show archived entries', () => api.run(async () => {
    if (api.mayLeave()) { byId('editor').replaceChildren(); await entries(api, root, bookId, !archived); }
  })));
  const refresh = () => entries(api, root, bookId, archived, after);
  if (!archived && api.writable()) actions(root, button('New entry', () => api.run(async () => {
    if (!api.mayLeave()) return;
    editor(api, null, {root, bookId, archived, refresh});
  })));
  const list = el('div', undefined, 'entry-list'); root.append(list);
  for (const entry of page.items) list.append(button(entry.name + (archived ? ' · Archived' : ''), () => api.run(async () => {
    if (!api.mayLeave()) return;
    if (archived) archivedEntry(api, entry, {root, bookId, archived, refresh});
    else await openEntry(api, entry.id, {root, bookId, archived, refresh});
  })));
  if (!page.items.length) list.append(el('p', archived ? 'No archived entries on this page.' : 'No active entries on this page.', 'empty'));
  if (page.next_cursor) actions(root, button('Next entries', () => api.run(async () => {
    if (api.mayLeave()) await entries(api, root, bookId, archived, page.next_cursor);
  })));
  if (after) actions(root, button('First entries', () => api.run(async () => { if (api.mayLeave()) await entries(api, root, bookId, archived); })));
}

async function openEntry(api, entryId, context) {
  const snapshot = await api.request(path('entries/get'), {...target(api), entry_id: entryId, revision: null});
  editor(api, snapshot, context);
}

function metadata(root, entry) {
  root.append(el('p', `Revision ${entry.revision} · Updated ${stamp(entry.updated_at)}`, 'metadata'));
  root.append(el('p', `Entry ${entry.id} · Owner ${entry.owner_client_id || 'No registered client (admin-managed)'}`, 'metadata'));
}

function archivedEntry(api, entry, context) {
  const root = byId('editor');
  root.replaceChildren(el('h2', entry.name)); metadata(root, entry);
  root.append(el('p', 'Archived entry. Body and history are unavailable until restoration.'));
  purgeForm(api, root, entry, () => showContext(api));
  if (api.writable()) actions(root, button('Restore entry', () => api.run(async () => {
    if (!api.mayLeave() || !confirm('Restore this entry? A name collision will be refused.')) return;
    await mutate(api, root, {action: 'restore', entry_id: entry.id, expected_revision: exactRevision(entry.revision)}, context);
  }), 'primary'));
}

function editor(api, snapshot, context) {
  const root = byId('editor');
  const entry = snapshot?.entry;
  const writable = api.writable();
  root.replaceChildren(el('h2', entry ? entry.name : 'New entry'));
  if (entry) metadata(root, entry);
  if (!writable) root.append(el('p', 'Read-only with this key or workspace state.', 'muted'));
  const form = el('form'); root.append(form);
  const name = entry ? null : field(form, 'Entry name');
  if (name) name.required = true;
  const description = field(form, 'Description', {value: snapshot?.content.description || '', readOnly: !writable});
  const labels = field(form, 'Labels (one per line)', {value: (snapshot?.content.labels || []).join('\n'), area: true, readOnly: !writable});
  labels.rows = 2;
  const text = field(form, 'Entry text', {value: snapshot?.content.text || '', area: true, readOnly: !writable});
  text.className = 'body';
  const draft = () => JSON.stringify({name: name?.value || entry?.name, description: description.value, labels: labels.value, text: text.value}, null, 2);
  form.addEventListener('input', () => api.draft(JSON.stringify({entry: JSON.parse(draft()), append_note: root.querySelector('[data-append]')?.value || ''})));
  if (writable) {
    const save = el('button', entry ? 'Save new revision' : 'Create entry', 'primary'); save.type = 'submit';
    actions(form, save, button('Discard edits / reload', () => api.run(async () => {
      if (api.state.pending) return notice('Resolve or explicitly forget the uncertain write first.');
      if (api.state.dirty && !confirm('Discard these unsaved edits and reload?')) return;
      // Keep the draft until the latest snapshot is successfully read.
      if (entry) await openEntry(api, entry.id, context); else editor(api, null, context);
      api.clean();
    })));
    form.addEventListener('submit', event => {
      event.preventDefault();
      api.run(async () => {
        if (root.querySelector('[data-append]')?.value.trim()) throw new Error('Append or discard your separate note before saving entry edits.');
        const content = {description: description.value, labels: labels.value.split('\n').filter(Boolean), text: text.value};
        const change = entry ? {action: 'revise', entry_id: entry.id, expected_revision: exactRevision(entry.revision), content}
          : {action: 'create', book_id: context.bookId, name: name.value, content};
        await mutate(api, root, change, context);
      });
    });
    if (entry) entryControls(api, root, entry, context);
  }
  if (entry) actions(root,
    button('Load revision history', () => api.run(() => history(api, root, entry.id))),
    button('Load source history', () => api.run(() => showNativeHistory(api, root, entry.id))));
}

function entryControls(api, root, entry, context) {
  actions(root, button('Archive entry', () => api.run(async () => {
    if (!api.mayLeave() || !confirm('Archive this entry? Its body and history become unavailable until restored.')) return;
    await mutate(api, root, {action: 'archive', entry_id: entry.id, expected_revision: exactRevision(entry.revision)}, context);
  }), 'danger'));
  const details = el('details'); details.append(el('summary', 'Append a note'));
  const form = el('form'); const note = field(form, 'Contribution', {area: true}); note.required = true;
  note.dataset.append = 'true';
  note.addEventListener('input', () => {
    const fields = [...root.querySelector('form').querySelectorAll('input,textarea')].map(node => node.value);
    api.draft(JSON.stringify({entry_fields: fields, append_note: note.value}));
  });
  const submit = el('button', 'Append note'); submit.type = 'submit';
  actions(form, submit, button('Discard note', () => {
    if (api.state.pending) return notice('Resolve or explicitly forget the uncertain write first.');
    if (!confirm('Discard the unsaved append note?')) return;
    note.value = '';
    if (!root.querySelector('form').dataset.changed) api.clean();
  }));
  form.addEventListener('submit', event => {
    event.preventDefault();
    api.run(async () => {
      // A full edit and a separate append must not silently discard each other.
      const editorForm = root.querySelector('form');
      if (editorForm.dataset.changed) throw new Error('Save or discard entry edits before appending.');
      await mutate(api, root, {action: 'append', entry_id: entry.id, expected_revision: exactRevision(entry.revision), text: note.value}, context);
    });
  });
  root.querySelector('form').addEventListener('input', () => { root.querySelector('form').dataset.changed = 'true'; });
  details.append(form); root.append(details);
}

async function mutate(api, root, change, context) {
  if (api.state.pending) throw new Error('Resolve or explicitly forget the previous uncertain write first.');
  await api.verify();
  const intent = new Intent(api.state.session, api.state.workspace.id, change);
  api.state.pending = intent;
  await sendIntent(api, root, intent, context);
}

async function sendIntent(api, root, intent, context) {
  // Freeze the editor while a write is uncertain. No edits can get silently
  // discarded by an eventual exact retry's success.
  for (const node of root.querySelectorAll('input,textarea')) node.readOnly = true;
  let result;
  try {
    await api.verify();
    const request = intent.request(api.state.session);
    result = await wire(path('entries/mutate'), request.body, request.proof);
    if (api.state.locked) return;
    api.state.pending = null;
  } catch (error) {
    if (api.state.locked) return;
    if (error.status === 401 || error.status === 403) { api.lock(error.message); return; }
    notice(error.message + ' The original write is retained below.');
    pendingControls(api, root, intent, context);
    return;
  }
  // Confirmation of the write is separate from refreshing current data. A
  // failed refresh must never relabel an acknowledged write as an unknown one.
  api.clean();
  notice(`Write confirmed at revision ${result.revision}.`);
  root.replaceChildren(el('p', `Write confirmed at revision ${result.revision}. Refreshing current data…`));
  try {
    await context.refresh();
    const action = JSON.parse(intent.body).change.action;
    if (action === 'archive') root.replaceChildren(el('p', 'Entry archived. Body and history are unavailable until restoration.'));
    else await openEntry(api, result.entry_id, context);
    notice(`Write confirmed at revision ${result.revision}.`);
  } catch (error) { notice(`Write confirmed at revision ${result.revision}. Refresh failed: ${error.message}`); }
}

function pendingControls(api, root, intent, context) {
  root.querySelector('.pending')?.remove();
  const box = el('section', undefined, 'pending warning');
  box.append(el('h3', 'Write not confirmed'), el('p', 'Do not create a replacement write. Check this exact original operation in the same session, or copy it for investigation. Forgetting locally does not cancel a request already sent.'));
  field(box, 'Original operation', {value: intent.body, area: true, readOnly: true});
  actions(box,
    button('Check / retry exact write', () => api.run(() => sendIntent(api, root, intent, context))),
    button('Copy original request', () => copy(intent.body)),
    button('Forget local attempt', () => {
      if (!confirm('Forget this local attempt? It may already have completed. Keep a copy before continuing. This does not cancel it.')) return;
      api.state.pending = null;
      box.replaceChildren(el('p', 'Local attempt forgotten. Copy your text before discarding/reloading the editor.'));
      // Keep local dirty text until an explicit discard/reload.
      api.state.dirty = true;
    }));
  root.prepend(box);
}

async function history(api, root, entryId, after = null) {
  const page = await api.request(path('entries/history'), {...target(api), entry_id: entryId, after, limit: 32});
  root.querySelector('.history')?.remove();
  const section = el('section', undefined, 'history'); section.append(el('h3', 'Recorded revisions'));
  for (const revision of page.items) {
    const row = el('div', undefined, 'history-item');
    row.append(el('strong', `Revision ${revision.revision} · ${revision.action}`));
    row.append(el('p', `${stamp(revision.recorded_at)} · ${revision.actor.kind} ${revision.actor.id}`, 'metadata'));
    if (revision.agent) row.append(el('p', `Agent label (self-reported): ${revision.agent}`, 'metadata'));
    row.append(button('Read this revision', () => api.run(async () => {
      const snapshot = await api.request(path('entries/get'), {...target(api), entry_id: entryId, revision: exactRevision(revision.revision)});
      row.querySelector('pre')?.remove();
      row.append(el('pre', `Description: ${snapshot.content.description}\nLabels: ${snapshot.content.labels.join(', ')}\n\n${snapshot.content.text}`));
    })));
    section.append(row);
  }
  if (page.next_cursor) actions(section, button('Next revisions', () => api.run(() => history(api, root, entryId, exactRevision(page.next_cursor)))));
  if (after) actions(section, button('First revisions', () => api.run(() => history(api, root, entryId))));
  root.append(section);
}
