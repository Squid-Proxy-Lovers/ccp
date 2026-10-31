<script lang="ts">
  // A terminal line for a public command (never a secret), with a copy button.
  import {Copy, Check} from '@lucide/svelte';
  let {command, label = 'Copy command'}: {command: string; label?: string} = $props();
  let copied = $state(false);
  let timer = 0;
  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      copied = true;
      clearTimeout(timer);
      timer = window.setTimeout(() => { copied = false; }, 1600);
    } catch { /* The command stays visible for manual copying. */ }
  }
</script>
<div class="flex items-center gap-3 rounded-lg bg-neutral-950 py-1.5 pr-1.5 pl-4 font-mono text-[13px] text-neutral-100 ring-1 ring-white/10 dark:bg-white/[0.06]">
  <span class="text-neutral-500 select-none" aria-hidden="true">$</span>
  <code class="min-w-0 flex-1 truncate">{command}</code>
  <button type="button" aria-label={label} onclick={() => void copy()}
    class="flex size-8 shrink-0 cursor-pointer items-center justify-center rounded-md text-neutral-400 outline-none hover:bg-white/10 hover:text-white focus-visible:ring-2 focus-visible:ring-white/60 [&_svg]:size-4">
    {#if copied}<Check aria-hidden="true" />{:else}<Copy aria-hidden="true" />{/if}
  </button>
</div>
