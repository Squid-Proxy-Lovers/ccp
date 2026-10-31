<script lang="ts">
  // Same grid as the loaded dashboard, so nothing shifts when data arrives.
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import SkeletonCard from '../../lib/ui/skeleton/SkeletonCard.svelte';
  // A fixed silhouette of column heights; it reads as "a chart" without implying data.
  const columns = ['h-10', 'h-16', 'h-12', 'h-24', 'h-20', 'h-14', 'h-28', 'h-20', 'h-32', 'h-24', 'h-16', 'h-36', 'h-28', 'h-20'];
</script>
<div class="grid gap-6" role="status" aria-busy="true">
  <span class="sr-only">Loading workspace activity</span>
  <div class="grid grid-cols-2 gap-px overflow-hidden rounded-xl border bg-border shadow-xs lg:grid-cols-5" aria-hidden="true">
    {#each [0, 1, 2] as index (index)}
      <div class={['space-y-3 bg-card p-4 sm:p-5', index === 2 && 'col-span-2 lg:col-span-1']}><Skeleton class="h-3.5 w-20" /><Skeleton class="h-8 w-14" /><Skeleton class="h-3 w-28 opacity-70" /></div>
    {/each}
    <div class="col-span-2 flex items-end gap-6 bg-card p-4 sm:p-5">
      <div class="space-y-3"><Skeleton class="h-3.5 w-24" /><Skeleton class="h-8 w-14" /><Skeleton class="h-3 w-36 opacity-70" /></div>
      <div class="flex h-14 flex-1 items-end gap-1">
        {#each columns as height, index (index)}<Skeleton class={['flex-1 rounded-sm', height === 'h-36' ? 'h-14' : height === 'h-32' || height === 'h-28' ? 'h-10' : 'h-4']} />{/each}
      </div>
    </div>
  </div>
  <div class="grid gap-6 lg:grid-cols-3" aria-hidden="true">
    <SkeletonCard class="min-w-0 lg:col-span-2" action>
      <div class="space-y-5 px-5">
        <div class="flex flex-wrap gap-5">
          {#each ['w-20', 'w-24', 'w-20', 'w-36'] as width, index (index)}<div class="flex items-center gap-2"><Skeleton class="size-2.5" /><Skeleton class={['h-3', width]} /></div>{/each}
        </div>
        <div class="flex h-56 items-end gap-[3%] border-b pb-px pl-9">
          {#each columns as height, index (index)}<Skeleton class={['flex-1 rounded-b-none', height]} />{/each}
        </div>
      </div>
    </SkeletonCard>
    <SkeletonCard>
      <div class="grid gap-5 px-5">
        {#each ['w-full', 'w-3/4', 'w-1/2', 'w-1/3'] as width, index (index)}
          <div class="space-y-2">
            <div class="flex justify-between"><Skeleton class="h-3.5 w-24" /><Skeleton class="h-3.5 w-8" /></div>
            <div class="h-1.5 rounded-full bg-muted"><Skeleton class={['h-full rounded-full', width]} /></div>
          </div>
        {/each}
      </div>
    </SkeletonCard>
  </div>
</div>
