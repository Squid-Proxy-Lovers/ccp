<script lang="ts">
  import type {Snippet, Component} from 'svelte';
  let {icon: Icon, title, description, children}: {
    icon?: Component<{class?: string}>; title: string; description?: string; children?: Snippet;
  } = $props();
</script>
<div class="flex flex-col items-center justify-center gap-3 rounded-lg border border-dashed px-6 py-10 text-center">
  {#if Icon}<div class="flex size-10 items-center justify-center rounded-lg bg-muted text-muted-foreground"><Icon class="size-5" /></div>{/if}
  <div class="space-y-1">
    <p class="text-sm font-medium">{title}</p>
    {#if description}<p class="mx-auto max-w-sm text-sm text-muted-foreground">{description}</p>{/if}
  </div>
  {#if children}<div class="mt-1 flex flex-wrap justify-center gap-2">{@render children()}</div>{/if}
</div>
