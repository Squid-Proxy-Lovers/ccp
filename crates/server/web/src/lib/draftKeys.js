// SPDX-License-Identifier: AGPL-3.0-or-later
// The draft-store keys for each creation form, computed once here so every
// reader (the create, the discard, and the shelf badge that peeks a book
// draft without owning it) agrees on the same key for the same target.
export const WORKSPACE_DRAFT_KEY = 'create-workspace';
export const shelfDraftKey = workspaceId => `context.shelf.${workspaceId}`;
export const bookDraftKey = (workspaceId, shelfId) => `context.book.${workspaceId}.${shelfId}`;
