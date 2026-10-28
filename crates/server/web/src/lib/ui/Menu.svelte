<script lang="ts">
  // Minimal disclosure menu: button + list, Escape/outside click close, arrow keys move.
  import type {Snippet} from 'svelte';
  import {cn} from './cn';
  let {trigger, children, label, align = 'start', side = 'bottom', class: className = '', open = $bindable(false), triggerClass = ''}: {
    trigger: Snippet; children: Snippet<[() => void]>; label: string; align?: 'start' | 'end'; side?: 'bottom' | 'top';
    class?: string; open?: boolean; triggerClass?: string;
  } = $props();
  let root: HTMLDivElement;
  let button: HTMLButtonElement;
  const close = () => { open = false; button?.focus(); };
  function outside(event: PointerEvent) { if (open && !root.contains(event.target as Node)) open = false; }
  function keydown(event: KeyboardEvent) {
    if (!open) return;
    if (event.key === 'Escape') { event.preventDefault(); close(); return; }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    event.preventDefault();
    const items = [...root.querySelectorAll<HTMLElement>('[role=menuitem]:not([aria-disabled=true])')];
    const index = items.indexOf(document.activeElement as HTMLElement);
    items[(index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length]?.focus();
  }
  $effect(() => {
    if (open) queueMicrotask(() => root.querySelector<HTMLElement>('[role=menuitem]')?.focus());
  });
</script>
<svelte:window onpointerdown={outside} />
<div bind:this={root} class="relative" role="presentation" onkeydown={keydown}>
  <button bind:this={button} type="button" aria-haspopup="menu" aria-expanded={open} aria-label={label}
    class={cn('cursor-pointer outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50', triggerClass)} onclick={() => (open = !open)}>
    {@render trigger()}
  </button>
  {#if open}
    <div role="menu" aria-label={label}
      class={cn('absolute z-50 min-w-48 overflow-hidden rounded-lg border bg-popover p-1 text-popover-foreground shadow-md',
        align === 'end' ? 'right-0' : 'left-0', side === 'top' ? 'bottom-full mb-1.5' : 'top-full mt-1.5', className)}>
      {@render children(close)}
    </div>
  {/if}
</div>
