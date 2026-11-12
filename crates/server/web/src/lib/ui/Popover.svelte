<script lang="ts">
  // Non-modal disclosure panel: Escape, outside click or tabbing away closes it,
  // and focus returns to the trigger when closed from inside.
  import type {Snippet} from 'svelte';
  import {cn} from './cn';
  let {trigger, children, label, align = 'end', class: className = '', triggerClass = '', open = $bindable(false)}: {
    trigger: Snippet; children: Snippet<[() => void]>; label: string; align?: 'start' | 'end';
    class?: string; triggerClass?: string; open?: boolean;
  } = $props();
  const id = $props.id();
  let root: HTMLDivElement;
  let button: HTMLButtonElement;
  let panel: HTMLDivElement | undefined = $state();
  const close = () => { open = false; button?.focus(); };
  function outside(event: PointerEvent) { if (open && !root.contains(event.target as Node)) open = false; }
  function focusout(event: FocusEvent) {
    if (open && event.relatedTarget instanceof Node && !root.contains(event.relatedTarget)) open = false;
  }
  $effect(() => {
    if (open && panel) panel.querySelector<HTMLElement>('input:checked, button, input, [tabindex="0"]')?.focus();
  });
</script>

<svelte:window onpointerdown={outside} />
<div bind:this={root} class="relative" role="presentation" onfocusout={focusout}
  onkeydown={event => { if (open && event.key === 'Escape') { event.preventDefault(); close(); } }}>
  <button bind:this={button} type="button" aria-expanded={open} aria-controls={id} aria-label={label}
    class={cn('cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-ring', triggerClass)} onclick={() => (open = !open)}>
    {@render trigger()}
  </button>
  {#if open}
    <div bind:this={panel} {id} role="dialog" aria-label={label}
      class={cn('absolute top-full z-50 mt-1.5 rounded-xl border bg-popover p-4 text-popover-foreground shadow-lg', align === 'end' ? 'right-0' : 'left-0', className)}>
      {@render children(close)}
    </div>
  {/if}
</div>
