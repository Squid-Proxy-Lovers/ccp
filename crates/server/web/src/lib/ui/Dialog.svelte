<script lang="ts">
  // Native modal <dialog>: focus trap, Escape and inert background come from the platform.
  import type {Snippet} from 'svelte';
  import {X} from '@lucide/svelte';
  let {open = $bindable(false), title, description, children, footer, onclose, size = 'md'}: {
    open?: boolean; title: string; description?: string; children?: Snippet; footer?: Snippet;
    onclose?: () => void; size?: 'md' | 'lg';
  } = $props();
  let dialog: HTMLDialogElement;
  $effect(() => {
    if (open && !dialog.open) dialog.showModal();
    else if (!open && dialog.open) dialog.close();
  });
  function closed() { open = false; onclose?.(); }
</script>
<dialog bind:this={dialog} onclose={closed} aria-labelledby="dialog-title"
  class={['m-auto w-[calc(100%-2rem)] rounded-xl border bg-background p-0 text-foreground shadow-lg backdrop:bg-black/50', size === 'lg' ? 'max-w-2xl' : 'max-w-lg']}>
  {#if open}
    <div class="grid gap-4 p-6">
      <div class="space-y-1.5 pr-8">
        <h2 id="dialog-title" class="text-lg leading-none font-semibold tracking-tight">{title}</h2>
        {#if description}<p class="text-sm text-muted-foreground">{description}</p>{/if}
      </div>
      {@render children?.()}
      {#if footer}<div class="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">{@render footer()}</div>{/if}
    </div>
    <button type="button" aria-label="Close" onclick={() => (open = false)}
      class="absolute top-4 right-4 cursor-pointer rounded-sm p-1 text-muted-foreground opacity-70 transition-opacity hover:opacity-100 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"><X class="size-4" /></button>
  {/if}
</dialog>
