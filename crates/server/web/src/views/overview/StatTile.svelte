<script lang="ts">
  import type {Snippet} from 'svelte';
  let {label, value, detail, trend}: {label: string; value: string; detail?: string; trend?: Snippet} = $props();
</script>

<div class="flex items-end justify-between gap-3 rounded-xl border bg-card p-4 shadow-xs sm:p-5">
  <div class="min-w-0 space-y-1">
    <p class="text-sm text-muted-foreground">{label}</p>
    <p class="text-2xl font-semibold tracking-tight sm:text-3xl">{value}</p>
    {#if detail}<p class="truncate text-xs text-muted-foreground">{detail}</p>{/if}
  </div>
  {#if trend}<div class="hidden shrink-0 sm:block">{@render trend()}</div>{/if}
</div>
