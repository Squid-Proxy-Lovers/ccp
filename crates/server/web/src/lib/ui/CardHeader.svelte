<script lang="ts">
  import type {Snippet} from 'svelte';
  import {cn} from './cn';
  let {title, description, class: className = '', action, headingLevel = 2}: {
    title: string; description?: string; class?: string; action?: Snippet; headingLevel?: 2 | 3;
  } = $props();
</script>
<div class={cn('flex items-start gap-4 px-5', className)}>
  <div class="min-w-0 flex-1 space-y-1">
    <svelte:element this={`h${headingLevel}`} class="text-[15px] leading-tight font-semibold tracking-tight">{title}</svelte:element>
    {#if description}<p class="text-sm text-muted-foreground">{description}</p>{/if}
  </div>
  {#if action}<div class="flex shrink-0 items-center gap-2">{@render action()}</div>{/if}
</div>
