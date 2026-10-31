<script lang="ts">
  // Shown while a session activates (sign-in, boot with a live cookie), instead of
  // briefly rendering the shell without a workspace.
  import Skeleton from '../lib/ui/Skeleton.svelte';
  import Delayed from '../lib/ui/Delayed.svelte';
  import PageSkeleton from './PageSkeleton.svelte';
  import Logo from './Logo.svelte';
</script>
<div class="flex min-h-svh bg-background" role="status" aria-busy="true">
  <span class="sr-only">Opening your workspace</span>
  <Delayed>
    <aside class="hidden w-64 shrink-0 flex-col gap-6 border-r border-sidebar-border bg-sidebar p-4 lg:flex" aria-hidden="true">
      <div class="flex items-center gap-2.5">
        <Logo class="size-8 shrink-0" motion="loading" />
        <div class="space-y-1.5"><Skeleton class="h-3.5 w-24" /><Skeleton class="h-3 w-16 opacity-70" /></div>
      </div>
      <div class="space-y-3">
        {#each ['w-28', 'w-20', 'w-32', 'w-24', 'w-22', 'w-18'] as width, index (index)}<div class="flex items-center gap-2.5"><Skeleton class="size-4" /><Skeleton class={['h-3.5', width]} /></div>{/each}
      </div>
    </aside>
    <div class="flex min-w-0 flex-1 flex-col">
      <div class="flex h-14 items-center justify-between border-b px-4" aria-hidden="true">
        <div class="flex items-center gap-3"><Logo class="size-7 lg:hidden" motion="loading" /><Skeleton class="h-3.5 w-40" /></div>
        <div class="flex gap-2"><Skeleton class="size-8" /><Skeleton class="size-8" /></div>
      </div>
      <div class="mx-auto w-full max-w-[1400px] p-4 md:p-6 lg:p-8"><PageSkeleton /></div>
    </div>
  </Delayed>
</div>
