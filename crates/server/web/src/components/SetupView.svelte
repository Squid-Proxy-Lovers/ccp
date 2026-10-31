<script lang="ts">
  // First-run setup: verify the code the server printed, name the first workspace,
  // then show both admin keys once. Keys never touch browser storage.
  import {onMount, untrack, tick} from 'svelte';
  import {ArrowRight, CircleAlert} from '@lucide/svelte';
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
  $effect(() => {
    const currentStep = step;
    void tick().then(() => { if (currentStep !== 'code') document.getElementById('setup-title')?.focus(); });
  });

  const STEPS = ['Verify code', 'Name workspace', 'Save keys'];
  const index = $derived(step === 'code' ? 0 : step === 'details' ? 1 : step === 'keys' ? 2 : -1);
  const heading = $derived({
    code: ['Set up CCP', 'Confirm the setup code your server printed.'],
    details: ['Name your workspace', "You'll own it and administer this server."],
    keys: ['Save your keys', "They're shown once. Anyone with a key has its access."],
    uncertain: ['Check whether setup finished', 'The request may have completed before the response was lost.'],
    complete: ['CCP is already set up', 'Sign in with a saved admin key.'],
  }[step]);
  const prefilled = $derived(Boolean(initialCode) && code === initialCode);

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
  const card = 'w-full rounded-xl border bg-card/90 p-6 shadow-[0_1px_2px_rgb(0_0_0/0.04),0_12px_32px_-12px_rgb(0_0_0/0.18)] backdrop-blur-sm dark:shadow-[0_0_0_1px_rgb(255_255_255/0.02),0_16px_40px_-16px_rgb(0_0_0/0.9)]';
</script>

