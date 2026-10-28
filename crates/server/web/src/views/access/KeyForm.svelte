<script lang="ts">
  // Appoint an administrator (new principal or existing UUID) with a first key,
  // or issue an additional key for an existing administrator.
  import {onMount} from 'svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import {app} from '../../lib/app.svelte';
  import {notice} from '../../lib/notify.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {keyExpiry} from '../../lib/model.js';
  import {changeSecurity, FormDraft, type Admin, type Report} from './actions';

  let {admin = null, onclose, onoutcome}: {admin?: Admin | null; onclose: () => void; onoutcome: Report} = $props();
  const form = $derived(new FormDraft(admin ? `team-key:${app.workspace?.id}:${admin.id}` : `team-appoint:${app.workspace?.id}`, {mode: 'new_principal'}));
  const identities = [{value: 'new_principal', label: 'Create new principal'}, {value: 'existing', label: 'Existing principal ID'}];
  let first: HTMLInputElement | null = $state(null);
  onMount(() => first?.focus());

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      if (app.pending) throw new Error('Resolve the original security action first.');
      const workspace = app.workspace!;
      const expiry = keyExpiry(form.get('expires'));
      let body: Record<string, unknown>;
      if (admin) body = {workspace_id: workspace.id, principal_id: admin.id, name: form.get('name'), expires_at: expiry};
      else {
        const mode = form.get('mode'), principal = form.get('principal').trim();
        if (!['new_principal', 'existing'].includes(mode) || !principal) throw new Error('Choose an identity and enter its name or UUID.');
        if (mode === 'existing' && !/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(principal)) throw new Error('Enter the existing principal’s complete UUID.');
        body = {workspace_id: workspace.id, principal: mode === 'existing'
          ? {kind: 'existing', principal_id: principal} : {kind: 'new_principal', name: principal}, key_expires_at: expiry};
      }
      const ok = await confirmAction(admin
        ? {title: 'Create an additional key?', confirm: 'Create key', body: `Create an additional key for ${admin.name} (${admin.id})? Existing keys remain valid.`}
        : {title: 'Appoint workspace administrator?', confirm: 'Appoint and create key',
          body: 'Appoint this principal as a workspace administrator and create its first key? This permits client admission and data management in this workspace.'});
      if (!ok) return;
      await changeSecurity(onoutcome, admin ? 'keys/issue' : 'team/appoint', body);
      form.drop();
    }, 'key-form-submit');
  }

  async function discard() {
    if (!await confirmAction({title: 'Discard draft?', body: 'Discard this local security draft?', confirm: 'Discard draft', destructive: true})) return;
    form.drop();
    onclose();
    notice('Local security draft discarded.');
  }
</script>

<Card>
  <CardHeader title={admin ? 'Issue additional workspace key' : 'Appoint workspace administrator'}
    description={admin ? `${admin.name} · ${admin.id}. Issuing does not revoke existing keys.` : undefined} />
  <CardContent>
    <form class="grid max-w-xl gap-5" onsubmit={submit}>
      {#if admin}
        <Field id="key-name" label="New key label">
          <Input id="key-name" required bind:ref={first} bind:value={() => form.get('name'), value => form.set('name', value)} />
        </Field>
      {:else}
        <Field id="appoint-mode" label="Administrator identity">
          <Select id="appoint-mode" options={identities}
            bind:value={() => form.get('mode'), value => { form.set('mode', value); form.set('principal', ''); }} />
        </Field>
        <Field id="appoint-principal" label="Principal name or existing UUID"
          hint="Create a new principal by label, or explicitly enter an existing principal UUID. Matching names never select an existing identity.">
          <Input id="appoint-principal" required aria-describedby="appoint-principal-hint" bind:ref={first}
            bind:value={() => form.get('principal'), value => form.set('principal', value)} />
        </Field>
      {/if}
      <Field id="key-expires" label="Key expiry (optional, local time)"
        hint="Leave empty for valid until revoked. The secret is returned once; deliver it privately. A lost response cannot retrieve the secret.">
        <Input id="key-expires" type="datetime-local" step="1" aria-describedby="key-expires-hint"
          bind:value={() => form.get('expires'), value => form.set('expires', value)} />
      </Field>
      <div class="flex flex-wrap gap-2">
        <Button type="submit" busy={app.working('key-form-submit')}>{admin ? 'Create workspace key' : 'Appoint and create key'}</Button>
        <Button variant="ghost" onclick={discard}>Discard security draft</Button>
      </div>
    </form>
  </CardContent>
</Card>
