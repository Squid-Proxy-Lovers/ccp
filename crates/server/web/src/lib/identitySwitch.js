// SPDX-License-Identifier: AGPL-3.0-or-later
// Shared decision for anything that leaves the current identity behind
// (sign-out, or signing in as a different key over a live session). Creation
// drafts must never carry across identities and must never be silently lost
// to whatever the switch is doing while it is in flight. No internal imports
// (what counts as "has drafts" or their labels is injected), so this stays
// loadable with no bundler.

/** Busy keys under which a creation form's inputs and actions must freeze:
 * an identity switch is in flight, so a draft typed now would otherwise
 * either be silently lost (sign-out) or carry over to the next identity
 * (a hash-key sign-in while already signed in). */
const IDENTITY_SWITCH_KEYS = ['signout', 'login'];

export function identitySwitchInFlight(busy, busyKey) {
  return busy && IDENTITY_SWITCH_KEYS.includes(busyKey);
}

/**
 * Asks before discarding kept creation drafts, naming what would be lost.
 * Resolves true when there is nothing to lose, or the caller confirms losing it.
 * @param {object} deps
 * @param {() => boolean} deps.hasDrafts
 * @param {() => string[]} deps.labels
 * @param {(kept: string[]) => Promise<boolean>} deps.confirm
 * @returns {Promise<boolean>}
 */
export async function confirmIdentitySwitch({hasDrafts, labels, confirm}) {
  if (!hasDrafts()) return true;
  return confirm(labels());
}
