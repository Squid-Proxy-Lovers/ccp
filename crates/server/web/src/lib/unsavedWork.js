// SPDX-License-Identifier: AGPL-3.0-or-later
// One decision, used everywhere something would be silently lost by leaving
// the current identity: sign-out, a hash-key sign-in, the beforeunload guard
// and LockedView. A kept creation draft, or an undismissed one-time
// workspace owner key (never shown again once dismissed, never persisted),
// both count.
import {hasAnyDraftContent} from './drafts.js';

/** @param {{drafts: Record<string, Record<string,string>>, createdWorkspace: unknown}} state */
export function hasUnsavedWork(state) {
  return hasAnyDraftContent(state.drafts) || state.createdWorkspace !== null;
}
