// SPDX-License-Identifier: AGPL-3.0-or-later
// The create-then-drop flow shared by workspace, shelf and book creation:
// the draft dropped on success, and the draft named "kept" on failure, is
// always the one that was actually submitted -- never a replacement typed
// into the (now-reacquired) form while the write's request was in flight.

/**
 * @param {object} deps
 * @param {() => Promise<unknown>} deps.write - the network write; resolves with its
 *   result, or throws (the confirmed-write-but-session-lost case drops the
 *   draft itself, inside its own confirmed callback, before throwing)
 * @param {() => boolean} deps.dropIfCurrent - drop the submitted draft iff nothing
 *   replaced it since submit; returns whether it dropped anything
 * @param {() => boolean} deps.isCurrent - whether the submitted draft is still live
 * @returns {Promise<{ok: true, result: unknown} | {ok: false, error: unknown, keptCurrent: boolean}>}
 *   `keptCurrent` is only meaningful on failure: whether the submitted draft
 *   is still the live one (so "your draft is kept" is true), or something
 *   else (a discard, a replacement) already happened to it.
 */
export async function submitDraftCreate({write, dropIfCurrent, isCurrent}) {
  let result;
  try { result = await write(); }
  catch (error) { return {ok: false, error, keptCurrent: isCurrent()}; }
  dropIfCurrent();
  return {ok: true, result};
}
