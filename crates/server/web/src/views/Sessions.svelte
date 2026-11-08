<script lang="ts">
  // Every session on the server: open one, create one, or delete one. Deletion
  // permanently removes the session and its library, so it needs the typed name.
  import {Plus, Trash2, ArrowRight, Layers, Search} from '@lucide/svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import WorkspaceAvatar from '../components/WorkspaceAvatar.svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Card from '../lib/ui/Card.svelte';
  import CardHeader from '../lib/ui/CardHeader.svelte';
  import CardContent from '../lib/ui/CardContent.svelte';
  import Dialog from '../lib/ui/Dialog.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import {table} from '../lib/ui/table';
  import {app} from '../lib/app.svelte';
  import {whole} from './overview/types';
  import type {SessionStats} from '../lib/types';

  let name = $state('');
  let query = $state('');
  let target = $state.raw<SessionStats | null>(null);
  let typed = $state('');
  let deleting = $state(false);

  // The server accepts any non-empty name; a slash would break the admin URLs.
  const nameError = $derived(name.includes('/') ? 'Session names cannot contain a slash.' : name.trim().length > 128 ? 'Use at most 128 characters.' : '');
  const exists = $derived(app.sessions.some(item => item.session.session_name === name.trim()));
  const agentsBySession = $derived(app.agents.reduce<Record<string, number>>((acc, agent) => ({...acc, [agent.session_name]: (acc[agent.session_name] ?? 0) + 1}), {}));
  const rows = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    return app.sessions.filter(item => !needle || [item.session.session_name, item.session.purpose, item.session.description, item.session.owner, ...item.session.labels]
      .some(field => field.toLowerCase().includes(needle)));
  });
  const totals = $derived(app.sessions.reduce((acc, item) => ({entries: acc.entries + item.entries, books: acc.books + item.books}), {entries: 0, books: 0}));

  function create(event: SubmitEvent) {
    event.preventDefault();
    const value = name.trim();
    if (!value || nameError || exists) return;
    void app.run(async () => {
      await app.createSession(value);
      name = '';
    }, 'create-session');
  }

  function askDelete(item: SessionStats) { target = item; typed = ''; deleting = true; }
  function confirmDelete() {
    const session = target;
    if (!session || typed !== session.session.session_name) return;
    void app.run(async () => {
      await app.deleteSession(session.session.session_name);
      deleting = false;
      target = null;
    }, 'delete-session');
  }
  function open(item: SessionStats) { void app.run(() => app.openSession(item.session.session_name, 'overview'), `open:${item.session.session_name}`); }
</script>

