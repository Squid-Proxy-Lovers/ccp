<script lang="ts">
  import type {HTMLSelectAttributes} from 'svelte/elements';
  import {ChevronDown} from '@lucide/svelte';
  import {cn} from './cn';
  let {value = $bindable(''), options, placeholder, class: className = '', ...rest}: HTMLSelectAttributes & {
    options: {value: string; label: string}[]; placeholder?: string;
  } = $props();
</script>
<div class={cn('relative', className)}>
  <select bind:value class="h-9 w-full cursor-pointer appearance-none rounded-md border border-input bg-transparent py-1 pr-9 pl-3 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 dark:bg-input/30 [&>option]:bg-popover" {...rest}>
    {#if placeholder !== undefined}<option value="">{placeholder}</option>{/if}
    {#each options as option (option.value)}<option value={option.value}>{option.label}</option>{/each}
  </select>
  <ChevronDown class="pointer-events-none absolute top-1/2 right-3 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
</div>
