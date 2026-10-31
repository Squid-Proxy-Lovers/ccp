<script lang="ts">
  // Recorded workspace revisions for one entry, 32 per page.
  import {onMount} from 'svelte';
  import {RotateCcw} from '@lucide/svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import Delayed from '../../lib/ui/Delayed.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Textarea from '../../lib/ui/Textarea.svelte';
  import {app} from '../../lib/app.svelte';
  import {stamp} from '../../lib/notify.svelte';
  import {exactRevision} from '../../lib/model.js';
  import {PAGE_SIZE, actorText, contentPath, focusId, message, type HistoryPage, type Snapshot} from './content';

  let {entryId}: {entryId: string} = $props();
  let page = $state.raw<HistoryPage | null>(null);
  let after = $state<number | null>(null);
  let error = $state('');
  let loading = $state(false);
  let opened = $state.raw<Record<number, Snapshot>>({});

  async function load(cursor: number | null) {
    loading = true;
    error = '';
    try {
      const data = await app.request<HistoryPage>(contentPath('entries/history'), {workspace_id: app.workspace?.id, entry_id: entryId, after: cursor, limit: PAGE_SIZE});
      page = data;
      after = cursor;
      opened = {};
      void focusId('entry-history-title');
    } catch (failure) {
      if (!app.locked) error = message(failure);
    } finally { loading = false; }
  }

  async function read(revision: number) {
    const snapshot = await app.request<Snapshot>(contentPath('entries/get'), {workspace_id: app.workspace?.id, entry_id: entryId, revision: exactRevision(revision)});
    opened = {...opened, [revision]: snapshot};
  }

  onMount(() => { void load(null); });
</script>

<section class="space-y-3 rounded-lg border p-4" aria-labelledby="entry-history-title">
  <h3 id="entry-history-title" tabindex="-1" class="text-sm font-semibold outline-none">Recorded revisions</h3>
  <div aria-live="polite" aria-busy={loading || undefined}>
    {#if error}
      <div class="space-y-2" role="alert">
        <p class="text-sm text-destructive">{error}</p>
        <Button variant="outline" size="sm" onclick={() => load(after)}><RotateCcw /> Retry</Button>
      </div>
    {:else if !page}
      <span class="sr-only">Loading revisions</span><Delayed><div class="divide-y rounded-lg border" aria-hidden="true">{#each [0, 1, 2] as row (row)}<div class="flex items-center justify-between gap-4 px-3 py-3"><div class="space-y-1.5"><Skeleton class="h-3.5 w-28" /><Skeleton class="h-3 w-40 opacity-70" /></div><Skeleton class="h-7 w-16" /></div>{/each}</div></Delayed>
    {:else if !page.items.length}
      <p class="text-sm text-muted-foreground">No recorded revisions on this page.</p>
    {:else}
      <ol class="divide-y">
        {#each page.items as revision (revision.revision)}
          <li class="space-y-2 py-3 first:pt-0 last:pb-0">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <div class="flex min-w-0 flex-wrap items-center gap-2">
                <span class="text-sm font-medium tabular">Revision {revision.revision}</span>
                <Badge tone="neutral">{revision.action}</Badge>
              </div>
              <Button variant="ghost" size="sm" busy={app.working(`revision-${revision.revision}`)}
                onclick={() => app.run(() => read(revision.revision), `revision-${revision.revision}`)}>Read this revision</Button>
            </div>
            <p class="text-xs text-muted-foreground">{stamp(revision.recorded_at)} · <span class="break-all">{actorText(revision)}</span></p>
            {#if revision.agent}<p class="text-xs text-muted-foreground">Agent label (self-reported): {revision.agent}</p>{/if}
            {#if opened[revision.revision]}
              {@const content = opened[revision.revision]!.content}
              <Field id={`revision-${revision.revision}-content`} label={`Revision ${revision.revision} content`}>
                <Textarea id={`revision-${revision.revision}-content`} readonly rows={8} class="font-mono text-xs"
                  value={`Description: ${content.description}\nLabels: ${content.labels.join(', ')}\n\n${content.text}`} />
              </Field>
            {/if}
          </li>
        {/each}
      </ol>
    {/if}
  </div>
  {#if page && (page.next_cursor !== null || after !== null)}
    <div class="flex justify-end gap-2">
      {#if after !== null}<Button variant="ghost" size="sm" onclick={() => app.run(() => load(null), 'history-first')}>First revisions</Button>{/if}
      {#if page.next_cursor !== null}
        {@const cursor = page.next_cursor}
        <Button variant="outline" size="sm" onclick={() => app.run(() => load(exactRevision(cursor)), 'history-next')}>Next revisions</Button>
      {/if}
    </div>
  {/if}
</section>
