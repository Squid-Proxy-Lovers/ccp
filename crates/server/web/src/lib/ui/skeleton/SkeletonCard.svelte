<script lang="ts">
  // A card-shaped placeholder: title and description bars, then lines or custom content.
  import type {Snippet} from 'svelte';
  import Skeleton from '../Skeleton.svelte';
  import {cn} from '../cn';
  let {lines = 3, action = false, class: className = '', children, label}: {
    lines?: number; action?: boolean; class?: string; children?: Snippet; label?: string;
  } = $props();
  const widths = ['w-full', 'w-11/12', 'w-4/5', 'w-2/3', 'w-3/4'];
</script>
<div class={cn('flex flex-col gap-5 rounded-xl border bg-card py-5 shadow-xs', className)} aria-busy="true">
  {#if label}<span class="sr-only" role="status">{label}</span>{/if}
  <div class="flex items-start justify-between gap-4 px-5">
    <div class="space-y-2">
      <Skeleton class="h-4 w-36" />
      <Skeleton class="h-3 w-56 max-w-full opacity-70" />
    </div>
    {#if action}<Skeleton class="h-8 w-24" />{/if}
  </div>
  {#if children}
    {@render children()}
  {:else}
    <div class="space-y-2.5 px-5">
      {#each Array.from({length: lines}, (_, index) => index) as line (line)}<Skeleton class={['h-3.5', widths[line % widths.length]]} />{/each}
    </div>
  {/if}
</div>
