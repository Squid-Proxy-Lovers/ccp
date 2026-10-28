<script lang="ts">
  // Generic dashboard-shaped placeholder: title block, stat row, two cards.
  import Skeleton from '../lib/ui/Skeleton.svelte';
  import SkeletonCard from '../lib/ui/skeleton/SkeletonCard.svelte';
</script>
<div class="space-y-6" aria-hidden="true">
  <div class="space-y-2.5"><Skeleton class="h-7 w-44" /><Skeleton class="h-3.5 w-72 max-w-full opacity-70" /></div>
  <div class="grid grid-cols-2 gap-3 sm:gap-4 xl:grid-cols-4">
    {#each [0, 1, 2, 3] as index (index)}
      <div class="space-y-3 rounded-xl border bg-card p-4 shadow-xs sm:p-5"><Skeleton class="h-3.5 w-20" /><Skeleton class="h-7 w-14" /><Skeleton class="h-3 w-24 opacity-70" /></div>
    {/each}
  </div>
  <div class="grid gap-6 lg:grid-cols-3">
    <SkeletonCard class="lg:col-span-2" lines={6} />
    <SkeletonCard lines={4} />
  </div>
</div>
