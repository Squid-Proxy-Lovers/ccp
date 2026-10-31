<script lang="ts">
  // A one-time secret: masked, with reveal and copy inside the field. When the
  // clipboard is unavailable the value is revealed and selected for manual copying.
  import {tick} from 'svelte';
  import {Copy, Check, Eye, EyeOff} from '@lucide/svelte';
  import Input from './Input.svelte';
  let {id, label, value, hint = '', oncopy}: {
    id: string; label: string; value: string; hint?: string; oncopy?: (copied: boolean) => void;
  } = $props();
  let shown = $state(false);
  let copied = $state(false);
  let field = $state<HTMLInputElement | null>(null);
  let timer = 0;

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      copied = true;
      clearTimeout(timer);
      timer = window.setTimeout(() => { copied = false; }, 1600);
      oncopy?.(true);
    } catch {
      shown = true;
      await tick();
      field?.focus();
      field?.select();
      oncopy?.(false);
    }
  }
  const icon = 'flex size-8 cursor-pointer items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring [&_svg]:size-4';
</script>
<div class="grid gap-1.5">
  <label for={id} class="text-sm font-medium">{label}</label>
  <div class="relative">
    <Input {id} bind:ref={field} type={shown ? 'text' : 'password'} readonly {value} autocomplete="off" spellcheck={false}
      aria-describedby={hint ? `${id}-hint` : undefined} class="h-10 pr-[4.75rem] font-mono text-xs" />
    <div class="absolute inset-y-0 right-1 flex items-center gap-0.5">
      <button type="button" class={icon} aria-label={shown ? `Hide ${label.toLowerCase()}` : `Show ${label.toLowerCase()}`} aria-pressed={shown} onclick={() => (shown = !shown)}>
        {#if shown}<EyeOff aria-hidden="true" />{:else}<Eye aria-hidden="true" />{/if}
      </button>
      <button type="button" class={icon} aria-label={`Copy ${label.toLowerCase()}`} onclick={() => void copy()}>
        {#if copied}<Check class="text-success" aria-hidden="true" />{:else}<Copy aria-hidden="true" />{/if}
      </button>
    </div>
  </div>
  {#if hint}<p id={`${id}-hint`} class="text-xs text-muted-foreground">{hint}</p>{/if}
</div>
