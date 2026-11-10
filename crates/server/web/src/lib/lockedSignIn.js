// SPDX-License-Identifier: AGPL-3.0-or-later
// The locked-screen discard decision, including the asynchronous confirmation.
function discardSnapshot(state) {
  // Compare the workspace ID, never the raw owner key. New creates have new
  // IDs. A session replacement also invalidates an open discard decision.
  return {
    recovery: state.recoveryText(),
    ownerWorkspaceId: state.createdWorkspace?.workspace_id ?? null,
    deletion: state.deletion,
    pending: state.pending,
    session: state.session,
  };
}

export async function discardAndSignIn(state, confirmAction, reset) {
  const snapshot = discardSnapshot(state);
  const recovery = snapshot.recovery;
  const ownerKey = snapshot.ownerWorkspaceId !== null;
  if (!recovery && !ownerKey && !state.deletion) { reset(); return; }
  const deletionOnly = Boolean(state.deletion && (!state.pending || state.pending === state.deletion.reference)) && !recovery && !ownerKey;
  const ok = await confirmAction(deletionOnly
    ? {title: 'Return to sign in?', body: 'Deletion tracking stays on this page. Any server request remains uncancelled.', confirm: 'Return to sign in'}
    : {
      title: 'Discard local work?',
      body: `Any uncertain server write remains uncertain.${ownerKey ? ' This also discards the workspace owner key shown below.' : ''} Copy your work first if you need it.`,
      confirm: 'Discard and sign in', destructive: true,
    });
  if (!ok) return;
  const current = discardSnapshot(state);
  // Refuse an outdated approval; another click opens a fresh decision that
  // describes the current work. No secret is cleared by the old approval.
  if (Object.keys(snapshot).some(key => snapshot[key] !== current[key])) return;
  reset();
}
