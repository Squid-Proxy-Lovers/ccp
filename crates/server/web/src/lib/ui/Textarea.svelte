<script lang="ts">
  import type {HTMLTextareaAttributes} from 'svelte/elements';
  import {cn} from './cn';
  let {value = $bindable(''), class: className = '', ref = $bindable(null), ...rest}:
    HTMLTextareaAttributes & {ref?: HTMLTextAreaElement | null} = $props();
</script>
<textarea bind:this={ref} bind:value class={cn('flex min-h-24 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs transition-[color,box-shadow] outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 read-only:bg-muted/50 dark:bg-input/30', className)} {...rest}></textarea>
