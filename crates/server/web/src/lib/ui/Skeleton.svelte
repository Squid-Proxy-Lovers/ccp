<script lang="ts">
  import {cn} from './cn';
  let {class: className = ''}: {class?: string} = $props();
</script>
<div aria-hidden="true" class={cn('animate-pulse rounded-md bg-accent', className)}></div>
