<script lang="ts">
  import type {HTMLInputAttributes} from 'svelte/elements';
  import {cn} from './cn';
  let {value = $bindable(''), class: className = '', ref = $bindable(null), ...rest}:
    HTMLInputAttributes & {ref?: HTMLInputElement | null} = $props();
</script>
<input bind:this={ref} bind:value class={cn('flex h-9 w-full min-w-0 rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-[color,box-shadow] outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 read-only:bg-muted/50 aria-invalid:border-destructive dark:bg-input/30', className)} {...rest} />
