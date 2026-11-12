<script lang="ts">
  import {X, CircleCheck, CircleAlert, TriangleAlert, Info} from '@lucide/svelte';
  import {toasts, dismiss} from '../lib/notify.svelte';
  const icons = {info: Info, success: CircleCheck, warning: TriangleAlert, destructive: CircleAlert};
  const tones = {info: 'text-muted-foreground', success: 'text-success', warning: 'text-warning', destructive: 'text-destructive'};
</script>
<section aria-label="Notifications" class="pointer-events-none fixed right-4 bottom-4 z-[60] flex w-[min(26rem,calc(100vw-2rem))] flex-col gap-2">
  <div role="status" aria-live="polite" class="contents">
    {#each toasts as toast (toast.id)}
      {@const Icon = icons[toast.tone]}
      <div class="pointer-events-auto flex items-start gap-3 rounded-lg border bg-popover p-3.5 text-sm text-popover-foreground shadow-lg">
        <Icon class={['mt-0.5 size-4 shrink-0', tones[toast.tone]]} aria-hidden="true" />
        <p class="min-w-0 flex-1 leading-relaxed break-words">{toast.text}</p>
        <button type="button" aria-label="Dismiss notification" onclick={() => dismiss(toast.id)}
          class="-m-1 cursor-pointer rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"><X class="size-3.5" /></button>
      </div>
    {/each}
  </div>
</section>
