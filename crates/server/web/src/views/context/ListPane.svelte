<script lang="ts" generics="T extends {id: string}">
  // One column of the context browser: a paged list of selectable rows.
  import {tick, type Snippet} from 'svelte';
  import {ChevronLeft, ChevronRight, RotateCcw} from '@lucide/svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import {app} from '../../lib/app.svelte';
  import type {Pager} from './pager.svelte';

  let {id, kind, title, description, pager, selectedId, pinned = null, label, detail, badge, onselect, emptyText,
    onretry, back, actions, footer, class: className = 'flex'}: {
    id: string; kind: string; title: string; description?: string; pager: Pager<T>; selectedId: string | null;
    pinned?: T | null; label: (item: T) => string; detail?: (item: T) => string; badge?: (item: T) => string | undefined;
    onselect: (item: T) => void; emptyText: string; onretry: () => void;
    back?: {label: string; onclick: () => void}; actions?: Snippet; footer?: Snippet; class?: string;
  } = $props();

  const rows = $derived(pinned && !pager.items.some(item => item.id === pinned.id) ? [pinned, ...pager.items] : pager.items);
  const offPage = (item: T) => item === pinned && !pager.items.some(other => other.id === item.id);
  let list: HTMLElement | undefined = $state();

  // A paging button can disappear (last or first page); keep focus inside the list.
  async function turn(action: () => Promise<void>, key: string) {
    await app.run(action, key);
    await tick();
    if (document.activeElement === document.body || !document.activeElement?.isConnected) {
      (list?.querySelector<HTMLElement>('[data-row]') ?? document.getElementById(`${id}-title`))?.focus();
    }
  }
</script>

<section aria-labelledby={`${id}-title`} class={['min-h-0 min-w-0 flex-col rounded-xl border bg-card text-card-foreground shadow-xs', className]}>
  <div class="flex flex-col gap-2 border-b px-4 py-3">
    {#if back}
      <Button variant="ghost" size="sm" class="-ml-2 w-fit md:hidden" onclick={back.onclick}><ChevronLeft /> {back.label}</Button>
    {/if}
    <div class="flex items-start justify-between gap-2">
      <div class="min-w-0 space-y-0.5">
        <h2 id={`${id}-title`} tabindex="-1" class="truncate text-[15px] leading-tight font-semibold tracking-tight outline-none">{title}</h2>
        {#if description}<p class="truncate text-[13px] text-muted-foreground" title={description}>{description}</p>{/if}
      </div>
      {#if actions}<div class="flex shrink-0 items-center gap-1">{@render actions()}</div>{/if}
    </div>
  </div>

  <div bind:this={list} class="min-h-0 flex-1 overflow-y-auto p-2 md:max-h-[26rem]" aria-busy={pager.loading || undefined}>
    {#if pager.error && !pager.loaded}
      <div class="space-y-2 p-2 text-sm" role="alert">
        <p class="text-destructive">{pager.error}</p>
        <Button variant="outline" size="sm" busy={app.working(`${id}-retry`)} onclick={onretry}><RotateCcw /> Retry</Button>
      </div>
    {:else if !pager.loaded}
      <div class="space-y-2 p-2">
        <Skeleton class="h-8 w-full" /><Skeleton class="h-8 w-4/5" /><Skeleton class="h-8 w-3/5" />
        <span class="sr-only">Loading {title.toLowerCase()}</span>
      </div>
    {:else if !rows.length}
      <p class="px-2 py-6 text-center text-sm text-muted-foreground">{emptyText}</p>
    {:else}
      <ul class="space-y-0.5" aria-label={title}>
        {#each rows as item (item.id)}
          {@const selected = item.id === selectedId}
          {@const tag = badge?.(item) ?? (offPage(item) ? 'Not on this page' : undefined)}
          <li>
            <button type="button" data-row={`${kind}:${item.id}`} aria-current={selected ? 'true' : undefined}
              onclick={() => onselect(item)}
              class={['flex w-full cursor-pointer items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm outline-none transition-colors focus-visible:ring-[3px] focus-visible:ring-ring/50',
                selected ? 'bg-accent font-medium text-accent-foreground' : 'hover:bg-muted/60']}>
              <span class="min-w-0 flex-1">
                <span class="block truncate" title={label(item)}>{label(item)}</span>
                {#if detail?.(item)}<span class="block truncate text-xs font-normal text-muted-foreground">{detail(item)}</span>{/if}
              </span>
              {#if tag}<Badge tone="outline" class="font-normal">{tag}</Badge>{/if}
              <ChevronRight class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  {#if pager.loaded && (pager.next || pager.after)}
    <nav aria-label={`${title} pages`} class="flex items-center justify-between gap-2 border-t px-4 py-2">
      <span class="text-xs text-muted-foreground tabular">Page {pager.page}</span>
      <div class="flex gap-1">
        {#if pager.after}
          <Button variant="ghost" size="sm" busy={app.working(`${id}-first`)} onclick={() => turn(() => pager.first(), `${id}-first`)}>First page</Button>
        {/if}
        {#if pager.next}
          <Button variant="outline" size="sm" busy={app.working(`${id}-next`)} onclick={() => turn(() => pager.forward(), `${id}-next`)}>Next page</Button>
        {/if}
      </div>
    </nav>
  {/if}

  {#if footer}<div class="border-t px-4 py-3">{@render footer()}</div>{/if}
</section>
