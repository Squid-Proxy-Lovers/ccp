<script lang="ts">
  import type {Snippet} from 'svelte';
  import {cn} from './cn';
  let {onselect, children, checked, destructive = false, disabled = false}: {
    onselect: () => void; children: Snippet; checked?: boolean; destructive?: boolean; disabled?: boolean;
  } = $props();
</script>
<button type="button" role={checked === undefined ? 'menuitem' : 'menuitemradio'} aria-checked={checked} aria-disabled={disabled || undefined} tabindex="-1"
  onclick={() => { if (!disabled) onselect(); }}
  class={cn('flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none select-none hover:bg-accent focus:bg-accent aria-disabled:pointer-events-none aria-disabled:opacity-50 [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-muted-foreground',
    destructive && 'text-destructive [&_svg]:!text-destructive')}>
  {@render children()}
</button>
