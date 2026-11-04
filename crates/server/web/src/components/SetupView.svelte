<script lang="ts">
  // First-run setup: verify the code the server printed, name the first workspace,
  // then show both admin keys once. Keys never touch browser storage.
  import {onMount, untrack, tick} from 'svelte';
  import {ArrowRight, Check, CircleAlert, ShieldAlert} from '@lucide/svelte';
  import Logo from './Logo.svelte';
  import CenteredPage from './CenteredPage.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Input from '../lib/ui/Input.svelte';
  import SecretField from '../lib/ui/SecretField.svelte';
  import {setupRequest, setupKeys, validSetupName, SetupError} from '../lib/setup.js';

  let {initialCode = '', initialError = ''}: {initialCode?: string; initialError?: string} = $props();
  let code = $state(untrack(() => initialCode));
  let error = $state(untrack(() => initialError));
  let step = $state<'code' | 'details' | 'keys' | 'uncertain' | 'complete'>('code');
  let busy = $state(false);
  let operator = $state('');
  let workspace = $state('');
  let acknowledged = $state(false);
  let copied = $state('');
  let keys = $state<{instance_admin_key: {secret: string}; workspace_owner_key: {secret: string}} | null>(null);
  let moved = false;
  // Focus follows the step once the user has acted, not on first load.
  $effect(() => {
    const target = index >= 0 ? `setup-step-${index}` : 'setup-title';
    if (!moved) { moved = true; return; }
    void tick().then(() => document.getElementById(target)?.focus());
  });

  const STEPS = ['Setup code', 'Name your workspace', 'Save your admin keys'];
  const index = $derived(step === 'code' ? 0 : step === 'details' ? 1 : step === 'keys' ? 2 : -1);
  const summary = $derived(['Verified', `${workspace.trim()} · ${operator.trim()}`, '']);
  const prefilled = $derived(Boolean(initialCode) && code === initialCode);
  const host = location.host;

  function failure(value: unknown, claiming: boolean) {
    error = value instanceof Error ? value.message : 'Setup could not be verified.';
    if (value instanceof SetupError && value.status === 409) step = 'complete';
    else if (value instanceof SetupError && value.uncertain && claiming) step = 'uncertain';
    else if (value instanceof SetupError && [401, 410].includes(value.status)) step = 'code';
  }
  async function verify() {
    if (busy) return;
    busy = true; error = '';
    try { await setupRequest('verify', {code}); step = 'details'; }
    catch (value) { failure(value, false); }
    finally { busy = false; }
  }
  async function claim() {
    if (busy) return;
    if (!validSetupName(operator) || !validSetupName(workspace)) { error = 'Enter your name and a workspace name, each at most 256 bytes without control characters.'; return; }
    busy = true; error = '';
    try {
      const result = await setupRequest('claim', {code, operator: operator.trim(), workspace: workspace.trim()});
      keys = setupKeys(result); code = ''; step = 'keys';
    } catch (value) { failure(value, true); }
    finally { busy = false; }
  }
  function keyCopied(ok: boolean) {
    error = ok ? '' : 'Clipboard access failed. The key is shown and selected; copy it manually before leaving.';
    copied = ok ? 'Key copied. Save it privately before continuing.' : '';
  }
  function open(role: 'workspace_owner_key' | 'instance_admin_key') {
    if (!acknowledged || !keys || busy) return;
    location.replace('/admin#key=' + encodeURIComponent(keys[role].secret));
  }
  onMount(() => {
    const warn = (event: BeforeUnloadEvent) => { if (keys && !acknowledged) { event.preventDefault(); event.returnValue = ''; } };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  });
  const panel = 'w-full max-w-md overflow-hidden rounded-xl border bg-card/90 shadow-[0_1px_2px_rgb(0_0_0/0.04),0_12px_32px_-12px_rgb(0_0_0/0.18)] backdrop-blur-sm dark:shadow-[0_0_0_1px_rgb(255_255_255/0.02),0_16px_40px_-16px_rgb(0_0_0/0.9)]';
  const marker = 'flex size-6 shrink-0 items-center justify-center rounded-md border font-mono text-[11px] transition-colors duration-300 [&_svg]:size-3.5';
