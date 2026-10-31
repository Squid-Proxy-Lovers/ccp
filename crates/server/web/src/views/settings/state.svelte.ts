// SPDX-License-Identifier: AGPL-3.0-or-later
// Per-visit state for workspace settings. Writes follow the original panel:
// lifecycle and ownership transfer have no operation ID and are never resent;
// retention changes carry an operation ID and the loaded base revision. After
// any acknowledged or unconfirmed write the cards freeze until a fresh read.
import {app} from '../../lib/app.svelte';
import {LifecycleAttempt, RetentionIntent, TransferAttempt, WorkspaceDeletionIntent, canManageTeam, canTransferOwnership,
  deletionPreview, retentionPolicy, retentionSeconds, wire, workspaceSnapshot} from '../../lib/model.js';
import {notice} from '../../lib/notify.svelte';
import {confirmAction} from '../../lib/ui/confirm.svelte';
import type {Session, Workspace} from '../../lib/types';
import {DELETION, startDeletion, tracked} from './deletion';

const RETENTION_GET = '/api/v1/admin/content/retention/get';
const RETENTION_CHANGE = '/api/v1/admin/content/retention/change';
const INSPECT = '/api/v1/admin/workspaces/inspect';
const ROSTER_PAGE = 32;

export interface Policy { workspace_id: string; revision: number; retention_seconds: number | null; changed_at: number | null }
export interface Admin { id: string; name: string; enabled: boolean; is_owner: boolean }
export interface Preview { workspace_id: string; name: string; lifecycle_revision: number }
export type Attempt = LifecycleAttempt | RetentionIntent | TransferAttempt;
/** `pending`: outcome unknown. `forgotten`: local tracking dropped, copy kept. `discarded`: copy dropped too. */
export interface Uncertain { attempt: Attempt; error: string; observation: string; stage: 'pending' | 'forgotten' | 'discarded' }

const message = (error: unknown): string => (error as Error).message;
/** Model attempts are plain frozen objects; `app.pending` holds them by identity. */
/** A retention window in the largest whole unit: "30 days", "12 hours", "90 seconds". */
export function duration(seconds: number): string {
  const units: [string, number][] = [['day', 86400], ['hour', 3600], ['minute', 60]];
  const [unit, size] = units.find(([, size]) => seconds % size === 0) ?? ['second', 1];
  const count = seconds / size;
  return `${count.toLocaleString()} ${unit}${count === 1 ? '' : 's'}`;
}
export const describePolicy = (seconds: number | null): string => seconds === null ? 'Manual retention' : duration(seconds);

export class SettingsState {
  policy = $state.raw<Policy | null>(null);
  policyError = $state('');
  loading = $state(true);
  /** Set after an acknowledged or unconfirmed write; cleared by a fresh read. */
  frozen = $state(false);
  uncertain = $state.raw<Uncertain | null>(null);
  lifecycleAck = $state('');
  lifecycleNote = $state('');
  retentionReceipt = $state('');
  retention = $state({duration: '' as string | number, unit: '86400', confirmed: false});

  roster = $state.raw<Admin[]>([]);
  rosterNext = $state<string | null>(null);
  rosterAfter = $state<string | null>(null);
  rosterError = $state('');
  rosterLoading = $state(false);
  admin = $state.raw<Admin | null>(null);
  transfer = $state({delivered: false, target: ''});

  preview = $state.raw<Preview | null>(null);
  deletion = $state({typed: '', confirmed: false});

  get session(): Session { return app.session as Session; }
  get workspace(): Workspace { return app.workspace as Workspace; }
  get ownerTools(): boolean { return Boolean(app.session && canManageTeam(app.session, app.workspace)); }

  async init(): Promise<void> {
    await Promise.all([this.loadPolicy(), this.loadRoster(null)]);
  }

  // ---- reads ---------------------------------------------------------------
  async loadPolicy(): Promise<Policy | null> {
    const workspace = app.workspace;
    if (!workspace) return null;
    const generation = app.generation;
    try {
      const policy = retentionPolicy(await app.request(RETENTION_GET, {workspace_id: workspace.id}), workspace.id) as Policy;
      this.policy = policy;
      this.policyError = '';
      return policy;
    } catch (error) {
      if (!app.locked && generation === app.generation) {
        this.policy = null;
        this.policyError = message(error);
        notice('Retention policy unavailable. ' + message(error), 'warning');
      }
      return null;
    } finally { this.loading = false; }
  }

