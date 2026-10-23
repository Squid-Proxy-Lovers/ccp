// SPDX-License-Identifier: AGPL-3.0-or-later
import {canTransferOwnership, TransferAttempt, workspaceSnapshot, wire} from './model.js';
import {byId, el, button, field, actions, notice, copy} from './view.js';

export function showTransfer(api, admin) {
  if (!api.mayLeave()) return;
  const workspace = api.state.workspace;
  if (!canTransferOwnership(api.state.session, workspace, admin)) throw new Error('Only the current owner of an active workspace can transfer to another enabled workspace administrator.');
  const root = byId('content').querySelector('.team-editor'); root.replaceChildren();
  const form = el('form'); root.append(form);
  form.append(el('h3', 'Transfer workspace ownership'), el('p', `Workspace: ${workspace.name} · ${workspace.id}`),
    el('p', `Current owner: ${api.state.session.identity.principal_id}`), el('p', `New owner: ${admin.name} · ${admin.id}`));
  form.append(el('p', 'There is one owner at a time. The recipient’s existing workspace key will gain owner powers. Every workspace key of the former owner will be revoked, including this login key. Unrelated roles, keys and stored content are preserved. No new key secret is returned.', 'warning'));
  form.append(el('p', 'The server requires an enabled current workspace administrator with a live scoped key record. A record does not prove delivery; verify privately that the recipient can use their key before transferring.'));
  const label = el('label', undefined, 'confirm-row'), delivered = el('input');
  delivered.type = 'checkbox'; delivered.required = true;
  label.append(delivered, el('span', 'I have delivered this administrator’s workspace key and confirmed that they can use it.')); form.append(label);
  const target = field(form, 'Confirm new owner principal ID'); target.required = true;
  form.addEventListener('input', () => api.draft(JSON.stringify({workspace_id: workspace.id, new_owner_id: admin.id,
    former_owner_id: api.state.session.identity.principal_id, delivery_confirmed: delivered.checked, typed_confirmation: target.value}, null, 2)));
  const submit = el('button', 'Confirm ownership transfer', 'danger'); submit.type = 'submit';
  actions(form, submit, button('Discard transfer draft', () => {
    if (!confirm('Discard the local ownership-transfer draft?')) return;
    api.clean(); root.replaceChildren(); notice('Local transfer draft discarded.');
  }));
  form.addEventListener('submit', event => {
    event.preventDefault(); api.run(async () => {
      if (api.state.pending) throw new Error('Resolve the original request before another transfer.');
      const attempt = new TransferAttempt(api.state.session, workspace, admin, delivered.checked, target.value.trim());
      if (!confirm(`Transfer ${workspace.name} (${workspace.id}) to ${admin.name} (${admin.id})? All former-owner workspace keys, including this login, will lose authority. The recipient’s existing key becomes the owner key.`)) return;
      await api.verify();
      api.state.pending = attempt;
      try {
        const request = attempt.request(api.state.session);
        const result = await wire(attempt.path, request.body, request.proof);
        if (result?.completed !== true) throw new Error('The transfer response could not be verified.');
      } catch (error) { uncertain(api, attempt, error); return; }
      api.state.pending = null; api.clean(); freeze();
      api.lock(`Ownership transfer confirmed for workspace ${workspace.name} (${workspace.id}). New owner: ${admin.name} (${admin.id}). All former-owner workspace keys have been revoked. Sign in using the new owner’s existing workspace key. No new secret was created. Other workspace/global roles and stored content are preserved.`);
    });
  });
  delivered.focus();
}

function freeze() {
  for (const node of byId('content').querySelectorAll('button,input,textarea,select')) { node.disabled = true; delete node.dataset.busy; }
}

function uncertain(api, attempt, error) {
  freeze();
  const box = el('section', undefined, 'warning transfer-pending');
  box.append(el('h3', 'Ownership transfer outcome unconfirmed'), el('p', error.message), el('p', 'There is no operation ID or safe retry. The transfer may have completed and revoked your former-owner keys. Read current ownership if this key still has access, or ask the intended new owner, an instance admin or the host operator to inspect. An observation does not prove which request changed ownership.'));
  field(box, 'Original ownership transfer', {value: attempt.body, area: true, readOnly: true});
  actions(box, button('Copy original transfer', () => copy(attempt.body)), button('Read current ownership', () => api.run(async () => {
    const value = await api.request('/api/v1/admin/workspaces/inspect', {workspace_id: attempt.workspace_id});
    workspaceSnapshot(value, attempt.workspace_id);
    if (typeof value.owner_id !== 'string') throw new Error('The current owner metadata could not be verified.');
    box.querySelector('.ownership-observation')?.remove();
    box.append(el('p', `Last read owner: ${value.owner_id}. This observation does not confirm attribution or cancel the original request.`, 'ownership-observation'));
  })), button('Forget transfer locally', () => {
    if (!confirm('Forget this transfer locally? It is not cancelled and may still complete. Copy the original endpoint and target before leaving.')) return;
    api.state.pending = null; api.draft(attempt.body);
    box.replaceChildren(el('h3', 'Original transfer retained as a local copy'), el('p', 'The server action remains uncancelled. Ask an authorized key holder to inspect current ownership before deciding on any further action.'));
    field(box, 'Local transfer copy', {value: attempt.body, area: true, readOnly: true});
    actions(box, button('Copy transfer', () => copy(attempt.body)), button('Discard local transfer copy', () => {
      if (!confirm('Discard this local copy? The server transfer remains uncancelled.')) return;
      api.clean();
      const root = byId('content'); root.replaceChildren(el('h2', 'Local transfer copy discarded'), el('p', 'No server cancellation or current ownership read is implied.'));
      actions(root, button('Read current workspace', () => api.run(() => api.reloadWorkspace())));
      notice('Local transfer copy discarded. Read current ownership with an authorized key.');
    }));
  }));
  byId('content').prepend(box); notice('Ownership transfer is unconfirmed. No retry was sent; keep the original request.');
}