<div class="space-y-6">
  <PageHeader title="Sessions" description="Each session is a separate shared library with its own master instructions. Agents subscribe to sessions by name.">
    {#snippet meta()}
      <span>{whole.format(app.sessions.length)} sessions</span>
      <span>{whole.format(totals.books)} books</span>
      <span>{whole.format(totals.entries)} entries</span>
    {/snippet}
  </PageHeader>

  <div class="grid items-start gap-6 xl:grid-cols-[minmax(0,1fr)_20rem]">
    <Card class="min-w-0">
      <CardHeader title="All sessions" description="Open a session to browse its library, activity and agents.">
        {#snippet action()}
          {#if app.sessions.length > 6}
            <div class="relative w-56">
              <Search class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
              <Input type="search" bind:value={query} placeholder="Filter sessions" aria-label="Filter sessions" class="h-8 pl-8" />
            </div>
          {/if}
        {/snippet}
      </CardHeader>
      {#if rows.length}
        <div class={table.wrap}>
          <table class={table.table}>
            <thead class={table.head}>
              <tr>
                <th scope="col" class={table.headCell}>Session</th>
                <th scope="col" class={[table.headCell, 'hidden text-right md:table-cell']}>Shelves</th>
                <th scope="col" class={[table.headCell, 'hidden text-right md:table-cell']}>Books</th>
                <th scope="col" class={[table.headCell, 'text-right']}>Entries</th>
                <th scope="col" class={[table.headCell, 'hidden text-right sm:table-cell']}>Agents</th>
                <th scope="col" class={table.headCell}><span class="sr-only">Actions</span></th>
              </tr>
            </thead>
            <tbody class={table.body}>
              {#each rows as item (item.session.session_id)}
                <tr class={[table.row, item.session.session_name === app.session && 'bg-muted/40']}>
                  <td class={[table.cell, 'min-w-40']}>
                    <button type="button" onclick={() => open(item)} class="group flex min-w-0 cursor-pointer items-center gap-3 rounded-md text-left outline-none focus-visible:ring-2 focus-visible:ring-ring">
                      <WorkspaceAvatar id={item.session.session_name} class="size-8" />
                      <span class="grid min-w-0">
                        <span class="flex items-center gap-2 truncate font-medium group-hover:underline">
                          {item.session.session_name}
                          {#if !item.is_active}<Badge>Inactive</Badge>{/if}
                          {#if item.session.visibility !== 'public'}<Badge tone="outline">{item.session.visibility}</Badge>{/if}
                        </span>
                        <span class="max-w-md truncate text-xs text-muted-foreground">{item.session.purpose || item.session.description || '—'}</span>
                      </span>
                    </button>
                  </td>
                  <td class={[table.cell, 'tabular hidden text-right md:table-cell']}>{whole.format(item.shelves)}</td>
                  <td class={[table.cell, 'tabular hidden text-right md:table-cell']}>{whole.format(item.books)}</td>
                  <td class={[table.cell, 'tabular text-right']}>{whole.format(item.entries)}</td>
                  <td class={[table.cell, 'tabular hidden text-right sm:table-cell']}>{agentsBySession[item.session.session_name] ?? 0}</td>
                  <td class={[table.cell, 'text-right whitespace-nowrap']}>
                    <Button variant="ghost" size="icon-sm" aria-label={`Delete ${item.session.session_name}`} onclick={() => askDelete(item)} class="text-muted-foreground hover:text-destructive"><Trash2 /></Button>
                    <Button variant="ghost" size="icon-sm" aria-label={`Open ${item.session.session_name}`} busy={app.working(`open:${item.session.session_name}`)} onclick={() => open(item)}><ArrowRight /></Button>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else}
        <CardContent>
          <Empty icon={Layers} title={app.sessions.length ? 'No matching sessions' : 'No sessions yet'}
            description={app.sessions.length ? 'Try a different filter.' : 'Create a session, then point agents at it with subscribe.'} />
        </CardContent>
      {/if}
    </Card>

    <Card>
      <CardHeader title="New session" description="Agents can subscribe to it as soon as it exists." />
      <CardContent>
        <form class="grid gap-3" onsubmit={create}>
          <Field id="new-session" label="Session name" error={nameError || (exists ? 'A session with this name already exists.' : undefined)}>
            <Input id="new-session" bind:value={name} placeholder="team-research" autocomplete="off" spellcheck={false} class="font-mono"
              aria-invalid={nameError || exists ? true : undefined} />
          </Field>
          <Button type="submit" busy={app.working('create-session')} disabled={!name.trim() || Boolean(nameError) || exists}><Plus /> Create session</Button>
        </form>
      </CardContent>
    </Card>
  </div>
</div>

<Dialog bind:open={deleting} title={`Delete ${target?.session.session_name ?? 'session'}?`}
  description="This permanently deletes the session, every shelf, book and entry in it, and its master instructions. Connected agents lose access. This cannot be undone.">
  {#if target}
    <div class="grid gap-3">
      <p class="text-sm">It holds <strong>{whole.format(target.entries)}</strong> entries in <strong>{whole.format(target.books)}</strong> books.</p>
      <Field id="confirm-delete" label={`Type ${target.session.session_name} to confirm`}>
        <Input id="confirm-delete" bind:value={typed} autocomplete="off" spellcheck={false} class="font-mono" />
      </Field>
    </div>
  {/if}
  {#snippet footer()}
    <Button variant="outline" onclick={() => (deleting = false)}>Cancel</Button>
    <Button variant="destructive" busy={app.working('delete-session')} disabled={typed !== target?.session.session_name} onclick={confirmDelete}><Trash2 /> Delete permanently</Button>
  {/snippet}
</Dialog>
