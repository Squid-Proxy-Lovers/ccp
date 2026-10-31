<script lang="ts">
  import {onMount, untrack, tick} from 'svelte';
  import Logo from './Logo.svelte';
  import Appearance from './Appearance.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Field from '../lib/ui/Field.svelte';
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
  let shownOwner = $state(false);
  let shownInstance = $state(false);
  $effect(() => {
    const currentStep = step;
    void tick().then(() => { if (currentStep !== 'code') document.getElementById('setup-title')?.focus(); });
  });
  let keys = $state<{instance_admin_key: {secret: string}; workspace_owner_key: {secret: string}} | null>(null);

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
    if (!validSetupName(operator) || !validSetupName(workspace)) { error = 'Enter an admin label and workspace name, each at most 256 bytes without control characters.'; return; }
    busy = true; error = '';
    try {
      const result = await setupRequest('claim', {code, operator: operator.trim(), workspace: workspace.trim()});
      keys = setupKeys(result); code = ''; step = 'keys';
    } catch (value) { failure(value, true); }
    finally { busy = false; }
  }
  async function copy(value: string, target: 'owner' | 'instance') {
    error = ''; copied = '';
    try { await navigator.clipboard.writeText(value); copied = 'Key copied. Save it privately before continuing.'; }
    catch {
      if (target === 'owner') shownOwner = true; else shownInstance = true;
      error = 'Clipboard access failed. This key is shown and selected; copy it manually before leaving.';
      await tick();
      const field = document.getElementById('setup-' + target + '-key');
      if (field instanceof HTMLInputElement) { field.focus(); field.select(); }
    }
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
</script>

<div class="min-h-svh bg-background text-foreground">
  <header class="flex h-16 items-center justify-between border-b px-6"><Logo class="size-8 shrink-0" /><Appearance /></header>
  <main class="mx-auto grid max-w-2xl gap-6 px-6 py-12" aria-busy={busy}>
    {#if error}<div role="alert" class="rounded-md border border-destructive/30 bg-destructive/5 p-4 text-sm">{error}</div>{/if}
    {#if step === 'code'}
      <div class="grid gap-2"><h1 id="setup-title" tabindex="-1" class="outline-none text-3xl font-semibold tracking-tight">Set up your CCP instance</h1><p class="text-muted-foreground">Use the setup link printed by your server. No administrator exists until this setup is completed.</p></div>
      <form class="grid gap-5" onsubmit={(event) => { event.preventDefault(); void verify(); }}>
        <Field id="setup-code" label="Setup code" hint="Find it in the Open link in your server or container logs. Codes expire after an hour; a restart prints a new one."><Input id="setup-code" aria-describedby="setup-code-hint" type="password" bind:value={code} required autocomplete="off" spellcheck="false" disabled={busy} /></Field>
        <div><Button type="submit" {busy}>Verify setup code</Button></div>
      </form>
    {:else if step === 'details'}
      <div class="grid gap-2"><h1 id="setup-title" tabindex="-1" class="outline-none text-3xl font-semibold tracking-tight">Create your first workspace</h1><p class="text-muted-foreground">The first operator becomes the instance administrator and this workspace’s owner. Their keys have separate scopes.</p></div>
      <form class="grid gap-5" onsubmit={(event) => { event.preventDefault(); void claim(); }}>
        <Field id="setup-operator" label="Your admin label" hint="A label for administration and audit records. This is not a password."><Input id="setup-operator" aria-describedby="setup-operator-hint" bind:value={operator} required maxlength={256} disabled={busy} /></Field>
        <Field id="setup-workspace" label="First workspace name"><Input id="setup-workspace" bind:value={workspace} required maxlength={256} disabled={busy} /></Field>
        <div class="flex gap-3"><Button type="submit" {busy}>Create instance and workspace</Button><Button variant="outline" disabled={busy} onclick={() => {step = 'code'; error = '';}}>Change setup code</Button></div>
      </form>
    {:else if step === 'keys' && keys}
      <div class="grid gap-2"><h1 id="setup-title" tabindex="-1" class="outline-none text-3xl font-semibold tracking-tight">Save your admin keys</h1><p class="text-muted-foreground">These keys are shown once. Save both privately before leaving. Anyone holding a key has its access.</p></div>
      <div class="grid gap-3 rounded-lg border p-5"><Field id="setup-owner-key" label="Workspace owner key" hint="Edit context and manage this workspace’s clients and admin team."><Input id="setup-owner-key" aria-describedby="setup-owner-key-hint" type={shownOwner ? 'text' : 'password'} value={keys.workspace_owner_key.secret} autocomplete="off" readonly /></Field><div><Button variant="outline" onclick={() => void copy(keys!.workspace_owner_key.secret, 'owner')}>Copy workspace owner key</Button><Button variant="ghost" aria-pressed={shownOwner} onclick={() => shownOwner = !shownOwner}>{shownOwner ? 'Hide' : 'Show'} workspace owner key</Button></div></div>
      <div class="grid gap-3 rounded-lg border p-5"><Field id="setup-instance-key" label="Instance admin key" hint="Create workspaces and administer the instance. Workspace writes use their scoped key."><Input id="setup-instance-key" aria-describedby="setup-instance-key-hint" type={shownInstance ? 'text' : 'password'} value={keys.instance_admin_key.secret} autocomplete="off" readonly /></Field><div><Button variant="outline" onclick={() => void copy(keys!.instance_admin_key.secret, 'instance')}>Copy instance admin key</Button><Button variant="ghost" aria-pressed={shownInstance} onclick={() => shownInstance = !shownInstance}>{shownInstance ? 'Hide' : 'Show'} instance admin key</Button></div></div>
      <p role="status" class="text-sm text-muted-foreground">{copied}</p>
      <label class="flex items-center gap-3 text-sm"><input type="checkbox" bind:checked={acknowledged} class="size-4 accent-primary" />I have saved both keys privately.</label>
      <div class="flex flex-wrap gap-3"><Button disabled={!acknowledged} onclick={() => open('workspace_owner_key')}>Open workspace panel</Button><Button variant="outline" disabled={!acknowledged} onclick={() => open('instance_admin_key')}>Open instance administration</Button></div>
    {:else if step === 'uncertain'}
      <h1 id="setup-title" tabindex="-1" class="outline-none text-3xl font-semibold tracking-tight">Check whether setup completed</h1>
      <p class="text-muted-foreground">The claim may have committed. Checking its state does not create another owner. If it completed without delivering the keys, the host operator must recover access.</p>
      <div><Button {busy} onclick={() => void verify()}>Check setup state</Button></div>
    {:else}
      <h1 id="setup-title" tabindex="-1" class="outline-none text-3xl font-semibold tracking-tight">This instance is already set up</h1>
      <p class="text-muted-foreground">Sign in with a saved admin key. If the first response was lost, stop the server and use the local key-reset commands. Damaged or missing admin roles require a matching backup.</p>
      <div><a href="/admin" class="font-medium underline underline-offset-4">Open sign-in</a></div>
    {/if}
  </main>
</div>
