<script lang="ts">
  // A one-time secret on a slip: only its public prefix shows until revealed, with
  // reveal and copy beside it. When the clipboard is unavailable the value is
  // revealed and selected for manual copying.
  import {tick} from 'svelte';
  import {Copy, Check, Eye, EyeOff} from '@lucide/svelte';
  let {id, label, value, prefix = '', note = '', hint = '', oncopy}: {
    id: string; label: string; value: string; prefix?: string; note?: string; hint?: string; oncopy?: (copied: boolean) => void;
  } = $props();
  let shown = $state(false);
  let copied = $state(false);
  let field = $state<HTMLInputElement | null>(null);
  const masked = $derived(prefix + '•'.repeat(12));

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      copied = true;
      oncopy?.(true);
    } catch {
      shown = true;
      await tick();
      field?.focus();
      field?.select();
      oncopy?.(false);
    }
  }
  const icon = 'flex size-8 shrink-0 cursor-pointer items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring [&_svg]:size-4';
</script>
<div class="grid gap-1.5">
  <div class="rounded-lg border bg-muted/40 transition-colors focus-within:border-ring">
    <div class="flex items-baseline justify-between gap-3 px-3 pt-2.5">
      <label for={id} class="text-sm font-medium">{label}</label>
      {#if note}<span class="truncate text-xs text-muted-foreground">{note}</span>{/if}
    </div>
    <div class="flex items-center gap-1 py-1.5 pr-1.5 pl-3">
      <input {id} bind:this={field} readonly value={shown ? value : masked} autocomplete="off" spellcheck={false}
        aria-describedby={hint ? `${id}-hint` : undefined} aria-label={shown ? undefined : `${label}, hidden`}
        class={['min-w-0 flex-1 truncate bg-transparent font-mono text-xs outline-none', shown ? 'text-foreground' : 'text-muted-foreground']} />
      <button type="button" class={icon} aria-label={shown ? `Hide ${label.toLowerCase()}` : `Show ${label.toLowerCase()}`} aria-pressed={shown} onclick={() => (shown = !shown)}>
        {#if shown}<EyeOff aria-hidden="true" />{:else}<Eye aria-hidden="true" />{/if}
      </button>
      <button type="button" aria-label={`Copy ${label.toLowerCase()}`} onclick={() => void copy()}
        class={['inline-flex h-8 shrink-0 cursor-pointer items-center gap-1.5 rounded-md border px-2.5 text-xs font-medium shadow-xs outline-none focus-visible:ring-2 focus-visible:ring-ring [&_svg]:size-3.5',
          copied ? 'border-transparent bg-foreground text-background' : 'bg-background hover:bg-accent']}>
        {#if copied}<Check aria-hidden="true" /> Copied{:else}<Copy aria-hidden="true" /> Copy{/if}
      </button>
    </div>
  </div>
  {#if hint}<p id={`${id}-hint`} class="text-xs text-muted-foreground">{hint}</p>{/if}
</div>
