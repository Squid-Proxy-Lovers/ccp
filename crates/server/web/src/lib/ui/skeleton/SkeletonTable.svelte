<script lang="ts">
  // Mirrors the shared table layout (`table.ts`): header row, then body rows.
  import Skeleton from '../Skeleton.svelte';
  let {rows = 5, columns = ['w-32', 'w-20', 'w-48', 'w-16'], label = 'Loading', flush = false}: {
    rows?: number; columns?: string[]; label?: string; flush?: boolean;
  } = $props();
  const pad = $derived(flush ? 'px-0' : 'px-5');
</script>
<div role="status" aria-busy="true">
  <span class="sr-only">{label}</span>
  <div class={['flex h-10 items-center gap-6 border-b', pad]}>
    {#each columns as width, index (index)}<Skeleton class={['h-3 opacity-70', width, index === columns.length - 1 && 'ml-auto']} />{/each}
  </div>
  {#each Array.from({length: rows}, (_, index) => index) as row (row)}
    <div class={['flex h-12 items-center gap-6 border-b last:border-0', pad]}>
      {#each columns as width, index (index)}<Skeleton class={['h-3.5', width, index === columns.length - 1 && 'ml-auto']} />{/each}
    </div>
  {/each}
</div>
