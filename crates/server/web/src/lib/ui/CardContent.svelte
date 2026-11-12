<script lang="ts">
  import type {Snippet} from 'svelte';
  import {cn} from './cn';
  let {class: className = '', children}: {class?: string; children?: Snippet} = $props();
</script>
<div class={cn('px-5', className)}>{@render children?.()}</div>
