<script lang="ts">
  import {Users} from '@lucide/svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import Empty from '../../lib/ui/Empty.svelte';
  import {css} from '../../lib/ui/css';
  import {relative, stamp} from '../../lib/notify.svelte';
  import {whole, type Writer} from './types';

  let {writers, range}: {writers: Writer[]; range: number} = $props();
  const peak = $derived(Math.max(1, ...writers.map(writer => writer.writes)));
</script>

<Card class="min-w-0">
  <CardHeader title="Top writers" description={`Most entry writes, last ${range} days`} />
  <CardContent>
    {#if writers.length}
      <ol class="grid gap-4">
        {#each writers as writer (writer.client_id ?? 'admin')}
          <li class="grid gap-1.5">
            <div class="flex items-baseline justify-between gap-3 text-sm">
              <span class="min-w-0 truncate font-medium" title={writer.client_id ?? 'Administrative writes from the web panel or admin keys'}>{writer.label}</span>
              <span class="tabular shrink-0 font-medium">{whole.format(writer.writes)}</span>
            </div>
            <div class="h-1.5 overflow-hidden rounded-full bg-muted" aria-hidden="true">
              <div class="h-full rounded-full bg-foreground" use:css={{width: `${(writer.writes / peak) * 100}%`}}></div>
            </div>
            <p class="text-xs text-muted-foreground">
              Last write <time datetime={new Date(writer.last_write_at * 1000).toISOString()} title={stamp(writer.last_write_at)}>{relative(writer.last_write_at)}</time>
            </p>
          </li>
        {/each}
      </ol>
    {:else}
      <Empty icon={Users} title="No writers yet" description="Clients that create or change entries in this period appear here." />
    {/if}
  </CardContent>
</Card>
