<script lang="ts">
  // Shown when the session can no longer be used. It keeps any unsaved local work
  // and pending write requests on screen so they can be copied before signing in again.
  import {Lock, Copy} from '@lucide/svelte';
  import Button from '../lib/ui/Button.svelte';
  import Textarea from '../lib/ui/Textarea.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Logo from './Logo.svelte';
  import DeletionTracker from '../views/DeletionTracker.svelte';
  import {app} from '../lib/app.svelte';
  import {copyText} from '../lib/notify.svelte';
  const recovery = $derived(app.recoveryText());
  const deletionOnly = $derived(Boolean(app.deletion && (!app.pending || app.pending === app.deletion.reference)));
  const wide = $derived(Boolean(recovery || app.deletion));
</script>
<div class="flex min-h-svh flex-col items-center justify-center gap-6 bg-background p-6">
  <Logo class="rise size-10" />
  <div class={['rise rise-2 w-full rounded-xl border bg-card p-6 shadow-[0_1px_2px_rgb(0_0_0/0.04),0_12px_32px_-12px_rgb(0_0_0/0.18)] dark:shadow-[0_0_0_1px_rgb(255_255_255/0.02),0_16px_40px_-16px_rgb(0_0_0/0.9)]', wide ? 'max-w-2xl' : 'max-w-sm']}>
    <div class="space-y-1.5">
      <h1 id="page-title" tabindex="-1" class="flex items-center gap-2 text-xl font-semibold tracking-tight outline-none">
        <Lock class="size-4 text-destructive" aria-hidden="true" /> Session ended
      </h1>
      <p role="alert" class="text-sm text-muted-foreground">{app.lockMessage}</p>
    </div>
    {#if recovery}
      <div class="mt-6 space-y-3">
        <Field id="recovery" label="Unsaved local work and original write requests">
          <Textarea id="recovery" readonly value={recovery} rows={10} class="font-mono text-xs" />
        </Field>
        {#if app.lockAdvice}<p class="text-sm text-muted-foreground">{app.lockAdvice}</p>{/if}
      </div>
    {/if}
    {#if app.deletion}<div class="mt-6"><DeletionTracker /></div>{/if}
    <div class={['mt-6 flex gap-2', wide ? 'flex-col-reverse sm:flex-row sm:justify-end' : 'flex-col']}>
      {#if recovery}<Button variant="outline" onclick={() => copyText(recovery)}><Copy /> Copy local work</Button>{/if}
      <Button variant={deletionOnly || !recovery ? 'default' : 'destructive'} class={wide ? '' : 'w-full'} onclick={() => app.discardAndSignIn()}>
        {deletionOnly ? 'Return to sign in, keep deletion tracking' : recovery ? 'Discard local work and sign in' : 'Return to sign in'}
      </Button>
    </div>
  </div>
</div>
