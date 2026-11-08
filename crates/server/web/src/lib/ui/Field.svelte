<script lang="ts">
  // Label + control + hint/error with stable ids for assistive tech.
  import type {Snippet} from 'svelte';
  let {label, hint, error, id, children, class: className = ''}: {
    label: string; hint?: string; error?: string; id: string; children: Snippet; class?: string;
  } = $props();
</script>
<div class={['grid gap-2', className]}>
  <label for={id} class="text-sm leading-none font-medium select-none">{label}</label>
  {@render children()}
  {#if error}<p id={`${id}-error`} class="text-[13px] text-destructive">{error}</p>
  {:else if hint}<p id={`${id}-hint`} class="text-[13px] text-muted-foreground">{hint}</p>{/if}
</div>
