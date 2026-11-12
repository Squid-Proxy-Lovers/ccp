// SPDX-License-Identifier: AGPL-3.0-or-later
// Workspace deletion start and status checks. Tracking lives in `app.deletion`
// so it survives sign-in changes; only body-free IDs and receipts are kept.
import type {ViewApp} from '../../lib/app.svelte';
import {WorkspaceDeletionIntent, deletionLookup, deletionReceipt as verifyReceipt, deletionStatusRequest} from '../../lib/model.js';
import type {PendingWork, Session} from '../../lib/types';

export const DELETION = '/api/v1/admin/workspaces/deletion/';

export interface Reference extends PendingWork { instance_id: string; workspace_id: string; operation_id: string }
export interface Receipt {
  workspace_id: string; operation_id: string; lifecycle_revision: number; started_at: number; completed_at: number | null;
}
export interface Tracked { reference: Reference; receipt: Receipt | null }

const deletionReceipt = verifyReceipt as (value: unknown, reference: unknown, previous?: Receipt | null) => Receipt;

export const tracked = (app: ViewApp): Tracked | null => app.deletion as Tracked | null;
const message = (error: unknown): string => (error as Error).message;
const status = (error: unknown): number | undefined => (error as {status?: number}).status;

export async function startDeletion(app: ViewApp, intent: WorkspaceDeletionIntent): Promise<void> {
  const reference = intent as unknown as Reference;
  app.pending = reference;
  app.deletion = {reference, receipt: null};
  try {
    const request = intent.request(app.session as Session);
    const value = deletionReceipt(await app.wire(DELETION + 'start', request.body, request.proof), intent);
    // Commit the body-free acknowledgment before any post-write session check.
    app.deletion = {reference, receipt: value};
    app.pending = null;
    app.clean();
  } catch (error) {
    app = app.clearWorkspace();
    if (status(error) === 401 || status(error) === 403) {
      app.lock('Deletion outcome is unconfirmed. Keep the original tracking details; permission loss does not prove it failed.');
    }
    app.notice('Deletion outcome unconfirmed. ' + message(error), 'warning');
    return;
  }
  app = app.clearWorkspace();
  app.notice('Deletion started. Cleanup completion is tracked separately.', 'success');
  if (app.session?.identity.scope.kind === 'workspace') {
    app.lock('Deletion started. This workspace key has lost its role. Save the receipt, then use an instance-admin key to check completion.');
  } else {
    try { await app.loadWorkspaces(); }
    catch (error) { app.notice('Deletion started. The receipt is retained. Workspace refresh failed: ' + message(error), 'warning'); }
  }
}

/** Reads status only; never resends the deletion. */
export async function inspectDeletion(app: ViewApp, current: Tracked): Promise<void> {
  try {
    const request = deletionStatusRequest(app.session as Session, current.reference);
    const value = await app.read(DELETION + 'status', request);
    const receipt = deletionReceipt(value, current.reference, current.receipt);
    if (tracked(app)?.reference !== current.reference) return;
    app.deletion = {reference: current.reference, receipt};
    if (app.pending === current.reference) { app.pending = null; app.clean(); }
    app.notice(receipt.completed_at === null ? 'Deletion start confirmed; cleanup is still pending.' : 'Workspace cleanup completion confirmed.', 'success');
  } catch (error) {
    app.notice('Deletion status could not be verified. The original details and last confirmed receipt are retained. ' + message(error), 'warning');
  }
}

export async function lookupDeletion(app: ViewApp, workspace: string, operation: string): Promise<void> {
  if (!app.mayLeave()) return;
  if (tracked(app)) throw new Error('Copy or explicitly forget the existing deletion tracking first.');
  await app.verify();
  const reference = deletionLookup(app.session as Session, workspace, operation) as Reference;
  app.deletion = {reference, receipt: null};
  await inspectDeletion(app, {reference, receipt: null});
}
