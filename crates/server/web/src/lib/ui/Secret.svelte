<script lang="ts">
  // One-time secret display. The value lives only in component state. It takes
  // focus when it appears; if the clipboard fails, the value is revealed and selected.
  import {onMount, tick} from 'svelte';
  import {Copy, Eye, EyeOff, KeyRound} from '@lucide/svelte';
  import Button from './Button.svelte';
  import {copyText} from '../notify.svelte';
  let {title, value, ondismiss}: {title: string; value: string; ondismiss?: () => void} = $props();
  let shown = $state(false);
  let root = $state<HTMLElement | null>(null);
  let code = $state<HTMLElement | null>(null);
  onMount(() => { root?.focus(); });

  async function copy() {
    if (await copyText(value)) return;
    shown = true;
    await tick();
    if (!code) return;
    code.focus();
    const range = document.createRange();
    range.selectNodeContents(code);
    getSelection()?.removeAllRanges();
    getSelection()?.addRange(range);
  }
</script>
<section bind:this={root} tabindex="-1" class="space-y-3 rounded-lg border bg-card p-4 outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-label={title}>
  <div class="flex items-start gap-3">
    <KeyRound class="mt-0.5 size-4 text-destructive" aria-hidden="true" />
    <div class="space-y-1">
      <h3 class="text-sm font-medium">{title}</h3>
      <p class="text-sm text-muted-foreground">Copy this now and store it privately. It won't be shown again.</p>
    </div>
  </div>
  <div class="flex items-center gap-2">
    <code bind:this={code} tabindex="-1" class="min-w-0 flex-1 truncate rounded-md border bg-muted/40 px-3 py-2 font-mono text-[13px] outline-none focus-visible:ring-2 focus-visible:ring-ring">{shown ? value : '•'.repeat(Math.min(value.length, 48))}</code>
    <Button variant="outline" size="icon" aria-label={shown ? 'Hide secret' : 'Show secret'} onclick={() => (shown = !shown)}>
      {#if shown}<EyeOff />{:else}<Eye />{/if}
    </Button>
    <Button variant="outline" onclick={() => void copy()}><Copy /> Copy</Button>
  </div>
  {#if ondismiss}<Button variant="ghost" size="sm" onclick={ondismiss}>I've stored it, dismiss</Button>{/if}
</section>
