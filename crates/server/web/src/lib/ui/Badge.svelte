<script lang="ts">
  import type {Snippet} from 'svelte';
  import {cn} from './cn';
  type Tone = 'neutral' | 'outline' | 'success' | 'warning' | 'destructive' | 'info';
  let {tone = 'neutral', class: className = '', children}: {tone?: Tone; class?: string; children: Snippet} = $props();
  const tones: Record<Tone, string> = {
    neutral: 'border-transparent bg-secondary text-secondary-foreground',
    outline: 'text-foreground',
    success: 'border-success/25 bg-success/10 text-success-text',
    warning: 'border-warning/30 bg-warning/10 text-warning-text',
    destructive: 'border-destructive/25 bg-destructive/10 text-destructive',
    info: 'border-chart-1/25 bg-chart-1/10 text-info-text',
  };
</script>
<span class={cn('inline-flex w-fit shrink-0 items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium whitespace-nowrap [&>svg]:size-3', tones[tone], className)}>{@render children()}</span>
