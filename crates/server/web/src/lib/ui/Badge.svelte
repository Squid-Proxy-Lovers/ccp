<script lang="ts">
  // Inline status mark rather than a pill: a small dot carries the state colour and
  // the label stays in text colour. Neutral marks are plain secondary text.
  import type {Snippet} from 'svelte';
  import {cn} from './cn';
  type Tone = 'neutral' | 'outline' | 'success' | 'warning' | 'destructive' | 'info';
  let {tone = 'neutral', class: className = '', children}: {tone?: Tone; class?: string; children: Snippet} = $props();
  const dots: Partial<Record<Tone, string>> = {
    success: 'bg-success',
    warning: 'bg-warning',
    destructive: 'bg-destructive',
    info: 'bg-primary',
  };
  const dot = $derived(dots[tone]);
</script>
<span class={cn('inline-flex w-fit shrink-0 items-center gap-1.5 text-xs whitespace-nowrap [&>svg]:size-3',
  dot ? 'font-medium text-foreground' : 'text-muted-foreground', className)}>
  {#if dot}<span class={['size-1.5 shrink-0 rounded-full', dot]} aria-hidden="true"></span>{/if}
  {@render children()}
</span>
