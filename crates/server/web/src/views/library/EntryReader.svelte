<script lang="ts">
  // Read-only view of one entry's full context.
  import {onMount} from 'svelte';
  import {Copy, Braces, AlignLeft, RefreshCw, X} from '@lucide/svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import Delayed from '../../lib/ui/Delayed.svelte';
  import {app} from '../../lib/app.svelte';
  import {enc, prettyJson, utf8Bytes} from '../../lib/api';
  import {copyText} from '../../lib/notify.svelte';
  import type {EntryRecord, EntryTarget} from '../../lib/types';

  let {session, target, onclose}: {session: string; target: EntryTarget; onclose: () => void} = $props();

  let record = $state.raw<EntryRecord | null>(null);
  let failed = $state('');
  let formatted = $state(true);
  let loading = $state(false);

  const body = $derived(record ? prettyJson(record.context) : null);
  const text = $derived(record ? (formatted && body?.json ? body.pretty : record.context) : '');
  const path = $derived(`${target.shelf}/${target.book}/${target.entry}`);

  async function load() {
    loading = true;
    try {
      record = await app.request<EntryRecord>(`/v1/admin/sessions/${enc(session)}/entries/${enc(target.shelf)}/${enc(target.book)}/${enc(target.entry)}`);
      failed = '';
    } catch (error) {
      if (!app.locked) failed = (error as Error).message;
    } finally { loading = false; }
  }
  onMount(() => { void load(); });
</script>

<Card class="min-w-0">
  <CardHeader title={target.entry} description={record?.description || path}>
    {#snippet action()}
      <div class="flex items-center gap-1">
        <Button variant="ghost" size="icon-sm" aria-label="Reload entry" busy={loading && record !== null} onclick={() => void load()}><RefreshCw /></Button>
        <Button variant="ghost" size="icon-sm" aria-label="Close entry" onclick={onclose}><X /></Button>
      </div>
    {/snippet}
  </CardHeader>
  <CardContent class="space-y-4">
    <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
      <button type="button" onclick={() => copyText(path)} title="Copy path"
        class="cursor-pointer rounded-sm font-mono outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">{path}</button>
      {#if record}
        {#each record.labels as label (label)}<Badge tone="outline">{label}</Badge>{/each}
        <span>· {utf8Bytes(record.context).toLocaleString()} bytes · {record.context.split('\n').length.toLocaleString()} lines</span>
      {/if}
    </div>
    {#if failed}
      <p class="text-sm text-destructive" role="alert">{failed}</p>
    {:else if !record}
      <Delayed><div class="space-y-2" role="status" aria-label="Loading entry"><Skeleton class="h-4 w-2/3" /><Skeleton class="h-4 w-1/2" /><Skeleton class="h-40 w-full" /></div></Delayed>
    {:else}
      <pre class="max-h-[36rem] overflow-auto rounded-md border bg-muted/30 p-4 font-mono text-[12.5px] leading-relaxed break-words whitespace-pre-wrap">{text || '(Empty entry)'}</pre>
      <div class="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" onclick={() => copyText(record?.context ?? '')}><Copy /> Copy content</Button>
        {#if body?.json}
          <Button variant="ghost" size="sm" aria-pressed={formatted} onclick={() => (formatted = !formatted)}>
            {#if formatted}<AlignLeft /> Show raw{:else}<Braces /> Format JSON{/if}
          </Button>
        {/if}
      </div>
    {/if}
  </CardContent>
</Card>
