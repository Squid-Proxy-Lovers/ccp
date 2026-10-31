<script lang="ts">
  // Renders its children only after `delay` ms, so fast loads never flash a placeholder.
  import type {Snippet} from 'svelte';
  let {delay = 150, children}: {delay?: number; children: Snippet} = $props();
  let ready = $state(false);
  $effect(() => {
    const timer = setTimeout(() => { ready = true; }, delay);
    return () => clearTimeout(timer);
  });
</script>
{#if ready}{@render children()}{/if}
