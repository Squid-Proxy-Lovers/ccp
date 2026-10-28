<script lang="ts">
  import {Lock, Copy} from '@lucide/svelte';
  import Button from '../lib/ui/Button.svelte';
  import Textarea from '../lib/ui/Textarea.svelte';
  import Field from '../lib/ui/Field.svelte';
  import DeletionTracker from '../views/DeletionTracker.svelte';
  import {app} from '../lib/app.svelte';
  import {copyText} from '../lib/notify.svelte';
  const recovery = $derived(app.recoveryText());
  const deletionOnly = $derived(Boolean(app.deletion && (!app.pending || app.pending === app.deletion.reference)));
</script>
<div class="flex min-h-svh items-center justify-center bg-muted/40 p-6">
  <div class="w-full max-w-2xl space-y-6 rounded-xl border bg-card p-6 shadow-sm">
    <div class="flex items-start gap-3">
      <div class="flex size-10 shrink-0 items-center justify-center rounded-lg bg-destructive/10 text-destructive"><Lock class="size-5" /></div>
      <div class="space-y-1.5">
        <h1 id="page-title" tabindex="-1" class="text-xl font-semibold tracking-tight outline-none">Session changed or unavailable</h1>
        <p class="text-sm text-muted-foreground">{app.lockMessage}</p>
      </div>
    </div>
    {#if recovery}
      <div class="space-y-3">
        <Field id="recovery" label="Unsaved local work and original write requests">
          <Textarea id="recovery" readonly value={recovery} rows={10} class="font-mono text-xs" />
        </Field>
        {#if app.lockAdvice}<p class="text-sm text-muted-foreground">{app.lockAdvice}</p>{/if}
        <Button variant="outline" onclick={() => copyText(recovery)}><Copy /> Copy local work</Button>
      </div>
    {/if}
    <DeletionTracker />
    <div class="flex justify-end border-t pt-4">
      <Button variant={deletionOnly || !recovery ? 'default' : 'destructive'} onclick={() => app.discardAndSignIn()}>
        {deletionOnly ? 'Return to sign in, keep deletion tracking' : recovery ? 'Discard local work and sign in' : 'Return to sign in'}
      </Button>
    </div>
  </div>
</div>
