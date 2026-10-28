<script lang="ts">
  import type {Snippet} from 'svelte';
  import {cn} from './cn';
  let {class: className = '', children, ...rest}: {class?: string; children?: Snippet; [key: string]: unknown} = $props();
</script>
<div class={cn('flex flex-col gap-5 rounded-xl border bg-card py-5 text-card-foreground shadow-xs', className)} {...rest}>
  {@render children?.()}
</div>
