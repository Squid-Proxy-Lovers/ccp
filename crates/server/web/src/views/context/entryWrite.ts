// SPDX-License-Identifier: AGPL-3.0-or-later
// Entry writes carry an operation ID and an exact revision. The intent stays in
// app.pending until the server confirms it or the user explicitly forgets it, so
// a lost response is never silently replaced by a new write.
import {Intent} from '../../lib/model.js';
import {app} from '../../lib/app.svelte';
import {contentPath, message, type EntryReceipt, type HeldIntent} from './content';

export type EntryChange = Record<string, unknown> & {action: 'create' | 'append' | 'revise' | 'archive' | 'restore'};
export type WriteOutcome = {ok: true; receipt: EntryReceipt} | {ok: false; message: string} | null;

export async function startEntryWrite(change: EntryChange): Promise<HeldIntent> {
  if (app.pending) throw new Error('Resolve or explicitly forget the previous uncertain write first.');
  await app.verify();
  const intent: HeldIntent = new Intent(app.session, app.workspace?.id, change);
  app.pending = intent;
  return intent;
}

/** Sends (or exactly re-sends) the held intent. `null` means the session locked. */
export async function sendEntryWrite(intent: HeldIntent): Promise<WriteOutcome> {
  try {
    const request = intent.request(app.session);
    const receipt = await app.request<EntryReceipt>(contentPath('entries/mutate'), JSON.parse(request.body),
      data => `Entry ${actionOf(intent)} confirmed at revision ${data.revision} (entry ${data.entry_id}). Sign in again to read it; do not repeat the write.`);
    if (app.locked) return null;
    app.pending = null;
    return {ok: true, receipt};
  } catch (error) {
    if (app.locked) return null;
    return {ok: false, message: message(error)};
  }
}

export const actionOf = (intent: HeldIntent): EntryChange['action'] => JSON.parse(intent.body).change.action;