</script>

{#snippet alert()}
  {#if error}
    <p role="alert" class="flex items-start gap-1.5 text-sm text-destructive"><CircleAlert class="mt-0.5 size-4 shrink-0" aria-hidden="true" />{error}</p>
  {/if}
{/snippet}

<CenteredPage {busy}>
  <header class="rise flex flex-col items-center gap-4 text-center">
    <div class="relative">
      <div class="login-halo pointer-events-none absolute -inset-10 -z-10" aria-hidden="true"></div>
      <Logo class="size-11 drop-shadow-[0_8px_18px_rgb(0_0_0/0.18)]" motion={busy ? 'loading' : 'once'} />
    </div>
    <div>
      <h1 id="setup-title" tabindex="-1" class="text-2xl font-semibold tracking-tight outline-none">
        {step === 'uncertain' ? 'Check whether setup finished' : step === 'complete' ? 'CCP is already set up' : 'Set up CCP'}
      </h1>
      <p class="mt-1 font-mono text-xs text-muted-foreground">{host}</p>
    </div>
  </header>

  {#if index >= 0}
    <ol class={['rise rise-2 divide-y', panel]} aria-label="Setup steps">
      {#each STEPS as name, position (name)}
        {@const done = position < index}
        {@const current = position === index}
        <li aria-current={current ? 'step' : undefined}>
          <div class="flex min-h-14 items-center gap-3 px-5 py-3.5">
            <span class={[marker, done ? 'border-foreground bg-foreground text-background' : current ? 'border-foreground text-foreground' : 'text-muted-foreground']} aria-hidden="true">
              {#if done}<Check />{:else}{String(position + 1).padStart(2, '0')}{/if}
            </span>
            <h2 id={`setup-step-${position}`} tabindex="-1" class={['min-w-0 flex-1 text-sm outline-none', current ? 'font-semibold' : done ? 'font-medium' : 'text-muted-foreground']}>
              {name}<span class="sr-only">{done ? ', done' : current ? ', current step' : ''}</span>
            </h2>
            {#if done}
              <span class="max-w-[45%] truncate text-xs text-muted-foreground">{summary[position]}</span>
              {#if position === 0 && step === 'details'}
                <button type="button" disabled={busy} onclick={() => { step = 'code'; error = ''; }} aria-label="Change setup code"
                  class="rounded text-xs font-medium underline-offset-4 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring">Change</button>
              {/if}
            {/if}
          </div>

          {#if current && step === 'code'}
            <form class="rise grid gap-4 px-5 pb-5" onsubmit={event => { event.preventDefault(); void verify(); }}>
              <div class="grid gap-2">
                <label for="setup-code" class="sr-only">Setup code</label>
                <Input id="setup-code" type="password" bind:value={code} required autocomplete="off" spellcheck={false} disabled={busy}
                  placeholder="ccp_setup_…" class="h-10 font-mono" aria-describedby="setup-code-hint" aria-invalid={error ? true : undefined} />
                <p id="setup-code-hint" class="text-xs text-muted-foreground">
                  {prefilled ? 'Filled in from your setup link.' : 'From your server log, after “Open:”.'} It expires after an hour; restarting the server prints a new one.
                </p>
              </div>
              {@render alert()}
              <Button type="submit" class="group h-10 w-full" {busy}>Continue <ArrowRight class="transition-transform duration-200 group-hover:translate-x-0.5" /></Button>
            </form>
          {:else if current && step === 'details'}
            <form class="rise grid gap-4 px-5 pb-5" onsubmit={event => { event.preventDefault(); void claim(); }}>
              <div class="grid gap-2">
                <label for="setup-workspace" class="text-sm font-medium">Workspace name</label>
                <Input id="setup-workspace" bind:value={workspace} required maxlength={256} disabled={busy} autocomplete="off" placeholder="Research" class="h-10" />
              </div>
              <div class="grid gap-2">
                <label for="setup-operator" class="text-sm font-medium">Your name</label>
                <Input id="setup-operator" bind:value={operator} required maxlength={256} disabled={busy} autocomplete="name" aria-describedby="setup-operator-hint" class="h-10" />
                <p id="setup-operator-hint" class="text-xs text-muted-foreground">Shown in audit records. You'll own the workspace and administer this server.</p>
              </div>
              {@render alert()}
              <Button type="submit" class="group h-10 w-full" {busy}>Create workspace <ArrowRight class="transition-transform duration-200 group-hover:translate-x-0.5" /></Button>
            </form>
          {:else if current && step === 'keys' && keys}
            <div class="rise grid gap-4 px-5 pb-5">
              <p class="flex items-start gap-2 text-sm text-muted-foreground">
                <ShieldAlert class="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
                Shown once. Anyone with a key has its access, so keep them somewhere private.
              </p>
              <SecretField id="setup-owner-key" label="Workspace owner key" note={workspace.trim()} prefix="ccp_admin_" value={keys.workspace_owner_key.secret} oncopy={keyCopied}
                hint="Edits context and manages clients and admins in this workspace." />
              <SecretField id="setup-instance-key" label="Instance admin key" note="This server" prefix="ccp_admin_" value={keys.instance_admin_key.secret} oncopy={keyCopied}
                hint="Creates workspaces and administers the server. It can't edit workspace context." />
              <p role="status" class="text-sm text-muted-foreground empty:hidden">{copied}</p>
              {@render alert()}
              <label class="flex cursor-pointer items-center gap-2.5 rounded-lg border px-3 py-2.5 text-sm transition-colors has-checked:border-foreground">
                <input type="checkbox" bind:checked={acknowledged} class="size-4 accent-primary" /> I've saved both keys somewhere private.
              </label>
              <div class="grid gap-2">
                <Button class="group h-10 w-full" disabled={!acknowledged} onclick={() => open('workspace_owner_key')}>
                  Open {workspace.trim()} <ArrowRight class="transition-transform duration-200 group-hover:translate-x-0.5" />
                </Button>
                <Button variant="ghost" class="w-full" disabled={!acknowledged} onclick={() => open('instance_admin_key')}>Open instance administration</Button>
              </div>
            </div>
          {/if}
        </li>
      {/each}
    </ol>
    <p class="rise rise-3 max-w-md text-center text-xs text-muted-foreground">
      {#if step === 'keys'}Lost a key later? Reset it on the server host with <code class="font-mono">ccp-server reset-owner-key</code> or <code class="font-mono">reset-instance-key</code>.
      {:else}Setup runs once. The keys it creates are the only way into this server.{/if}
    </p>
  {:else if step === 'uncertain'}
    <div class={['rise rise-2 grid gap-4 p-5', panel]}>
      <p class="text-sm text-muted-foreground">The request may have completed before the response was lost. Checking doesn't create another owner. If setup finished without delivering the keys, the host operator must recover access.</p>
      {@render alert()}
      <Button class="h-10 w-full" {busy} onclick={() => void verify()}>Check setup state</Button>
    </div>
  {:else}
    <div class={['rise rise-2 grid gap-4 p-5', panel]}>
      <p class="text-sm text-muted-foreground">Sign in with a saved admin key. If the first response was lost, stop the server and use the local key-reset commands. Damaged or missing admin roles need a matching backup.</p>
      {@render alert()}
      <a href="/admin" class="inline-flex h-10 w-full items-center justify-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow-xs outline-none hover:bg-primary/90 focus-visible:ring-2 focus-visible:ring-ring">Go to sign in</a>
    </div>
  {/if}
</CenteredPage>
