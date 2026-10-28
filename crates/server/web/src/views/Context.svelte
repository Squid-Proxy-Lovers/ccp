<script lang="ts">
  // Context browser: shelves | books | entries side by side from the md
  // breakpoint, a stacked drill-down below it, and one detail panel.
  // Selection is kept by ID, so a selected or newly created item stays visible
  // even when it is not on the page currently shown.
  import {onMount} from 'svelte';
  import {Archive, ChevronLeft, Library, Plus} from '@lucide/svelte';
  import Delayed from '../lib/ui/Delayed.svelte';
  import EntrySkeleton from './context/EntrySkeleton.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import {app} from '../lib/app.svelte';
  import {notice} from '../lib/notify.svelte';
  import ListPane from './context/ListPane.svelte';
  import CreateNodeForm from './context/CreateNodeForm.svelte';
  import EntryEditor from './context/EntryEditor.svelte';
  import ArchivedEntry from './context/ArchivedEntry.svelte';
  import ArchivedShelves from './context/ArchivedShelves.svelte';
  import ShelfState from './context/ShelfState.svelte';
  import ArchiveOperation from './context/ArchiveOperation.svelte';
  import {Pager} from './context/pager.svelte';
  import {contentPath, focusId, focusRow, message, outcomeUnknown, wide,
    type Book, type EntryReceipt, type EntrySummary, type Shelf, type Snapshot} from './context/content';
  import type {EntryChange} from './context/entryWrite';
  import type {AdminIntent, ArchiveOutcome} from './context/archiveWrite';

  type Stage = 'shelves' | 'books' | 'entries' | 'detail';
  type Detail =
    | {kind: 'none'}
    | {kind: 'loading'}
    | {kind: 'entry'; snapshot: Snapshot}
    | {kind: 'new-entry'; bookId: string}
    | {kind: 'archived-entry'; entry: EntrySummary}
    | {kind: 'archived-shelves'}
    | {kind: 'shelf-state'; shelfId: string}
    | {kind: 'archive-op'; intent: AdminIntent; outcome: ArchiveOutcome}
    | {kind: 'message'; title: string; text: string};

  const workspaceId = app.workspace?.id ?? '';
  const writable = app.writable();

  let shelf = $state.raw<Shelf | null>(null);
  let book = $state.raw<Book | null>(null);
  let entry = $state.raw<EntrySummary | null>(null);
  let archivedEntries = $state(false);
  let created = $state.raw<Set<string>>(new Set());
  let stage = $state<Stage>('shelves');
  let detail = $state.raw<Detail>({kind: 'none'});
  let detailSerial = $state(0);
  let detailFrom = $state<Stage>('shelves');

  const shelves = new Pager<Shelf>((after, limit) => app.request(contentPath('shelves/list'), {workspace_id: workspaceId, after, limit}));
  const books = new Pager<Book>((after, limit) => app.request(contentPath('books/list'), {workspace_id: workspaceId, shelf_id: shelf?.id, after, limit}));
  const entries = new Pager<EntrySummary>((after, limit) => app.request(contentPath('entries/list'), {workspace_id: workspaceId, book_id: book?.id, archived: archivedEntries, after, limit}));

  const pinnedEntry = $derived(entry && entry.book_id === book?.id && entry.archived === archivedEntries ? entry : null);
  const isNew = (item: {id: string}) => (created.has(item.id) ? 'New' : undefined);
  const paneClass = (pane: Stage) => (stage === pane ? 'flex' : 'hidden md:flex');

  // ---- detail -------------------------------------------------------------
  function show(next: Detail, from: Stage) {
    detail = next;
    detailSerial += 1;
    detailFrom = from;
    stage = 'detail';
    void focusId('context-detail-title');
  }

  function closeDetail() {
    detail = {kind: 'none'};
    detailSerial += 1;
  }

  async function openEntry(id: string) {
    const snapshot = await app.request<Snapshot>(contentPath('entries/get'), {workspace_id: workspaceId, entry_id: id, revision: null});
    entry = snapshot.entry;
    show({kind: 'entry', snapshot}, 'entries');
  }

  // ---- selection ------------------------------------------------------------
  function selectShelf(next: Shelf) {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      shelf = next; book = null; entry = null; archivedEntries = false;
      books.clear(); entries.clear(); closeDetail();
      stage = 'books';
      if (!wide()) void focusId('books-title');
      await books.first();
    }, 'select-shelf');
  }

  function selectBook(next: Book) {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      book = next; entry = null; archivedEntries = false;
      entries.clear(); closeDetail();
      stage = 'entries';
      if (!wide()) void focusId('entries-title');
      await entries.first();
    }, 'select-book');
  }

  function selectEntry(next: EntrySummary) {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      if (next.archived) { entry = next; show({kind: 'archived-entry', entry: next}, 'entries'); }
      else {
        detail = {kind: 'loading'};
        detailSerial += 1;
        try { await openEntry(next.id); } catch (error) { closeDetail(); throw error; }
      }
    }, 'select-entry');
  }

  function toggleArchived() {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      archivedEntries = !archivedEntries;
      entry = null;
      closeDetail();
      await entries.first();
    }, 'toggle-archived');
  }

  function newEntry() {
    if (!book) return;
    const bookId = book.id;
    void app.run(async () => {
      if (!app.mayLeave()) return;
      entry = null;
      show({kind: 'new-entry', bookId}, 'entries');
    }, 'new-entry');
  }

  function back(to: Stage) {
    stage = to;
    const row = to === 'shelves' && shelf ? `shelf:${shelf.id}` : to === 'books' && book ? `book:${book.id}` : to === 'entries' && entry ? `entry:${entry.id}` : '';
    void (row ? focusRow(row) : focusId(`${to}-title`));
  }

  // ---- creation (shelf and book) ---------------------------------------------
  // The acknowledged create and the following list read are reported apart:
  // a failed refresh never turns a confirmed create into an unknown outcome.
  async function create<T extends Shelf>(kind: 'shelf' | 'book', path: string, body: Record<string, unknown>,
    reveal: (item: T) => Promise<string>): Promise<boolean> {
    if (!app.mayLeave()) return false;
    let item: T;
    try { item = await app.request<T>(contentPath(path), {workspace_id: workspaceId, ...body}); }
    catch (error) {
      if (app.locked) return false;
      notice(outcomeUnknown(error)
        ? `${message(error)} Your ${kind} draft is kept. Refresh the list to check whether the ${kind} exists before you try again.`
        : `${message(error)} Your ${kind} draft is kept.`, 'destructive');
      return false;
    }
    created = new Set([...created, item.id]);
    const failure = await reveal(item);
    if (failure) notice(`${kind === 'shelf' ? 'Shelf' : 'Book'} “${item.name}” created and selected. The list refresh failed: ${failure} Use refresh to try again.`, 'warning');
    else notice(`${kind === 'shelf' ? 'Shelf' : 'Book'} “${item.name}” created.`, 'success');
    void focusRow(`${kind}:${item.id}`);
    return true;
  }

  // Runs every read; returns the first failure message, or '' when all succeeded.
  const refreshed = async (...loads: (() => Promise<void>)[]): Promise<string> => {
    let failure = '';
    for (const load of loads) {
      try { await load(); } catch (error) { if (app.locked) return ''; failure ||= message(error); }
    }
    return failure;
  };

  const createShelf = (name: string, description: string) => create<Shelf>('shelf', 'shelves/create', {name, description}, item => {
    shelf = item; book = null; entry = null; archivedEntries = false;
    books.clear(); entries.clear(); closeDetail();
    stage = 'shelves';
    return refreshed(() => shelves.reload(), () => books.first());
  });

  const createBook = (name: string, description: string) => {
    const parent = shelf;
    if (!parent) return Promise.resolve(false);
    return create<Book>('book', 'books/create', {shelf_id: parent.id, name, description}, item => {
      book = item; entry = null; archivedEntries = false;
      entries.clear(); closeDetail();
      stage = 'books';
      return refreshed(() => books.reload(), () => entries.first());
    });
  };

  // ---- entry writes -----------------------------------------------------------
  async function written(action: EntryChange['action'], receipt: EntryReceipt) {
    if (action === 'create') created = new Set([...created, receipt.entry_id]);
    const failure = await refreshed(() => entries.reload());
    if (action === 'archive') {
      entry = null;
      show({kind: 'message', title: 'Entry archived', text: 'Its body and history are unavailable until it is restored. Find it under archived entries.'}, 'entries');
    } else {
      try { await openEntry(receipt.entry_id); }
      catch (error) {
        if (app.locked) return;
        show({kind: 'message', title: 'Write confirmed', text: `Entry ${receipt.entry_id} is at revision ${receipt.revision}. Reading the current entry failed: ${message(error)} Select it from the list to try again. Do not repeat the write.`}, 'entries');
        return;
      }
    }
    if (failure) notice(`Write confirmed at revision ${receipt.revision}. The entry list refresh failed: ${failure}`, 'warning');
  }

  async function reloadEntry() {
    if (detail.kind === 'entry') await openEntry(detail.snapshot.entry.id);
  }

  // ---- shelf archive state, archived shelves and purge -------------------------
  function browseArchived() {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      show({kind: 'archived-shelves'}, stage === 'detail' ? detailFrom : 'shelves');
    }, 'archived-shelves');
  }

  function inspectShelf(shelfId: string) {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      show({kind: 'shelf-state', shelfId}, stage === 'detail' ? detailFrom : 'books');
    }, 'shelf-state');
  }

  function archived(intent: AdminIntent, outcome: ArchiveOutcome) {
    show({kind: 'archive-op', intent, outcome}, detailFrom);
    if (outcome.state === 'confirmed') void confirmedArchive(intent, outcome);
  }

  async function confirmedArchive(intent: AdminIntent, outcome: {text: string}) {
    let failure: string;
    if (intent.action === 'shelf') {
      if (shelf?.id === intent.target_id && JSON.parse(intent.body).archived) {
        shelf = null; book = null; entry = null; books.clear(); entries.clear();
      }
      failure = await refreshed(() => shelves.reload());
    } else {
      if (entry?.id === intent.target_id) entry = null;
      failure = await refreshed(() => entries.reload());
    }
    if (failure) notice(`${outcome.text} The list refresh failed: ${failure}`, 'warning');
  }

  async function readAfterArchive(intent: AdminIntent) {
    if (intent.action === 'shelf') { show({kind: 'shelf-state', shelfId: intent.target_id}, detailFrom); return; }
    await entries.reload();
    closeDetail();
    back('entries');
  }

  async function returnToBrowser() {
    closeDetail();
    back(detailFrom);
    await shelves.reload();
  }

  onMount(() => { shelves.first().catch(() => {}); });
