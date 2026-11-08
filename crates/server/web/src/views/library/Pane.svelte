<script lang="ts">
  // One column of the library explorer. Renders at most `shown` rows so very
  // large books stay responsive; "Show more" extends the window.
  import type {Snippet} from 'svelte';
  import {ChevronRight} from '@lucide/svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import {whole} from '../overview/types';
  import type {Row} from './row';

  let {id, title, description, rows, selected, onselect, empty, toolbar, step = 200}: {
    id: string; title: string; description?: string; rows: Row[]; selected: string | null; onselect: (row: Row) => void;
    empty: string; toolbar?: Snippet; step?: number;
  } = $props();

  let extra = $state(0);
  // A new row list (another shelf, book or search) starts from the first window again.
  $effect(() => { void rows; extra = 0; });
  const shown = $derived(step + extra);
  const visible = $derived(rows.slice(0, shown));
</script>

<section aria-labelledby={`${id}-title`} class="flex min-h-0 min-w-0 flex-col rounded-xl border bg-card text-card-foreground shadow-xs">
  <div class="flex flex-col gap-2 border-b px-4 py-3">
    <div class="flex items-baseline justify-between gap-2">
      <h2 id={`${id}-title`} tabindex="-1" class="truncate text-[15px] leading-tight font-semibold tracking-tight outline-none">{title}</h2>
      <span class="tabular text-xs text-muted-foreground">{whole.format(rows.length)}</span>
    </div>
    {#if description}<p class="truncate text-[13px] text-muted-foreground" title={description}>{description}</p>{/if}
    {#if toolbar}{@render toolbar()}{/if}
  </div>
  <div class="min-h-0 flex-1 overflow-y-auto p-2 md:h-[26rem] md:flex-none">
    {#if !rows.length}
      <p class="px-2 py-6 text-center text-sm text-muted-foreground">{empty}</p>
    {:else}
      <ul class="space-y-0.5" aria-label={title}>
        {#each visible as row (row.id)}
          {@const current = row.id === selected}
          <li>
            <button type="button" data-row={row.id} aria-current={current ? 'true' : undefined} onclick={() => onselect(row)}
              class={['flex w-full cursor-pointer items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring',
                current ? 'bg-accent font-medium text-accent-foreground' : 'hover:bg-muted/60']}>
              <span class="min-w-0 flex-1">
                <span class="block truncate" title={row.label}>{row.label}</span>
                {#if row.detail}<span class="block truncate text-xs font-normal text-muted-foreground" title={row.detail}>{row.detail}</span>{/if}
              </span>
              {#if row.badge}<Badge tone="outline" class="font-normal">{row.badge}</Badge>{/if}
              {#if row.count !== undefined}<span class="tabular text-xs text-muted-foreground">{whole.format(row.count)}</span>{/if}
              <ChevronRight class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            </button>
          </li>
        {/each}
      </ul>
      {#if rows.length > shown}
        <div class="flex justify-center p-2">
          <Button variant="outline" size="sm" onclick={() => (extra += step)}>Show {whole.format(Math.min(step, rows.length - shown))} more of {whole.format(rows.length - shown)}</Button>
        </div>
      {/if}
    {/if}
  </div>
</section>
