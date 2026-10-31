// SPDX-License-Identifier: AGPL-3.0-or-later
import {GuidanceIntent, canPublish, guidanceBundle, wire} from './model.js';
import {byId, el, button, field, actions, notice, stamp, copy} from './view.js';

const GET = '/api/v1/guidance/get';
const SAVE = '/api/v1/admin/guidance/save';

async function current(api, workspaceId) {
  return guidanceBundle(await api.request(GET, {workspace_id: workspaceId}), workspaceId);
}

export async function showGuidance(api) {
  const workspaceId = api.state.workspace?.id || null;
  byId('content').replaceChildren(el('p', 'Loading guidance…', 'muted'));
  try { render(api, await current(api, workspaceId), workspaceId); }
  catch (error) {
    if (!api.state.locked) byId('content').replaceChildren(el('p', 'Guidance unavailable. Use Refresh to try again.', 'empty'));
    throw error;
  }
}

function render(api, bundle, workspaceId) {
  const root = byId('content'); root.replaceChildren(el('h2', 'Instruction boards'));
  root.append(el('p', 'Global guidance takes precedence; workspace guidance adds compatible detail. Host instructions and actual server permissions remain authoritative.'));
  root.append(el('p', 'Each Save immediately updates live guidance. Agents receive initial or changed guidance with applicable CCP tool responses; this does not prove they read or followed it. This page shows a loaded snapshot.', 'muted'));
  const grid = el('div', undefined, 'board-grid'); root.append(grid);
  for (const board of [bundle.global, bundle.workspace].filter(Boolean)) boardEditor(api, grid, board, workspaceId);
}

function boardMetadata(root, board) {
  root.append(el('p', board.revision === 0 ? 'Not yet published · Revision 0' : `Loaded revision ${board.revision}`, 'metadata'));
  if (board.publisher) {
    const p = board.publisher;
    root.append(el('p', `${stamp(p.recorded_at)} · ${p.actor_kind} ${p.actor_id} · Principal ${p.principal_id}`, 'metadata'));
  }
}

function boardEditor(api, grid, board, workspaceId) {
  const root = el('section', undefined, 'guidance-board'); grid.append(root);
  const global = board.scope.kind === 'global';
  root.append(el('h3', global ? 'Global board' : 'Workspace board'));
  boardMetadata(root, board);
  const editable = !(global && workspaceId) && canPublish(api.state.session, board.scope, api.state.workspace);
  root.append(el('p', editable ? 'Your key can save this board. Save goes live immediately.'
    : global && workspaceId ? 'Read-only here. Instance-wide changes belong in Global guidance above.'
    : global ? 'Read-only with a workspace key.' : 'Read-only with this key or while the workspace is archived.', 'muted'));
  const form = el('form'); root.append(form);
  const text = field(form, global ? 'Global instructions' : 'Workspace instructions', {value: board.text, area: true, readOnly: !editable});
  text.rows = 14;
  const bytes = el('p', undefined, 'metadata'); form.append(bytes);
  const count = () => { const n = new TextEncoder().encode(text.value).length; bytes.textContent = `${n} / 8192 UTF-8 bytes`; return n; };
  count();
  if (!editable) return;
  const save = el('button', 'Save guidance', 'primary'); save.type = 'submit'; save.disabled = true;
  text.addEventListener('input', () => {
    api.draft(JSON.stringify({scope: board.scope, base_revision: board.revision, text: text.value}, null, 2));
    save.disabled = count() > 8192 || !!api.state.pending;
  });
  actions(form, save, button('Discard edits / reload guidance', () => api.run(async () => {
    if (api.state.pending) return notice('Resolve or explicitly forget the original save before reloading.');
    if (api.state.dirty && !confirm('Discard this guidance draft and read the current boards? Copy any text you want to keep first.')) return;
    // Preserve the draft until the read succeeds; a failed read can lock safely.
    const bundle = await current(api, workspaceId);
    api.clean(); render(api, bundle, workspaceId);
    notice('Loaded current guidance.');
  })));
  form.addEventListener('submit', event => {
    event.preventDefault();
    api.run(async () => {
      if (api.state.pending) throw new Error('Resolve or explicitly forget the original save first.');
      if (!text.value.length && !confirm('Clear this board by saving empty guidance? The accepted save goes live immediately.')) return;
      const intent = new GuidanceIntent(api.state.session, board.scope, board.revision, text.value);
      await api.verify();
      api.state.pending = intent;
      await send(api, root, text, save, intent, workspaceId);
    });
  });
}

