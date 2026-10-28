<script lang="ts">
  // Recovery (same client UUID, still-authorized grants) or targeted readmission
  // (a new grant after every prior one was revoked) for one registered client.
  import {onMount} from 'svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import {app} from '../../lib/app.svelte';
  import {notice, stamp} from '../../lib/notify.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {clientRecoveryScope, invitationDeadline} from '../../lib/model.js';
  import {changeClientAccess, FormDraft, roleLabel, roles, scopeLabel, type Grant, type RecoveryScope, type Report} from './actions';

  let {client, action, onclose, onoutcome}: {
    client: Grant; action: 'recover' | 'readmit'; onclose: () => void; onoutcome: Report;
  } = $props();

  const workspace = $derived(app.workspace!);
  const scope = $derived(clientRecoveryScope(app.session, app.workspace) as RecoveryScope);
  const readmit = $derived(action === 'readmit');
  const form = $derived(new FormDraft(`client-access:${app.workspace?.id}:${client.client_id}:${action}`, {role: 'read', hours: '24'}));
  const id = $derived(`client-access-${action}`);
  let hoursInput: HTMLInputElement | null = $state(null);
  onMount(() => hoursInput?.focus());

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      if (app.pending) throw new Error('Resolve the original client access action first.');
      const expires_at = invitationDeadline(form.get('hours'));
      const role = readmit ? form.get('role') : undefined;
      const change = {action, expires_at, role};
      const sent = await changeClientAccess(onoutcome, workspace, client, change, () => confirmAction({
        title: readmit ? 'Issue explicit new membership?' : 'Issue credential recovery?',
        body: `${readmit ? 'Issue explicit new membership' : 'Issue credential recovery'} for ${client.client_id}? Scope: ${scopeLabel(scope, workspace)}.${role ? ' New role: ' + roleLabel(role) + '.' : ''} Deadline: ${stamp(expires_at)}.`,
        confirm: readmit ? 'Create readmission invitation' : 'Create recovery invitation',
      }));
      if (sent) form.drop();
    }, `client-access-submit:${action}`);
  }

  async function discard() {
    if (!await confirmAction({title: 'Discard draft?', body: 'Discard this local client access draft?', confirm: 'Discard draft', destructive: true})) return;
    form.drop();
    onclose();
    notice('Local client access draft discarded.');
  }
</script>

<Card>
  <CardHeader title={readmit ? 'Explicit targeted readmission' : 'Recover existing client credentials'}
    description={`Target client ${client.client_name} · ${client.client_id}. ${scopeLabel(scope, workspace)}.`} />
  <CardContent>
    <form class="grid max-w-xl gap-5" onsubmit={submit}>
      <Alert tone="warning">
        {#if readmit}
          <p>Creates a new grant with the chosen role for this same registered client. Every prior grant in this workspace must be revoked; the server rechecks complete history. Any later admission makes this pending invitation stale, even if that later grant is revoked. It preserves old revocations and unrelated access, and does not select the workspace.</p>
          <p>Issuing this targeted readmission invitation permanently closes an unused initial group phase, even if the invitation is never redeemed or is later revoked. Use a fresh client key.</p>
        {:else}
          <p>Recovery preserves this client UUID and captures only still-authorized grants in this scope at issuance. Later grants are not included; grants revoked before redemption are excluded. It never restores revoked access or bypasses a disabled client.</p>
          <p>Issuance alone does not rotate credentials. On redemption, older credential bindings for captured grants are replaced; unrelated workspace bindings remain. Use a fresh key and a separate empty recovery profile.</p>
        {/if}
      </Alert>
      {#if !readmit}
        <p class="text-sm text-muted-foreground">Issuing another recovery invitation does not revoke earlier ones. Deliver the secret privately; revoke undelivered invitations deliberately.</p>
      {/if}
      {#if readmit}
        <Field id={`${id}-role`} label="Readmission data access">
          <Select id={`${id}-role`} options={roles} bind:value={() => form.get('role'), value => form.set('role', value)} />
        </Field>
      {/if}
      <Field id={`${id}-hours`} label="Invitation lifetime (hours)" hint="1 to 168 whole hours.">
        <Input id={`${id}-hours`} type="number" min="1" max="168" step="1" required aria-describedby={`${id}-hours-hint`}
          bind:ref={hoursInput} bind:value={() => form.get('hours'), value => form.set('hours', value)} />
      </Field>
      <div class="flex flex-wrap gap-2">
        <Button type="submit" busy={app.working(`client-access-submit:${action}`)}>
          {readmit ? 'Create targeted readmission invitation' : 'Create recovery invitation'}
        </Button>
        <Button variant="ghost" onclick={discard}>Discard draft</Button>
      </div>
    </form>
  </CardContent>
</Card>
