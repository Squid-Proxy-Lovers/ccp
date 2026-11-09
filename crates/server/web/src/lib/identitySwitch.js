// SPDX-License-Identifier: AGPL-3.0-or-later
// Every decision around leaving the current identity behind: signing out, or
// signing in as a different key over a live session. Creation drafts and an
// undismissed one-time owner key must never carry to the next identity, and
// must never be silently lost to whatever the switch is doing while it is in
// flight. Each flow below is the whole decision -- the confirm gate, the
// request, and the order forms/activates/clears happen in -- so the
// Svelte/`.ts` call site has nothing left to gate or reorder itself.

/** Busy keys under which a creation form's inputs and actions must freeze:
 * an identity switch is in flight, so a draft typed now would otherwise
 * either be silently lost (sign-out) or carry over to the next identity
 * (a hash-key sign-in while already signed in). */
const IDENTITY_SWITCH_KEYS = ['signout', 'login'];

export function identitySwitchInFlight(busy, busyKey) {
  return busy && IDENTITY_SWITCH_KEYS.includes(busyKey);
}

/** True while a form's own inputs and actions must freeze: an identity
 * switch is in flight, or this exact form's own create is. */
export function formFrozen(busy, busyKey, formKey) {
  return identitySwitchInFlight(busy, busyKey) || (busy && busyKey === formKey);
}

/** Plain-text summary of what a decline would keep and a confirm would lose. */
export function unsavedWorkSummary(draftLabelList, hasOwnerKey) {
  const parts = [...draftLabelList];
  if (hasOwnerKey) parts.push('the workspace owner key shown on screen');
  return parts.join(', ');
}

/**
 * Sign out: ask first only if there is unsaved work to lose, then verify,
 * log out and reset, in that order. Declining (or having nothing to lose)
 * skips straight past verify/logout/reset entirely.
 * @param {object} deps
 * @param {() => boolean} deps.hasUnsavedWork
 * @param {() => Promise<boolean>} deps.confirm
 * @param {() => Promise<unknown>} deps.verify
 * @param {(session: unknown) => Promise<void>} deps.logout
 * @param {() => void} deps.reset
 * @returns {Promise<boolean>} true if sign-out actually happened
 */
export async function runSignOut({hasUnsavedWork, confirm, verify, logout, reset}) {
  if (hasUnsavedWork() && !await confirm()) return false;
  const session = await verify();
  await logout(session);
  reset();
  return true;
}

/**
 * Establish a new identity (initial sign-in, or a rejected key leaving the
 * current one untouched): request the session, and only once that succeeds,
 * clear anything that must never carry to a new identity, then activate it.
 * A rejected request leaves every existing draft and owner key exactly as
 * they were -- `clearForNewIdentity` never runs before `request` resolves.
 * @param {object} deps
 * @param {() => Promise<unknown>} deps.request
 * @param {() => void} deps.clearForNewIdentity
 * @param {(session: unknown) => Promise<void>} deps.activate
 */
export async function runLogin({request, clearForNewIdentity, activate}) {
  const session = await request();
  clearForNewIdentity();
  await activate(session);
}

/**
 * A hash-key sign-in while already signed in: request the prospective
 * session first. If it is the same identity already signed in, this is a
 * no-op -- nothing is asked, nothing is cleared. Otherwise ask before
 * discarding unsaved work, and only on confirmation clear it and activate
 * the new session; declining leaves the current session, its drafts and its
 * owner key exactly as they were.
 * @param {object} deps
 * @param {() => Promise<unknown>} deps.request
 * @param {(session: unknown) => boolean} deps.sameIdentity
 * @param {() => boolean} deps.hasUnsavedWork
 * @param {() => Promise<boolean>} deps.confirm
 * @param {() => void} deps.clearForNewIdentity
 * @param {(session: unknown) => Promise<void>} deps.activate
 * @returns {Promise<'same' | 'declined' | 'switched'>}
 */
export async function runIdentitySwitch({request, sameIdentity, hasUnsavedWork, confirm, clearForNewIdentity, activate}) {
  const session = await request();
  if (sameIdentity(session)) return 'same';
  if (hasUnsavedWork() && !await confirm()) return 'declined';
  clearForNewIdentity();
  await activate(session);
  return 'switched';
}
