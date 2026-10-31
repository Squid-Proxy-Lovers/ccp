<script lang="ts">
  // Summary metadata for archived shelves only; their books and entries stay hidden.
  import {onMount} from 'svelte';
  import {RotateCcw} from '@lucide/svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import Delayed from '../../lib/ui/Delayed.svelte';
  import {table} from '../../lib/ui/table';
  import {app} from '../../lib/app.svelte';
  import {archivedShelfPage} from '../../lib/model.js';
  import {contentPath, type ArchivedShelf, type ContentPage} from './content';
  import {Pager} from './pager.svelte';

  let {oninspect, onreturn}: {oninspect: (shelfId: string) => void; onreturn: () => void} = $props();

  const workspaceId = app.workspace?.id;
  const pager = new Pager<ArchivedShelf>(async (after, limit) =>
    archivedShelfPage(await app.request(contentPath('shelves/archived'), {workspace_id: workspaceId, after, limit}), workspaceId) as ContentPage<ArchivedShelf>);

  onMount(() => { pager.first().catch(() => {}); });
</script>

<article class="space-y-4" aria-labelledby="context-detail-title">
  <div class="space-y-1">
    <h2 id="context-detail-title" tabindex="-1" class="text-lg leading-tight font-semibold tracking-tight outline-none">Archived shelves</h2>
    <p class="text-sm text-muted-foreground">Summary metadata only. Books, entry bodies and history stay hidden until the shelf is restored. Nothing is purged here.</p>
  </div>
  <div aria-live="polite" aria-busy={pager.loading || undefined}>
    {#if pager.error && !pager.loaded}
      <div class="space-y-2" role="alert">
        <p class="text-sm text-destructive">{pager.error}</p>
        <Button variant="outline" size="sm" busy={app.working('archived-shelves-retry')} onclick={() => app.run(() => pager.first(), 'archived-shelves-retry')}><RotateCcw /> Retry</Button>
      </div>
    {:else if !pager.loaded}
      <span class="sr-only">Loading archived shelves</span><Delayed><div class="divide-y rounded-lg border" aria-hidden="true">{#each [0, 1, 2] as row (row)}<div class="flex items-center justify-between gap-4 px-3 py-3"><div class="space-y-1.5"><Skeleton class="h-3.5 w-28" /><Skeleton class="h-3 w-40 opacity-70" /></div><Skeleton class="h-7 w-16" /></div>{/each}</div></Delayed>
    {:else if !pager.items.length}
      <p class="rounded-lg border border-dashed px-4 py-8 text-center text-sm text-muted-foreground">No archived shelves on this page.</p>
    {:else}
      <div class={['rounded-lg border', table.wrap]}>
        <table class={table.table}>
          <thead class={table.head}><tr>
            <th scope="col" class={table.headCell}>Name</th>
            <th scope="col" class={table.headCell}>Lifecycle revision</th>
            <th scope="col" class={table.headCell}><span class="sr-only">Actions</span></th>
          </tr></thead>
          <tbody class={table.body}>
            {#each pager.items as shelf (shelf.id)}
              <tr class={table.row}>
                <td class={table.cell}><span class="font-medium break-words">{shelf.name}</span><span class="block font-mono text-xs break-all text-muted-foreground">{shelf.id}</span></td>
                <td class={[table.cell, 'tabular']}>{shelf.revision}</td>
                <td class={[table.cell, 'text-right']}>
                  <Button variant="outline" size="sm" aria-label={`Inspect archived shelf ${shelf.name}`} onclick={() => oninspect(shelf.id)}>Inspect</Button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>
  <div class="flex flex-wrap items-center gap-2">
    <Button variant="outline" onclick={onreturn}>Return to context browser</Button>
    {#if pager.loaded && pager.after}
      <Button variant="ghost" class="ml-auto" busy={app.working('archived-shelves-first')} onclick={() => app.run(() => pager.first(), 'archived-shelves-first')}>First page</Button>
    {/if}
    {#if pager.loaded && pager.next}
      <Button variant="outline" class={pager.after ? '' : 'ml-auto'} busy={app.working('archived-shelves-next')} onclick={() => app.run(() => pager.forward(), 'archived-shelves-next')}>Next page</Button>
    {/if}
  </div>
</article>
