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
 *   on acknowledgement, before the post-write session check, it drops the
 *   draft here before returning its text.
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

/**
 * The message (if any) for a failed shelf/book create. Locked means the
 * create was confirmed and the page has already moved to the lock screen --
 * nothing more to say here, the lock message already covers it. Otherwise
 * the "draft is kept" claim only holds if `keptCurrent` says so; a draft
 * already discarded or replaced while this request was in flight gets a
 * different, honest message instead.
 * @param {object} deps
 * @param {boolean} deps.locked
 * @param {{error: unknown, keptCurrent: boolean}} deps.outcome
 * @param {string} deps.kind
 * @param {(error: unknown) => string} deps.message
 * @param {(error: unknown) => boolean} deps.outcomeUnknown
 * @param {(text: string, tone: 'destructive') => void} deps.notify
 */
export function reportCreateFailure({locked, outcome, kind, message, outcomeUnknown, notify}) {
  if (locked) return;
  const text = message(outcome.error);
  notify(outcome.keptCurrent
    ? (outcomeUnknown(outcome.error)
      ? `${text} Your ${kind} draft is kept. Refresh the list to check whether the ${kind} exists before you try again.`
      : `${text} Your ${kind} draft is kept.`)
    : `${text} Your original ${kind} draft was discarded or replaced before this finished. Check whether it exists before creating another.`,
    'destructive');
}
