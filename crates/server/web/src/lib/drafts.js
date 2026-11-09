// SPDX-License-Identifier: AGPL-3.0-or-later
// Creation-form draft storage: plain operations on the `drafts` record kept by
// app.svelte.ts (`Record<string, Record<string, string>>`), kept here so they
// stay testable without the rune compiler. Dropping a key must leave any other
// caller holding the old field object orphaned; re-acquire after a drop to get
// the live one back (see views/Workspaces.svelte and context/CreateNodeForm.svelte).

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** A readable name for a form-draft key such as `context.book.<uuid>.<uuid>`: "Context book draft". */
export function draftLabel(key) {
  const words = key.split(/[.:/]/).filter(part => part && !UUID.test(part)).join(' ').replace(/[-_]+/g, ' ');
  return `${words.charAt(0).toUpperCase()}${words.slice(1)} draft`;
}

/** Returns the live field object for `key`, creating an empty one if needed. */
export function acquireDraft(drafts, key) {
  drafts[key] ??= {};
  return drafts[key];
}

/** Reads `key` without creating it; use where acquiring would pollute `drafts` (e.g. a list render). */
export function peekDraft(drafts, key) {
  return drafts[key] ?? {};
}

/** Removes `key`. Any object a caller already holds for it becomes orphaned. */
export function dropDraft(drafts, key) {
  delete drafts[key];
}

export function hasDraftContent(fields) {
  return Object.values(fields).some(value => value.trim() !== '');
}

export function hasAnyDraftContent(drafts) {
  return Object.values(drafts).some(hasDraftContent);
}

/** One recovery block per non-empty draft: its label, then `field: value` lines. */
export function draftsRecoveryParts(drafts) {
  const parts = [];
  for (const [key, fields] of Object.entries(drafts)) {
    const filled = Object.entries(fields).filter(([, value]) => value.trim());
    if (filled.length) parts.push([draftLabel(key), ...filled.map(([name, value]) => `${name}: ${value}`)].join('\n'));
  }
  return parts;
}

/** Unsaved local work as plain text: the pending or edited body, then each form draft by name. */
export function recoveryText(pending, draft, drafts) {
  const parts = [];
  if (pending) parts.push(pending.body);
  else if (draft) parts.push(draft);
  parts.push(...draftsRecoveryParts(drafts));
  return parts.join('\n\n');
}
