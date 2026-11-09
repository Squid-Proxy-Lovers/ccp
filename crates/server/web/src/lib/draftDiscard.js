// SPDX-License-Identifier: AGPL-3.0-or-later
// The discard decision shared by workspace, shelf and book creation forms:
// never drop a draft while that form's create is in flight (the write may
// still succeed, and dropping the draft it was reading from is misleading at
// best), checked again after the confirmation dialog closes in case a create
// started while it was open. No internal imports: every effect (reading
// whether a create is still running, dropping the draft, confirming with the
// user) is an injected dependency, so this stays loadable with no bundler.

/**
 * @param {object} deps
 * @param {() => boolean} deps.isBlocked - true while a matching create is in flight
 * @param {() => Promise<boolean>} deps.confirm
 * @param {() => boolean} deps.drop - drops the draft iff it's still the one asked
 *   about (identity-safe); returns whether it actually dropped anything
 * @returns {Promise<boolean>} true if the draft was actually dropped
 */
export async function discardDraft({isBlocked, confirm, drop}) {
  if (isBlocked()) return false;
  if (!await confirm()) return false;
  if (isBlocked()) return false;
  return drop();
}
