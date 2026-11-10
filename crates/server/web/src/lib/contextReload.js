// SPDX-License-Identifier: AGPL-3.0-or-later
// A paged list cannot prove that an off-page selection was removed. Only an
// active-item lookup returning NotFound retires it and its descendants.
export async function reloadContextSelection({selection, pages, read}) {
  const next = {...selection};
  for (const kind of ['shelf', 'book', 'entry']) {
    const page = pages[kind];
    if (kind !== 'shelf' && !next[kind === 'book' ? 'shelf' : 'book']) break;
    await page.reload();
    const selected = next[kind];
    if (!selected) continue;
    const listed = page.items.find(item => item.id === selected.id);
    if (listed) { next[kind] = listed; continue; }
    // Archived-entry summaries intentionally have no readable active body.
    if (kind === 'entry' && selected.archived) continue;
    try { next[kind] = await read(kind, selected.id); }
    catch (error) {
      if (error.status !== 404) throw error;
      return {...next, missing: kind};
    }
  }
  return next;
}
