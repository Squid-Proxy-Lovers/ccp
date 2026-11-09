// SPDX-License-Identifier: AGPL-3.0-or-later
// The discard decision shared by workspace, shelf and book creation forms.
// The drop is performed here, directly against the draft store (not through
// an injectable "drop" callback that could be replaced with a no-op or a
// wholesale wipe): never drop a draft while that form's own create is in
// flight (the write may still succeed, and dropping the draft it is reading
// from is misleading at best), checked again after the confirmation dialog
// closes in case a create started while it was open.
import {dropDraftIfCurrent} from './drafts.js';

/**
 * @param {object} deps
 * @param {Record<string, Record<string,string>>} deps.drafts
 * @param {string} deps.key
 * @param {Record<string,string>} deps.current - the draft object the user asked to discard
 * @param {() => boolean} deps.isBlocked - true while a matching create is in flight
 * @param {() => Promise<boolean>} deps.confirm
 * @returns {Promise<boolean>} true if the draft was actually dropped
 */
export async function discardDraft({drafts, key, current, isBlocked, confirm}) {
  if (isBlocked()) return false;
  if (!await confirm()) return false;
  if (isBlocked()) return false;
  return dropDraftIfCurrent(drafts, key, current);
}
