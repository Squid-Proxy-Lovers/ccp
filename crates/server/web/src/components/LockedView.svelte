<script lang="ts">
  // Shown when the session can no longer be used. It keeps any unsaved local work
  // and pending write requests on screen so they can be copied before signing in again.
  import {Copy, ArrowRight} from '@lucide/svelte';
  import Button from '../lib/ui/Button.svelte';
  import Textarea from '../lib/ui/Textarea.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Logo from './Logo.svelte';
  import DeletionTracker from '../views/DeletionTracker.svelte';
  import {app} from '../lib/app.svelte';
  import {copyText} from '../lib/notify.svelte';
  const recovery = $derived(app.recoveryText());
  const deletionOnly = $derived(Boolean(app.deletion && (!app.pending || app.pending === app.deletion.reference)));
  const identity = $derived(app.session?.identity);
  const role = $derived(identity?.scope.kind === 'instance' ? 'Instance admin' : `Workspace ${identity?.scope.role ?? ''}`);
  const label = $derived(deletionOnly ? 'Return to sign in, keep deletion tracking' : recovery ? 'Discard local work and sign in' : 'Sign in again');
</script>
<div class="relative isolate flex min-h-svh flex-col items-center justify-center gap-10 overflow-hidden bg-background p-6">
  <div class="login-dots pointer-events-none -z-10" aria-hidden="true"></div>
  <div class="rise flex max-w-sm flex-col items-center text-center">
    <div class="relative mb-7">
      <div class="login-halo pointer-events-none absolute -inset-10 -z-10" aria-hidden="true"></div>
      <Logo class="size-16 drop-shadow-[0_8px_18px_rgb(0_0_0/0.18)]" closed />
    </div>
    <h1 id="page-title" tabindex="-1" class="text-2xl font-semibold tracking-tight outline-none">Session ended</h1>
    <p role="alert" class="mt-2 text-sm text-pretty text-muted-foreground">{app.lockMessage}</p>
    {#if identity}
      <p class="mt-4 text-xs text-muted-foreground">
        Was signed in as <span class="font-medium text-foreground">{identity.principal_name}</span> · {role}
      </p>
    {/if}
    {#if !recovery && !app.deletion}
      <Button class="group mt-7 min-w-44" onclick={() => app.discardAndSignIn()}>
        {label} <ArrowRight class="transition-transform duration-200 group-hover:translate-x-0.5" />
      </Button>
    {/if}
  </div>

  {#if recovery || app.deletion}
    <div class="rise rise-2 w-full max-w-2xl space-y-6 rounded-xl border bg-card/90 p-6 shadow-[0_1px_2px_rgb(0_0_0/0.04),0_12px_32px_-12px_rgb(0_0_0/0.18)] backdrop-blur-sm dark:shadow-[0_0_0_1px_rgb(255_255_255/0.02),0_16px_40px_-16px_rgb(0_0_0/0.9)]">
      {#if recovery}
        <div class="space-y-3">
          <Field id="recovery" label="Unsaved local work and original write requests">
            <Textarea id="recovery" readonly value={recovery} rows={10} class="font-mono text-xs" />
          </Field>
          {#if app.lockAdvice}<p class="text-sm text-muted-foreground">{app.lockAdvice}</p>{/if}
        </div>
      {/if}
      <DeletionTracker />
      <div class="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
        {#if recovery}<Button variant="outline" onclick={() => copyText(recovery)}><Copy /> Copy local work</Button>{/if}
        <Button variant={deletionOnly ? 'default' : 'destructive'} onclick={() => app.discardAndSignIn()}>{label}</Button>
      </div>
    </div>
  {/if}
</div>
