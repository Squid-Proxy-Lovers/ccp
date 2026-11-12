<script lang="ts">
  // Mirrors EntryEditor: title and revision, metadata grid, fields, actions.
  import Skeleton from '../../lib/ui/Skeleton.svelte';
</script>
<div role="status" aria-busy="true" class="space-y-6">
  <span class="sr-only">Loading entry</span>
  <div class="flex items-center justify-between" aria-hidden="true"><Skeleton class="h-6 w-48" /><Skeleton class="h-5 w-20" /></div>
  <div class="grid gap-4 sm:grid-cols-2" aria-hidden="true">
    {#each ['w-36', 'w-36', 'w-72', 'w-52'] as width, index (index)}
      <div class="space-y-1.5"><Skeleton class="h-3 w-16 opacity-70" /><Skeleton class={['h-4 max-w-full', width]} /></div>
    {/each}
  </div>
  <div class="space-y-5" aria-hidden="true">
    <div class="space-y-2"><Skeleton class="h-3.5 w-24" /><Skeleton class="h-9 w-full" /></div>
    <div class="space-y-2"><Skeleton class="h-3.5 w-16" /><Skeleton class="h-20 w-full" /></div>
    <div class="space-y-2"><Skeleton class="h-3.5 w-20" /><Skeleton class="h-64 w-full" /></div>
    <div class="flex justify-end gap-2"><Skeleton class="h-9 w-44" /><Skeleton class="h-9 w-36" /></div>
  </div>
</div>