  async loadRoster(after: string | null): Promise<void> {
    if (!this.ownerTools || this.workspace.archived) { this.roster = []; return; }
    const generation = app.generation;
    this.rosterLoading = true;
    try {
      const page = await app.request<{admins: Admin[]; next_cursor: string | null}>('/api/v1/admin/team/list',
        {workspace_id: this.workspace.id, after, limit: ROSTER_PAGE});
      this.roster = page.admins;
      this.rosterNext = page.next_cursor;
      this.rosterAfter = after;
      this.rosterError = '';
    } catch (error) {
      if (!app.locked && generation === app.generation) this.rosterError = message(error);
    } finally { this.rosterLoading = false; }
  }

  #resetLocal(): void {
    this.frozen = false;
    this.uncertain = null;
    this.lifecycleAck = '';
    this.lifecycleNote = '';
    this.retentionReceipt = '';
    this.retention = {duration: '', unit: '86400', confirmed: false};
    this.admin = null;
    this.transfer = {delivered: false, target: ''};
    this.preview = null;
    this.deletion = {typed: '', confirmed: false};
  }

  /** Re-reads workspace metadata, the policy and the roster; returns the policy or null. */
  async refresh(): Promise<Policy | null> {
    await app.inspectWorkspace();
    this.#resetLocal();
    const [policy] = await Promise.all([this.loadPolicy(), this.loadRoster(null)]);
    return policy;
  }

  async reload(lifecycleOnly = false): Promise<void> {
    if (app.pending) { notice('Resolve or explicitly forget the original request before reloading.', 'warning'); return; }
    if (app.dirty && !await confirmAction({title: 'Discard local draft?',
      body: 'Discard this local configuration draft and read current settings? Copy any text you want to keep first.',
      confirm: 'Discard and reload', destructive: true})) return;
    const policy = await this.refresh();
    if (policy || lifecycleOnly && !app.locked) {
      app.clean();
      notice(policy ? 'Loaded current workspace and retention settings.' : 'Loaded current workspace state. Retention policy remains unavailable.', 'success');
    }
  }

  async discardDraft(): Promise<void> {
    if (!await confirmAction({title: 'Discard this local draft?', body: 'Any forgotten server request remains uncancelled.',
      confirm: 'Discard draft', destructive: true})) return;
    app.clean();
    notice('Local configuration draft discarded. Current policy is still unavailable.');
  }

  // ---- lifecycle -------------------------------------------------------------
  async toggleLifecycle(): Promise<void> {
    if (!app.mayLeave()) return;
    const workspace = this.workspace;
    const ok = await confirmAction(workspace.archived
      ? {title: 'Reactivate workspace?', confirm: 'Reactivate workspace',
        body: `Reactivate ${workspace.name}? Content writes and admission resume, and timed cleanup continues from its remaining window.`}
      : {title: 'Archive workspace?', confirm: 'Archive workspace', destructive: true,
        body: `Archive ${workspace.name}? Context stays readable; content writes and new admission stop, and timed cleanup pauses.`});
    if (!ok) return;
    await app.verify();
    const attempt = new LifecycleAttempt(this.session, workspace);
    app.pending = attempt;
    try {
      const request = attempt.request(this.session);
      const result = await wire(attempt.path, request.body, request.proof);
      if (result?.completed !== true) throw new Error('The lifecycle response could not be verified.');
      app.pending = null;
      this.frozen = true;
      app.workspaceAcknowledged(attempt.archived);
      const label = attempt.archived ? 'Archive' : 'Reactivation';
      this.lifecycleAck = `${label} action confirmed. Use Refresh to read the current workspace state before another action.`;
      try {
        await this.refresh();
        notice(`${label} action confirmed. Last read workspace state: ${app.workspace?.archived ? 'Archived' : 'Active'}.`, 'success');
      } catch (error) { notice(`${label} action confirmed. Current state refresh failed: ${message(error)}`, 'warning'); }
    } catch (error) { this.#uncertainFrom(attempt, error); }
  }

  // ---- retention -------------------------------------------------------------
  retentionDraft(): void {
    const policy = this.policy;
    if (!policy) return;
    app.setDraft(JSON.stringify({workspace_id: policy.workspace_id, base_revision: policy.revision,
      duration: this.retention.duration, unit: this.retention.unit, confirm_timed_purge: this.retention.confirmed}, null, 2));
  }

  async enableRetention(): Promise<void> {
    if (app.pending) throw new Error('Resolve or explicitly forget the original request first.');
    const policy = this.policy;
    if (!policy) return;
    const seconds = retentionSeconds(this.retention.duration, this.retention.unit);
    await this.changeRetention(new RetentionIntent(this.session, policy, seconds, this.retention.confirmed));
  }

  async disableRetention(): Promise<void> {
    const policy = this.policy;
    if (!policy || !app.mayLeave()) return;
    if (!await confirmAction({title: 'Disable timed cleanup?', confirm: 'Disable timed cleanup', destructive: true,
      body: 'Archive clocks are removed and entries return to manual retention.'})) return;
    await this.changeRetention(new RetentionIntent(this.session, policy, null, false));
  }

  /** Sends, or deliberately re-sends, one exact retention change. */
  async changeRetention(intent: RetentionIntent): Promise<void> {
    if (app.pending && app.pending !== intent) throw new Error('Resolve the original request first.');
    await app.verify();
    app.pending = intent;
    let saved: Policy;
    try {
      const request = intent.request(this.session);
      saved = intent.receipt(await wire(RETENTION_CHANGE, request.body, request.proof)) as Policy;
      app.pending = null;
      app.clean();
      this.uncertain = null;
      this.frozen = true;
      this.retentionReceipt = `Saved revision ${saved.revision}: ${describePolicy(saved.retention_seconds)}. This is a historical receipt; use Refresh for the current policy.`;
    } catch (error) { this.#uncertainFrom(intent, error); return; }
    try {
      const latest = await this.refresh();
      notice(`Retention change confirmed at revision ${saved.revision}.${latest ? ' Showing current revision ' + latest.revision + '.' : ' Current policy is unavailable.'}`, 'success');
    } catch (error) { notice(`Retention change confirmed at revision ${saved.revision}. Refresh failed: ${message(error)}`, 'warning'); }
  }

  // ---- ownership transfer ----------------------------------------------------
  async rosterPage(after: string | null): Promise<void> {
    if (!app.mayLeave()) return;
    this.admin = null;
    await this.loadRoster(after);
  }

  prepareTransfer(admin: Admin): void {
    if (!app.mayLeave()) return;
    if (!canTransferOwnership(this.session, this.workspace, admin)) {
      throw new Error('Only the current owner of an active workspace can transfer to another enabled workspace administrator.');
    }
    this.transfer = {delivered: false, target: ''};
    this.admin = admin;
  }

  transferDraft(): void {
    if (!this.admin) return;
    app.setDraft(JSON.stringify({workspace_id: this.workspace.id, new_owner_id: this.admin.id,
      former_owner_id: this.session.identity.principal_id, delivery_confirmed: this.transfer.delivered,
      typed_confirmation: this.transfer.target}, null, 2));
  }

  async discardTransfer(): Promise<void> {
    if (!await confirmAction({title: 'Discard transfer draft?', body: 'Discard the local ownership-transfer draft?',
      confirm: 'Discard draft', destructive: true})) return;
    app.clean();
    this.admin = null;
    notice('Local transfer draft discarded.');
  }

  async submitTransfer(): Promise<void> {
    if (app.pending) throw new Error('Resolve the original request before another transfer.');
    const workspace = this.workspace, admin = this.admin;
    if (!admin) return;
    const attempt = new TransferAttempt(this.session, workspace, admin, this.transfer.delivered, this.transfer.target.trim());
    if (!await confirmAction({title: 'Transfer ownership?', confirm: 'Transfer ownership', destructive: true,
      body: `Transfer ${workspace.name} (${workspace.id}) to ${admin.name} (${admin.id})? All former-owner workspace keys, including this login, will lose authority. The recipient’s existing key becomes the owner key.`})) return;
    await app.verify();
    app.pending = attempt;
    try {
      const request = attempt.request(this.session);
      const result = await wire(attempt.path, request.body, request.proof);
      if (result?.completed !== true) throw new Error('The transfer response could not be verified.');
    } catch (error) { this.#uncertainFrom(attempt, error); return; }
    app.pending = null;
    app.clean();
    this.frozen = true;
    app.lock(`Ownership transfer confirmed for workspace ${workspace.name} (${workspace.id}). New owner: ${admin.name} (${admin.id}). All former-owner workspace keys have been revoked. Sign in using the new owner’s existing workspace key. No new secret was created. Other workspace/global roles and stored content are preserved.`);
  }

  // ---- unconfirmed writes ----------------------------------------------------
  #uncertainFrom(attempt: Attempt, error: unknown): void {
    if (app.locked) return;
    const status = (error as {status?: number}).status;
    if (attempt.kind !== 'transfer' && (status === 401 || status === 403)) { app.lock(message(error)); return; }
    this.frozen = true;
    this.uncertain = {attempt, error: message(error), observation: '', stage: 'pending'};
    notice(attempt.kind === 'transfer'
      ? 'Ownership transfer is unconfirmed. No retry was sent; keep the original request.'
      : message(error) + ' The original settings request is retained.', 'warning');
  }

  /** Reads current state. An observation never attributes the outcome to the original request. */
  async compare(): Promise<void> {
    const current = this.uncertain;
    if (!current) return;
    const id = current.attempt.workspace_id;
    let text: string;
    if (current.attempt.kind === 'retention') {
      const policy = retentionPolicy(await app.request(RETENTION_GET, {workspace_id: id}), id) as Policy;
      text = `Last read policy revision ${policy.revision}: ${describePolicy(policy.retention_seconds)}. The original request and base are unchanged.`;
    } else {
      const value = await app.request(INSPECT, {workspace_id: id});
      const workspace = workspaceSnapshot(value, id) as Workspace;
      if (current.attempt.kind === 'transfer') {
        if (typeof value.owner_id !== 'string') throw new Error('The current owner metadata could not be verified.');
        text = `Last read owner: ${value.owner_id}. This observation does not confirm attribution or cancel the original request.`;
      } else {
        app.workspaceObserved(workspace.archived);
        const state = workspace.archived ? 'Archived' : 'Active';
        this.lifecycleNote = `Last read: ${state}. Original request outcome remains unattributed.`;
        text = `Last read workspace state: ${state}. This is a state observation, not proof of this request’s outcome.`;
      }
    }
    if (this.uncertain === current) this.uncertain = {...current, observation: text};
  }

  async forget(): Promise<void> {
    const current = this.uncertain;
    if (!current) return;
    const transfer = current.attempt.kind === 'transfer';
    if (!await confirmAction(transfer
      ? {title: 'Forget this transfer locally?', confirm: 'Forget locally', destructive: true,
        body: 'It is not cancelled and may still complete. Copy the original endpoint and target before leaving.'}
      : {title: 'Forget this local request?', confirm: 'Forget locally', destructive: true,
        body: 'It may already have completed or still complete later. This does not cancel it. Keep a copy before continuing.'})) return;
    if (!transfer) await app.verify();
    app.pending = null;
    app.setDraft(current.attempt.body);
    this.uncertain = {...current, observation: '', stage: 'forgotten'};
  }

  async discardTransferCopy(): Promise<void> {
    const current = this.uncertain;
    if (!current || !await confirmAction({title: 'Discard local transfer copy?', confirm: 'Discard copy', destructive: true,
      body: 'The server transfer remains uncancelled.'})) return;
    app.clean();
    this.uncertain = {...current, stage: 'discarded'};
    notice('Local transfer copy discarded. Read current ownership with an authorized key.', 'warning');
  }

  // ---- permanent deletion ----------------------------------------------------
  async prepareDeletion(): Promise<void> {
    if (!app.mayLeave()) return;
    if (tracked()) {
      notice('Copy or explicitly forget the existing deletion tracking before preparing another deletion.', 'warning');
      return;
    }
    const workspace = this.workspace;
    const preview = deletionPreview(await app.request(DELETION + 'preview', {workspace_id: workspace.id}), workspace) as Preview;
    this.frozen = true;
    this.deletion = {typed: '', confirmed: false};
    this.preview = preview;
    this.deletionDraft();
  }

  deletionDraft(): void {
    const preview = this.preview;
    if (!preview) return;
    app.setDraft(JSON.stringify({unsubmitted: true, workspace_id: preview.workspace_id,
      lifecycle_revision: preview.lifecycle_revision, confirmation: this.deletion.typed, confirmed: this.deletion.confirmed}, null, 2));
  }

  async cancelDeletion(): Promise<void> {
    app.clean();
    await this.refresh();
  }

  async submitDeletion(): Promise<void> {
    if (app.pending || tracked()) throw new Error('Resolve or explicitly forget the existing deletion tracking first.');
    await app.verify();
    const intent = new WorkspaceDeletionIntent(this.session, this.workspace, this.preview, this.deletion.typed, this.deletion.confirmed);
    await startDeletion(intent);
  }
}
