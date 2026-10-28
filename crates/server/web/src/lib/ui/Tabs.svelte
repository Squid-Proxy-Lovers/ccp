<script lang="ts" generics="T extends string">
  // Segmented tabs; arrow keys move between options (WAI-ARIA tabs pattern).
  let {value = $bindable(), items, label, class: className = '', onchange}: {
    value: T; items: {value: T; label: string; count?: number}[]; label: string; class?: string;
    onchange?: (value: T) => void;
  } = $props();
  let list: HTMLDivElement;
  function select(next: T) { if (next !== value) { value = next; onchange?.(next); } }
  function keydown(event: KeyboardEvent) {
    const index = items.findIndex(item => item.value === value);
    const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = items[(index + step + items.length) % items.length];
    if (!next) return;
    select(next.value);
    list.querySelector<HTMLButtonElement>(`[data-value="${CSS.escape(next.value)}"]`)?.focus();
  }
</script>
<div bind:this={list} role="tablist" aria-label={label} tabindex="-1" onkeydown={keydown}
  class={['inline-flex h-9 w-fit items-center rounded-lg bg-muted p-[3px] text-muted-foreground', className]}>
  {#each items as item (item.value)}
    <button type="button" role="tab" data-value={item.value} aria-selected={item.value === value}
      tabindex={item.value === value ? 0 : -1} onclick={() => select(item.value)}
      class="inline-flex h-full cursor-pointer items-center gap-1.5 rounded-md px-3 text-sm font-medium whitespace-nowrap transition-colors hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none aria-selected:bg-background aria-selected:text-foreground aria-selected:shadow-sm dark:aria-selected:bg-input/40">
      {item.label}
      {#if item.count !== undefined}<span class="tabular rounded bg-foreground/[0.07] px-1.5 text-xs">{item.count}</span>{/if}
    </button>
  {/each}
</div>
