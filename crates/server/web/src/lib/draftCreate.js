// SPDX-License-Identifier: AGPL-3.0-or-later
// The create-then-drop flow shared by workspace, shelf and book creation.
// The drop is performed here, directly against the draft store, rather than
// through an injectable "drop" callback a caller (or a mutation) could
// silently replace with a no-op: the draft dropped on success, and the
// draft named "kept" on failure, is always the one that was actually
// submitted -- never a replacement typed into the reacquired form while
// the write's request was in flight.
import {dropDraftIfCurrent, isCurrentDraft} from './drafts.js';

/**
 * @param {object} deps
 * @param {Record<string, Record<string,string>>} deps.drafts
 * @param {string} deps.key
 * @param {Record<string,string>} deps.submitted
 * @param {(confirmed: (data: unknown) => string) => Promise<unknown>} deps.request
 *   performs the network write, forwarding `confirmed` through to it. Called
 *   only when the post-write session check fails (the write is already known
 *   to have happened), it drops the draft here before returning its text.
 * @param {(data: unknown) => string} deps.confirmedText
 * @returns {Promise<{ok: true, result: unknown} | {ok: false, error: unknown, keptCurrent: boolean}>}
 *   `keptCurrent` is only meaningful on failure: whether the submitted draft
 *   is still the live one (so "your draft is kept" is true), or something
 *   else (a discard, a replacement) already happened to it.
 */
export async function submitDraftCreate({drafts, key, submitted, request, confirmedText}) {
  let result;
  try {
    result = await request(data => {
      dropDraftIfCurrent(drafts, key, submitted);
      return confirmedText(data);
    });
  }
  catch (error) { return {ok: false, error, keptCurrent: isCurrentDraft(drafts, key, submitted)}; }
  dropDraftIfCurrent(drafts, key, submitted);
  return {ok: true, result};
}
