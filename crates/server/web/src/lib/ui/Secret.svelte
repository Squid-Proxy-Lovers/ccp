<script lang="ts">
  // One-time secret display. The value lives only in component state.
  import {Copy, Eye, EyeOff, KeyRound} from '@lucide/svelte';
  import Button from './Button.svelte';
  import {copyText} from '../notify.svelte';
  let {title, value, ondismiss}: {title: string; value: string; ondismiss?: () => void} = $props();
  let shown = $state(false);
</script>
<section class="space-y-3 rounded-lg border border-warning/35 bg-warning/[0.06] p-4" aria-label={title}>
  <div class="flex items-start gap-3">
    <KeyRound class="mt-0.5 size-4 text-warning" aria-hidden="true" />
    <div class="space-y-1">
      <h3 class="text-sm font-medium">{title}</h3>
      <p class="text-sm text-muted-foreground">Copy this now and store it privately. It won't be shown again.</p>
    </div>
  </div>
  <div class="flex items-center gap-2">
    <code class="min-w-0 flex-1 truncate rounded-md border bg-background px-3 py-2 font-mono text-[13px]">{shown ? value : '•'.repeat(Math.min(value.length, 48))}</code>
    <Button variant="outline" size="icon" aria-label={shown ? 'Hide secret' : 'Show secret'} onclick={() => (shown = !shown)}>
      {#if shown}<EyeOff />{:else}<Eye />{/if}
    </Button>
    <Button variant="outline" onclick={() => copyText(value)}><Copy /> Copy</Button>
  </div>
  {#if ondismiss}<Button variant="ghost" size="sm" onclick={ondismiss}>I've stored it, dismiss</Button>{/if}
</section>
