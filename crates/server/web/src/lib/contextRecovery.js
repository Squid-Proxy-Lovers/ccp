// SPDX-License-Identifier: AGPL-3.0-or-later
// A closed editor's recovery copy owns a separate draft record. Its controls
// must never clear or copy the draft of an editor opened afterwards.
export function captureContextRecovery(app, key) {
  if (!app.draft) return;
  const text = app.draft;
  const record = app.formDraft(key);
  record.text = text;
  app.clean();
}

export function discardContextRecovery(app, key, record) {
  if (app.peekFormDraft(key) !== record) return false;
  app.dropFormDraft(key);
  return true;
}

export function clearCopiedContextRecovery(app, prefix) {
  for (const [key, record] of Object.entries(app.drafts)) {
    if (key.startsWith(prefix) && record.copied === 'true') app.dropFormDraft(key);
  }
}

export async function focusRetiredContext(app, hadFocus, afterDom, document) {
  if (!hadFocus) return;
  await afterDom();
  app.effect(() => {
    if (document.activeElement === document.body) document.getElementById('context-inactive-notice')?.focus();
  });
}
