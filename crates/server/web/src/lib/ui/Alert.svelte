<script lang="ts">
  import type {Snippet} from 'svelte';
  import {CircleAlert, Info, TriangleAlert, CircleCheck} from '@lucide/svelte';
  import {cn} from './cn';
  type Tone = 'info' | 'warning' | 'destructive' | 'success';
  let {tone = 'info', title, children, action, class: className = '', live = false}: {
    tone?: Tone; title?: string; children?: Snippet; action?: Snippet; class?: string; live?: boolean;
  } = $props();
  const tones: Record<Tone, string> = {
    info: 'bg-card text-card-foreground [&>svg]:text-muted-foreground',
    warning: 'border-warning/35 bg-warning/[0.06] [&>svg]:text-warning',
    destructive: 'border-destructive/35 bg-destructive/[0.06] [&>svg]:text-destructive',
    success: 'border-success/35 bg-success/[0.06] [&>svg]:text-success',
  };
  const Icon = $derived({info: Info, warning: TriangleAlert, destructive: CircleAlert, success: CircleCheck}[tone]);
</script>
<div role={live ? (tone === 'destructive' ? 'alert' : 'status') : undefined}
  class={cn('relative grid w-full grid-cols-[1rem_1fr_auto] items-start gap-x-3 gap-y-1 rounded-lg border px-4 py-3 text-sm', tones[tone], className)}>
  <Icon class="mt-0.5 size-4" aria-hidden="true" />
  <div class="min-w-0 space-y-1">
    {#if title}<p class="leading-snug font-medium">{title}</p>{/if}
    {#if children}<div class="text-muted-foreground [&_p]:leading-relaxed">{@render children()}</div>{/if}
  </div>
  {#if action}<div class="flex items-center gap-2 self-center">{@render action()}</div>{/if}
</div>