{#snippet alert()}
  {#if error}
    <p role="alert" class="flex items-start gap-1.5 text-sm text-destructive"><CircleAlert class="mt-0.5 size-4 shrink-0" aria-hidden="true" />{error}</p>
  {/if}
{/snippet}

<CenteredPage {busy}>
  <div class="rise flex flex-col items-center gap-5 text-center">
    <div class="relative">
      <div class="login-halo pointer-events-none absolute -inset-10 -z-10" aria-hidden="true"></div>
      <Logo class="size-14 drop-shadow-[0_8px_18px_rgb(0_0_0/0.18)]" motion={busy ? 'loading' : 'once'} />
    </div>
    <div>
      <h1 id="setup-title" tabindex="-1" class="text-2xl font-semibold tracking-tight outline-none">{heading[0]}</h1>
      <p class="mt-1.5 text-sm text-muted-foreground">{heading[1]}</p>
    </div>
    {#if index >= 0}
      <ol class="flex items-center gap-1.5" aria-label="Setup progress">
        {#each STEPS as name, position (name)}
          <li class={['h-1 w-8 rounded-full transition-colors duration-500', position <= index ? 'bg-foreground' : 'bg-border']}
            aria-current={position === index ? 'step' : undefined}>
            <span class="sr-only">Step {position + 1} of {STEPS.length}: {name}{position < index ? ', done' : ''}</span>
          </li>
        {/each}
      </ol>
    {/if}
  </div>

  {#if step === 'code'}
    <form class={['rise rise-2 grid max-w-sm gap-4', card]} onsubmit={event => { event.preventDefault(); void verify(); }}>
      <div class="grid gap-2">
        <label for="setup-code" class="text-sm font-medium">Setup code</label>
        <Input id="setup-code" type="password" bind:value={code} required autocomplete="off" spellcheck={false} disabled={busy}
          placeholder="ccp_setup_…" class="font-mono" aria-describedby="setup-code-hint" aria-invalid={error ? true : undefined} />
        <p id="setup-code-hint" class="text-xs text-muted-foreground">
          {prefilled ? 'Filled in from your setup link.' : 'In your server log, after “Open:”.'} Codes expire after an hour; restarting the server prints a new one.
        </p>
      </div>
      {@render alert()}
      <Button type="submit" class="group w-full" {busy}>Continue <ArrowRight class="transition-transform duration-200 group-hover:translate-x-0.5" /></Button>
    </form>
  {:else if step === 'details'}
    <form class={['rise grid max-w-sm gap-4', card]} onsubmit={event => { event.preventDefault(); void claim(); }}>
      <div class="grid gap-2">
        <label for="setup-operator" class="text-sm font-medium">Your name</label>
        <Input id="setup-operator" bind:value={operator} required maxlength={256} disabled={busy} autocomplete="name" aria-describedby="setup-operator-hint" />
        <p id="setup-operator-hint" class="text-xs text-muted-foreground">Shown in audit records. Not a password.</p>
      </div>
      <div class="grid gap-2">
        <label for="setup-workspace" class="text-sm font-medium">Workspace name</label>
        <Input id="setup-workspace" bind:value={workspace} required maxlength={256} disabled={busy} autocomplete="off" placeholder="Research" />
      </div>
      {@render alert()}
      <Button type="submit" class="group w-full" {busy}>Create workspace <ArrowRight class="transition-transform duration-200 group-hover:translate-x-0.5" /></Button>
      <button type="button" disabled={busy} onclick={() => { step = 'code'; error = ''; }}
        class="justify-self-center rounded text-xs text-muted-foreground underline-offset-4 outline-none hover:text-foreground hover:underline focus-visible:ring-2 focus-visible:ring-ring">
        Use a different setup code
      </button>
    </form>
  {:else if step === 'keys' && keys}
    <div class={['rise grid max-w-md gap-5', card]}>
      <SecretField id="setup-owner-key" label="Workspace owner key" value={keys.workspace_owner_key.secret} oncopy={keyCopied}
        hint={`Edit context and manage clients and admins in ${workspace.trim()}.`} />
      <SecretField id="setup-instance-key" label="Instance admin key" value={keys.instance_admin_key.secret} oncopy={keyCopied}
        hint="Create workspaces and administer this server. It can't edit workspace context." />
      <p role="status" class="text-sm text-muted-foreground empty:hidden">{copied}</p>
      {@render alert()}
      <label class="flex items-center gap-2.5 border-t pt-4 text-sm">
        <input type="checkbox" bind:checked={acknowledged} class="size-4 accent-primary" /> I've saved both keys somewhere private.
      </label>
      <div class="grid gap-2">
        <Button class="group w-full" disabled={!acknowledged} onclick={() => open('workspace_owner_key')}>
          Open {workspace.trim()} <ArrowRight class="transition-transform duration-200 group-hover:translate-x-0.5" />
        </Button>
        <Button variant="ghost" class="w-full" disabled={!acknowledged} onclick={() => open('instance_admin_key')}>Open instance administration</Button>
      </div>
    </div>
    <p class="rise rise-3 max-w-md text-center text-xs text-muted-foreground">Lost a key later? Reset it on the server host with <code class="font-mono">ccp-server reset-owner-key</code> or <code class="font-mono">reset-instance-key</code>.</p>
  {:else if step === 'uncertain'}
    <div class={['rise grid max-w-sm gap-4', card]}>
      <p class="text-sm text-muted-foreground">Checking doesn't create another owner. If setup finished without delivering the keys, the host operator must recover access.</p>
      {@render alert()}
      <Button class="w-full" {busy} onclick={() => void verify()}>Check setup state</Button>
    </div>
  {:else}
    <div class={['rise grid max-w-sm gap-4', card]}>
      <p class="text-sm text-muted-foreground">If the first response was lost, stop the server and use the local key-reset commands. Damaged or missing admin roles need a matching backup.</p>
      {@render alert()}
      <a href="/admin" class="inline-flex h-9 w-full items-center justify-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow-xs outline-none hover:bg-primary/90 focus-visible:ring-2 focus-visible:ring-ring">Go to sign in</a>
    </div>
  {/if}
</CenteredPage>
