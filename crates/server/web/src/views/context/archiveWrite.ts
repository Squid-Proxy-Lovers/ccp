// SPDX-License-Identifier: AGPL-3.0-or-later
// Shelf archive/restore and archived-entry purge. Each carries an operation ID
// and exact revision; the original stays in app.pending until a verified receipt.
import {app} from '../../lib/app.svelte';
import {notice} from '../../lib/notify.svelte';
import {message, type HeldIntent} from './content';

export interface AdminIntent extends HeldIntent {
  action: 'shelf' | 'purge'; workspace_id: string; book_id: string | null; target_id: string; path: string;
  receipt(value: unknown): unknown;
}
export type ArchiveOutcome = {state: 'confirmed'; text: string; receipt: string} | {state: 'unconfirmed'; error: string};

/** Sends (or exactly re-sends) the operation. `null` means the session locked. */
export async function sendArchive(intent: AdminIntent): Promise<ArchiveOutcome | null> {
  await app.verify();
  app.pending = intent;
  let receipt: unknown;
  try {
    const request = intent.request(app.session);
    const label = intent.action === 'purge' ? 'Entry purge' : 'Shelf transition';
    receipt = intent.receipt(await app.request(intent.path, JSON.parse(request.body),
      data => `${label} confirmed. Historical receipt: ${JSON.stringify(data)}. Sign in again to inspect current metadata; do not create a replacement operation.`));
  } catch (error) {
    if (app.locked) return null;
    notice('Archive action unconfirmed. The original operation is retained.', 'warning');
    return {state: 'unconfirmed', error: message(error)};
  }
  app.pending = null;
  app.clean();
  const text = `${intent.action === 'purge' ? 'Entry purge' : 'Shelf transition'} confirmed.`;
  const historical = JSON.stringify(receipt, null, 2);
  try { await app.verify(); } catch {
    app.lock(`${text} Historical receipt: ${JSON.stringify(receipt)}. Sign in again to inspect current metadata; do not create a replacement operation.`);
    return null;
  }
  notice(text, 'success');
  return {state: 'confirmed', text, receipt: historical};
}