</script>

<div class="space-y-6">
  <PageHeader title="Context" description="Shelves, books and entries shared in this workspace.">
    {#snippet meta()}
      {#if !writable}<Badge tone="outline">Read-only</Badge>{/if}
      {#if app.workspace?.archived}<Badge tone="warning">Workspace archived</Badge>{/if}
    {/snippet}
  </PageHeader>

  <nav aria-label="Context location" class="flex min-w-0 flex-wrap items-center gap-1 text-sm text-muted-foreground">
    <button type="button" class="cursor-pointer rounded px-1 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring" onclick={() => back('shelves')}>Shelves</button>
    {#if shelf}<span aria-hidden="true">/</span>
      <button type="button" class="max-w-48 cursor-pointer truncate rounded px-1 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring" onclick={() => back('books')}>{shelf.name}</button>{/if}
    {#if book}<span aria-hidden="true">/</span>
      <button type="button" class="max-w-48 cursor-pointer truncate rounded px-1 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring" onclick={() => back('entries')}>{book.name}</button>{/if}
    {#if entry}<span aria-hidden="true">/</span><span class="max-w-48 truncate px-1 text-foreground" aria-current="location">{entry.name}</span>{/if}
  </nav>

  <div class="grid gap-4 md:grid-cols-3">
    <ListPane id="shelves" kind="shelf" title="Shelves" pager={shelves} selectedId={shelf?.id ?? null} pinned={shelf}
      label={item => item.name} detail={item => item.description} badge={isNew} onselect={selectShelf}
      emptyText="No active shelves on this page." class={paneClass('shelves')}
      onretry={() => app.run(() => shelves.first(), 'shelves-retry')}>
      {#snippet actions()}
        <Button variant="ghost" size="sm" busy={app.working('archived-shelves')} onclick={browseArchived}><Archive /> Archived</Button>
      {/snippet}
      {#snippet footer()}
        {#if writable}
          <CreateNodeForm kind="shelf" draftKey={`context.shelf.${workspaceId}`} oncreate={createShelf} />
        {:else}<p class="text-xs text-muted-foreground">Read-only with this key or workspace state.</p>{/if}
      {/snippet}
    </ListPane>

    {#if shelf}
      <ListPane id="books" kind="book" title="Books" description={shelf.description || shelf.name} pager={books} selectedId={book?.id ?? null} pinned={book}
        label={item => item.name} detail={item => item.description} badge={isNew} onselect={selectBook}
        emptyText="No books on this page." class={paneClass('books')} back={{label: 'Shelves', onclick: () => back('shelves')}}
        onretry={() => app.run(() => books.first(), 'books-retry')}>
        {#snippet actions()}
          {@const shelfId = shelf?.id ?? ''}
          <Button variant="ghost" size="sm" busy={app.working('shelf-state')} onclick={() => inspectShelf(shelfId)}>Archive state</Button>
        {/snippet}
        {#snippet footer()}
          {#if writable}
            {#key shelf?.id}<CreateNodeForm kind="book" draftKey={`context.book.${workspaceId}.${shelf?.id}`} oncreate={createBook} />{/key}
          {/if}
        {/snippet}
      </ListPane>
    {:else}
      <section aria-labelledby="books-title" class="hidden min-w-0 flex-col rounded-xl border border-dashed p-4 md:flex">
        <h2 id="books-title" tabindex="-1" class="text-[15px] font-semibold outline-none">Books</h2>
        <p class="mt-2 text-sm text-muted-foreground">Select a shelf to list its books.</p>
      </section>
    {/if}

    {#if book}
      <ListPane id="entries" kind="entry" title={archivedEntries ? 'Archived entries' : 'Entries'} description={book.description || book.name}
        pager={entries} selectedId={entry?.id ?? null} pinned={pinnedEntry}
        label={item => item.name} detail={item => `Revision ${item.revision}`} badge={isNew} onselect={selectEntry}
        emptyText={archivedEntries ? 'No archived entries on this page.' : 'No active entries on this page.'}
        class={paneClass('entries')} back={{label: 'Books', onclick: () => back('books')}}
        onretry={() => app.run(() => entries.first(), 'entries-retry')}>
        {#snippet actions()}
          <Button variant={archivedEntries ? 'secondary' : 'ghost'} size="sm" aria-pressed={archivedEntries}
            busy={app.working('toggle-archived')} onclick={toggleArchived}><Archive /> Archived</Button>
        {/snippet}
        {#snippet footer()}
          {#if writable && !archivedEntries}
            <button type="button" aria-disabled={app.busy || undefined} onclick={() => { if (!app.busy) newEntry(); }}
              class="flex cursor-pointer items-center gap-1.5 rounded-md text-sm font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring aria-disabled:opacity-60">
              <Plus class="size-4 text-muted-foreground" aria-hidden="true" /> New entry
            </button>
          {:else if archivedEntries}
            <p class="text-xs text-muted-foreground">Archived entries show summary metadata only.</p>
          {/if}
        {/snippet}
      </ListPane>
    {:else}
      <section aria-labelledby="entries-title" class="hidden min-w-0 flex-col rounded-xl border border-dashed p-4 md:flex">
        <h2 id="entries-title" tabindex="-1" class="text-[15px] font-semibold outline-none">Entries</h2>
        <p class="mt-2 text-sm text-muted-foreground">Select a book to list its entries.</p>
      </section>
    {/if}
  </div>

  <section aria-label="Details" class={['rounded-xl border bg-card p-5 text-card-foreground shadow-xs', stage === 'detail' ? 'block' : 'hidden md:block']}>
    <Button variant="ghost" size="sm" class="-mt-1 mb-3 -ml-2 md:hidden" onclick={() => back(detailFrom)}>
      <ChevronLeft /> {{shelves: 'Shelves', books: 'Books', entries: 'Entries', detail: 'Back'}[detailFrom]}
    </Button>
    {#key detailSerial}
      {#if detail.kind === 'loading'}
        <Delayed><EntrySkeleton /></Delayed>
      {:else if detail.kind === 'entry'}
        <EntryEditor snapshot={detail.snapshot} bookId={detail.snapshot.entry.book_id} onwritten={written} onreload={reloadEntry} />
      {:else if detail.kind === 'new-entry'}
        <EntryEditor snapshot={null} bookId={detail.bookId} onwritten={written} onreload={reloadEntry} />
      {:else if detail.kind === 'archived-entry'}
        <ArchivedEntry entry={detail.entry} onwritten={written} onarchive={archived} />
      {:else if detail.kind === 'archived-shelves'}
        <ArchivedShelves oninspect={inspectShelf} onreturn={() => void app.run(returnToBrowser, 'archive-return')} />
      {:else if detail.kind === 'shelf-state'}
        <ShelfState shelfId={detail.shelfId} onarchive={archived} onbrowse={browseArchived} />
      {:else if detail.kind === 'archive-op'}
        {@const intent = detail.intent}
        <ArchiveOperation {intent} outcome={detail.outcome} onconfirmed={outcome => confirmedArchive(intent, outcome)}
          onread={() => readAfterArchive(intent)} onreturn={returnToBrowser} />
      {:else if detail.kind === 'message'}
        <div class="space-y-2">
          <h2 id="context-detail-title" tabindex="-1" class="text-lg font-semibold tracking-tight outline-none">{detail.title}</h2>
          <p class="text-sm text-muted-foreground" role="status">{detail.text}</p>
        </div>
      {:else}
        <Empty icon={Library} title="Nothing selected" description="Choose a shelf, book and entry to read context or inspect its revisions." />
      {/if}
    {/key}
  </section>
</div>
