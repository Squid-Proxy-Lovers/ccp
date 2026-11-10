// SPDX-License-Identifier: AGPL-3.0-or-later
// Workspace creation's one-time owner key safety: the key is shown exactly
// once and never persisted, so creating a second workspace before dismissing
// the first would silently overwrite and lose it. There is no safe default a
// confirm dialog could offer here, so this blocks instead of asking.
import {submitDraftCreate} from './draftCreate.js';

/**
 * @param {object} deps
 * @param {{workspaceCreates: number}} [deps.activity]
 * @param {() => boolean} deps.hasUndismissedOwnerKey
 * @param {Record<string, Record<string,string>>} deps.drafts
 * @param {string} deps.key
 * @param {Record<string,string>} deps.submitted
 * @param {(confirmed: (data: unknown) => string) => Promise<unknown>} deps.request
 * @param {(data: unknown) => string} deps.confirmedText
 * @returns {Promise<{ok: false, blocked: true} | {ok: true, blocked: false, result: unknown} | {ok: false, blocked: false, error: unknown, keptCurrent: boolean}>}
 */
export async function createWorkspaceFlow({hasUndismissedOwnerKey, drafts, key, submitted, request, confirmedText, activity = {workspaceCreates: 0}}) {
  if (hasUndismissedOwnerKey()) return {ok: false, blocked: true};
  activity.workspaceCreates += 1;
  try {
    const outcome = await submitDraftCreate({drafts, key, submitted, request, confirmedText});
    return {...outcome, blocked: false};
  } finally { activity.workspaceCreates -= 1; }
}
