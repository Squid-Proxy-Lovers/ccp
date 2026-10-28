<script lang="ts">
  import {TriangleAlert, Info, CircleCheck, ChevronRight} from '@lucide/svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import type {View} from '../../lib/types';
  import type {AttentionItem as Item} from './types';

  let {items, onopen}: {items: Item[]; onopen: (view: View) => void} = $props();
</script>

<Card class="min-w-0">
  <CardHeader title="Needs attention" description="Access and lifecycle items worth a look" />
  <CardContent>
    {#if items.length}
      <ul class="-mx-2 grid gap-0.5">
        {#each items as item (item.title)}
          <li>
            <button type="button" onclick={() => onopen(item.view)}
              class="group flex w-full cursor-pointer items-start gap-3 rounded-lg px-2 py-2.5 text-left outline-none hover:bg-muted/60 focus-visible:ring-[3px] focus-visible:ring-ring/50">
              {#if item.tone === 'warning'}
                <TriangleAlert class="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
              {:else}
                <Info class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
              {/if}
              <span class="min-w-0 flex-1">
                <span class="block text-sm font-medium">{item.title}</span>
                <span class="block text-xs text-muted-foreground">{item.detail}</span>
              </span>
              <ChevronRight class="mt-0.5 size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" aria-hidden="true" />
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <div class="flex items-center gap-3 rounded-lg border border-dashed px-4 py-6 text-sm text-muted-foreground">
        <CircleCheck class="size-4 text-success" aria-hidden="true" />
        Nothing needs attention right now.
      </div>
    {/if}
  </CardContent>
</Card>
