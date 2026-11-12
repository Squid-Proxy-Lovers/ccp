<script lang="ts">
  import {cn} from './cn';
  import type {ClassValue} from './cn';
  let {class: className = ''}: {class?: ClassValue} = $props();
</script>
<div aria-hidden="true" class={cn('skeleton rounded-md', className)}></div>
