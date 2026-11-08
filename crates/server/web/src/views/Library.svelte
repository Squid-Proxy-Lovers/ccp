<script lang="ts">
  // Library explorer: shelves → books → entries, then the selected entry's
  // full context. The server returns the whole index in one read, so entries
  // are indexed once and every column renders a bounded window.
  import {onMount, tick} from 'svelte';
  import {RefreshCw, Search, BookOpen} from '@lucide/svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Alert from '../lib/ui/Alert.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Skeleton from '../lib/ui/Skeleton.svelte';
  import Delayed from '../lib/ui/Delayed.svelte';
  import Pane from './library/Pane.svelte';
  import EntryReader from './library/EntryReader.svelte';
  import {app} from '../lib/app.svelte';
  import {enc} from '../lib/api';
  import {whole} from './overview/types';
  import type {Row} from './library/row';
  import type {EntryTarget, Library, LibraryEntry} from '../lib/types';

  // svelte-ignore state_referenced_locally
  const name = app.session as string;
  let library = $state.raw<Library | null>(null);
  let failed = $state('');
  let loading = $state(false);
  let shelf = $state<string | null>(null);
  let book = $state<string | null>(null);
  let entry = $state.raw<EntryTarget | null>(null);
  let query = $state('');
  let reader: HTMLElement | undefined = $state();

  const SEP = '\u0000';
  const bookKey = (shelfName: string, bookName: string) => `${shelfName}${SEP}${bookName}`;

  const index = $derived.by(() => {
    const byBook = new Map<string, LibraryEntry[]>();
    for (const item of library?.entries ?? []) {
      const key = bookKey(item.shelf_name, item.book_name);
      const list = byBook.get(key);
      if (list) list.push(item); else byBook.set(key, [item]);
    }
    return byBook;
  });

  const shelfRows = $derived<Row[]>((library?.shelves ?? []).map(item => ({
    id: item.shelf_name, label: item.shelf_name, detail: item.description || `${whole.format(item.book_count)} books`, count: item.entry_count,
  })));
  const bookRows = $derived<Row[]>((library?.books ?? []).filter(item => item.shelf_name === shelf).map(item => ({
    id: bookKey(item.shelf_name, item.book_name), label: item.book_name, detail: item.description || undefined, count: item.entry_count,
  })));

  const searching = $derived(query.trim().length > 0);
  const entryRows = $derived.by<Row[]>(() => {
    const toRow = (item: LibraryEntry, withPath: boolean): Row => ({
      id: `${bookKey(item.shelf_name, item.book_name)}${SEP}${item.name}`,
      label: item.name,
      detail: withPath ? `${item.shelf_name}/${item.book_name}${item.description ? ` · ${item.description}` : ''}` : item.description || undefined,
      badge: item.labels[0],
    });
    if (searching) {
      const needle = query.trim().toLowerCase();
      const scope = shelf && book ? (index.get(book) ?? []) : (library?.entries ?? []);
      const hits: Row[] = [];
      for (const item of scope) {
        if (item.name.toLowerCase().includes(needle) || item.description.toLowerCase().includes(needle) || item.labels.some(label => label.toLowerCase().includes(needle))) {
          hits.push(toRow(item, !(shelf && book)));
          if (hits.length >= 2000) break;
        }
      }
      return hits;
    }
    return book ? (index.get(book) ?? []).map(item => toRow(item, false)) : [];
  });

  const selectedEntryId = $derived(entry ? `${bookKey(entry.shelf, entry.book)}${SEP}${entry.entry}` : null);
  const currentBook = $derived(library?.books.find(item => bookKey(item.shelf_name, item.book_name) === book) ?? null);

  async function load() {
    const generation = app.generation;
    loading = true;
    try {
      const next = await app.request<Library>(`/v1/admin/sessions/${enc(name)}/library`);
      if (generation !== app.generation) return;
      library = next;
      failed = '';
      const wanted = app.target;
      if (wanted) {
        app.target = null;
        shelf = wanted.shelf;
        book = bookKey(wanted.shelf, wanted.book);
        await show(wanted);
      } else if (!shelf && next.shelves.length === 1) {
        shelf = next.shelves[0]?.shelf_name ?? null;
      }
    } catch (error) {
      if (!app.locked && generation === app.generation) failed = (error as Error).message;
    } finally { loading = false; }
  }
  onMount(() => { void load(); });

  async function show(target: EntryTarget) {
    entry = target;
    await tick();
    reader?.scrollIntoView({behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth', block: 'start'});
  }
  function pickShelf(row: Row) { shelf = row.id; book = null; entry = null; }
  function pickBook(row: Row) { book = row.id; entry = null; }
  function pickEntry(row: Row) {
    const [shelfName = '', bookName = '', entryName = ''] = row.id.split(SEP);
    void show({shelf: shelfName, book: bookName, entry: entryName});
  }
</script>

<div class="space-y-6">
  <PageHeader title="Library" description={`Shelves, books and entries shared by agents in ${name}. Read-only here; agents write through CCP.`}>
    {#snippet meta()}
      {#if library}
        <span>{whole.format(library.shelves.length)} shelves</span>
        <span>{whole.format(library.books.length)} books</span>
        <span>{whole.format(library.entries.length)} entries</span>
      {/if}
    {/snippet}
    {#snippet actions()}
      <Button variant="outline" size="sm" busy={loading && library !== null} onclick={() => void load()}><RefreshCw /> Reload</Button>
    {/snippet}
  </PageHeader>

  {#if failed}
    <Alert tone="destructive" title="Couldn't load the library" live><p>{failed}</p></Alert>
  {/if}

  {#if !library && !failed}
    <Delayed>
      <div class="grid gap-4 md:grid-cols-3" role="status" aria-label="Loading library">
        {#each [0, 1, 2] as column (column)}
          <div class="space-y-2 rounded-xl border bg-card p-4">
            <Skeleton class="h-4 w-24" />
            {#each [0, 1, 2, 3, 4] as row (row)}<Skeleton class="h-8 w-full opacity-70" />{/each}
          </div>
        {/each}
      </div>
    </Delayed>
  {:else if library && !library.entries.length && !library.shelves.length}
    <Empty icon={BookOpen} title="This library is empty" description="Shelves, books and entries appear once agents add them with add_shelf, add_book and add_entry." />
  {:else if library}
    <div class="relative">
      <Search class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
      <Input type="search" bind:value={query} aria-label="Search entries"
        placeholder={book ? `Search entries in ${currentBook?.book_name ?? 'this book'}` : `Search all ${whole.format(library.entries.length)} entries by name, description or label`} class="pl-9" />
    </div>

    <div class="grid gap-4 md:grid-cols-3">
      <Pane id="shelves" title="Shelves" rows={shelfRows} selected={shelf} onselect={pickShelf} empty="No shelves." />
      <Pane id="books" title="Books" description={shelf ?? undefined} rows={bookRows} selected={book} onselect={pickBook}
        empty={shelf ? 'No books on this shelf.' : 'Choose a shelf.'} />
      <Pane id="entries" title={searching ? 'Search results' : 'Entries'}
        description={searching ? (entryRows.length >= 2000 ? 'First 2,000 matches' : undefined) : currentBook?.book_name}
        rows={entryRows} selected={selectedEntryId} onselect={pickEntry}
        empty={searching ? 'No matching entries.' : book ? 'No entries in this book.' : 'Choose a book, or search.'} />
    </div>

    {#if entry}
      <div bind:this={reader} class="scroll-mt-20">
        {#key selectedEntryId}
          <EntryReader session={name} target={entry} onclose={() => (entry = null)} />
        {/key}
      </div>
    {/if}
  {/if}
</div>
