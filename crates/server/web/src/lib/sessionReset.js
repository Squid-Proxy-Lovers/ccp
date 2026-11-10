// SPDX-License-Identifier: AGPL-3.0-or-later
// What signing out, or activating a new identity, actually clears. Plain
// data instead of a sequence of `this.x = ...` statements, so a mutation
// dropping one field (keeping the owner key, keeping drafts) is a diff in a
// returned object a test can assert on directly, not a removed statement
// buried in a method body.
export function clearedSessionState() {
  return {
    session: null,
    workspace: null,
    inspection: null,
    workspaces: [],
    workspacesNext: null,
    workspacesAfter: null,
    dirty: false,
    pending: null,
    draft: '',
    drafts: {},
    locked: false,
    lockMessage: '',
    createdWorkspace: null,
    accessSecret: null,
    view: 'overview',
  };
}
