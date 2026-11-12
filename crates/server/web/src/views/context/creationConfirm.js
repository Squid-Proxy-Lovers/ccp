// SPDX-License-Identifier: AGPL-3.0-or-later
// Text for a shelf or book create whose confirming session check failed. Mirrors
// entryWrite.ts's `confirmed` callback: the write is known to have happened, so
// the lock message must say so instead of leaving it an unconfirmed draft.
export function creationConfirmedText(kind, item) {
  const label = kind === 'shelf' ? 'Shelf' : 'Book';
  return `${label} “${item.name}” creation confirmed (id ${item.id}). Sign in again to see it; do not repeat the create.`;
}