async function send(api, root, text, save, intent, workspaceId) {
  text.readOnly = true; save.disabled = true;
  delete save.dataset.busy;
  let revision;
  try {
    await api.verify();
    const request = intent.request(api.state.session);
    revision = intent.receipt(await wire(SAVE, request.body, request.proof));
    api.state.pending = null; api.clean();
    root.querySelector('.guidance-pending')?.remove();
    root.querySelector('.guidance-confirmed')?.remove();
    root.append(el('p', `Save confirmed at revision ${revision}. Reload to read the current server board.`, 'guidance-confirmed metadata'));
    notice(`Guidance save confirmed at revision ${revision}.`);
  } catch (error) {
    if (api.state.locked) return;
    if (error.status === 401 || error.status === 403) { api.lock(error.message); return; }
    notice(error.message + ' Your draft and original save are retained.');
    pending(api, root, text, save, intent, workspaceId);
    return;
  }
  try {
    const bundle = await current(api, workspaceId);
    const original = JSON.parse(intent.body);
    const latest = original.scope.kind === 'global' ? bundle.global : bundle.workspace;
    render(api, bundle, workspaceId);
    notice(`Guidance save confirmed at revision ${revision}. Showing current revision ${latest.revision}.`);
  } catch (error) { notice(`Guidance save confirmed at revision ${revision}. Refresh failed: ${error.message}`); }
}

function pending(api, root, text, save, intent, workspaceId) {
  root.querySelector('.guidance-pending')?.remove();
  const box = el('section', undefined, 'guidance-pending warning');
  box.append(el('h3', 'Guidance save not confirmed'), el('p', 'The editor keeps the original draft, operation ID and base revision. Nothing retries automatically. Forgetting locally does not cancel server execution. Copy your draft before deliberately reloading and reconciling it with the current board.'));
  field(box, 'Original guidance save', {value: intent.body, area: true, readOnly: true});
  actions(box,
    button('Check / retry exact guidance save', () => api.run(() => send(api, root, text, save, intent, workspaceId))),
    button('Copy guidance draft', () => copy(JSON.parse(intent.body).text)),
    button('Compare current guidance', () => api.run(async () => {
      const bundle = await current(api, workspaceId);
      const scope = JSON.parse(intent.body).scope;
      const board = scope.kind === 'global' ? bundle.global : bundle.workspace;
      box.querySelector('.guidance-comparison')?.remove();
      const comparison = el('section', undefined, 'guidance-comparison');
      comparison.append(el('h3', 'Current server board')); boardMetadata(comparison, board);
      comparison.append(el('pre', board.text || '(Empty guidance)'));
      box.append(comparison);
      notice('Current guidance loaded for comparison. Your original draft and operation are unchanged.');
    })),
    button('Forget local guidance attempt', () => api.run(async () => {
      if (!confirm('Forget this local save attempt? It may already have completed or still complete later. This does not cancel it. Keep a copy before continuing.')) return;
      await api.verify();
      api.state.pending = null;
      text.readOnly = false;
      api.state.dirty = true;
      save.disabled = new TextEncoder().encode(text.value).length > 8192;
      box.replaceChildren(el('p', 'Local attempt forgotten. The editor still uses its original base revision. Copy/merge your draft, deliberately reload current guidance, then paste and review before a new Save.'));
    })));
  root.prepend(box);
}
