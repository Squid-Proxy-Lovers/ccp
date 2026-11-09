// SPDX-License-Identifier: AGPL-3.0-or-later
// The discard decision shared by workspace, shelf and book creation forms.
// The drop, and the re-acquire that must follow a real drop (so later typing
// is tracked again instead of landing in an orphaned object), both happen
// here, directly against the draft store -- not through an injectable
// callback a caller or a mutation could silently drop. Never discards while
// that form's own create is in flight (the write may still succeed, and
// dropping the draft it is reading from is misleading at best), checked
// again after the confirmation dialog closes in case a create started while
// it was open.
import {dropDraftIfCurrent, acquireDraft} from './drafts.js';

/**
 * @param {object} deps
 * @param {Record<string, Record<string,string>>} deps.drafts
 * @param {string} deps.key
 * @param {Record<string,string>} deps.current - the draft object the user asked to discard
 * @param {() => boolean} deps.isBlocked - true while a matching create is in flight
 * @param {() => Promise<boolean>} deps.confirm
 * @returns {Promise<Record<string,string> | null>} the fresh, empty draft that
 *   replaced the discarded one, or null if nothing was dropped (blocked or declined)
 */
export async function discardDraft({drafts, key, current, isBlocked, confirm}) {
  if (isBlocked()) return null;
  if (!await confirm()) return null;
  if (isBlocked()) return null;
  if (!dropDraftIfCurrent(drafts, key, current)) return null;
  return acquireDraft(drafts, key);
}
