// SPDX-License-Identifier: AGPL-3.0-or-later
// App.svelte's focus-handoff $effect dependency key: changes exactly when a
// transition should (eventually) refocus #page-title. `activating` must be
// part of the key so the run that lands when ShellSkeleton gives way to the
// real shell also refocuses, not just the run that entered the activating
// state (ShellSkeleton has no #page-title to focus).
export function focusHandoffKey(session, locked, activating, workspaceId, view, refreshTick = 0) {
  return `${session ? 'in' : 'out'}:${locked}:${activating}:${workspaceId ?? ''}:${view}:${refreshTick}`;
}
