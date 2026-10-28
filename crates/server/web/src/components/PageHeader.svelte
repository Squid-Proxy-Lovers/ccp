<script lang="ts">
  // Every view's title. Receives focus after navigation so keyboard and
  // screen-reader users land at the start of the new view.
  import type {Snippet} from 'svelte';
  let {title, description, actions, meta}: {title: string; description?: string; actions?: Snippet; meta?: Snippet} = $props();
</script>
<div class="flex flex-wrap items-end justify-between gap-4">
  <div class="min-w-0 space-y-1.5">
    <h1 id="page-title" tabindex="-1" class="text-2xl font-semibold tracking-tight outline-none">{title}</h1>
    {#if description}<p class="max-w-2xl text-sm text-muted-foreground">{description}</p>{/if}
    {#if meta}<div class="flex flex-wrap items-center gap-2 pt-1">{@render meta()}</div>{/if}
  </div>
  {#if actions}<div class="flex flex-wrap items-center gap-2">{@render actions()}</div>{/if}
</div>
