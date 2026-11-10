// SPDX-License-Identifier: AGPL-3.0-or-later
// Invalidating an action releases its slot without letting its late finally
// release the next action's slot. Locks alone do not release serialization.
export function releaseAction(state) {
  state.actionOwner = null;
  state.busy = false;
  state.busyKey = '';
}
