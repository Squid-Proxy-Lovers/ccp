<script lang="ts">
  // One client's recovery history in the key's explicit scope (instance-wide or
  // this workspace), with recovery issuance, revocation and targeted readmission.
  import {onMount} from 'svelte';
  import {ArrowLeft, History} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Empty from '../../lib/ui/Empty.svelte';
  import Delayed from '../../lib/ui/Delayed.svelte';
  import SkeletonTable from '../../lib/ui/skeleton/SkeletonTable.svelte';
  import {table} from '../../lib/ui/table';
  import {app} from '../../lib/app.svelte';
  import {notice, stamp} from '../../lib/notify.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {canManageInvitations, clientRecoveryScope} from '../../lib/model.js';
  import ClientAccessForm from './ClientAccessForm.svelte';
  import {changeClientAccess, readRecovery, readmissionBlocked, scopeLabel, type Grant, type RecoveryItem, type RecoveryScope, type Report} from './actions';

  let {client, onback, onoutcome}: {client: Grant; onback: () => Promise<void>; onoutcome: Report} = $props();

  const workspace = $derived(app.workspace!);
  const scope = $derived(clientRecoveryScope(app.session, app.workspace) as RecoveryScope);
  let page = $state.raw<{invitations: RecoveryItem[]; next_cursor: string | null} | null>(null);
  let after = $state<string | null>(null);
  let error = $state('');
  let editor = $state<'recover' | 'readmit' | null>(null);

  async function read(cursor: string | null) {
    try {
      page = await readRecovery(client.client_id, scope, cursor);
      after = cursor;
      error = '';
    } catch (failure) {
      if (app.locked) return;
      error = (failure as Error).message;
    }
  }
  onMount(() => { void read(null); });

  function reload(cursor: string | null, key: string, announce = false) {
    void app.run(async () => {
      await read(cursor);
      if (announce && !error) notice('Loaded client recovery history in the explicit scope.');
    }, key);
  }

  function leave() {
    void app.run(async () => { if (app.mayLeave()) await onback(); }, 'client-back');
  }

  function revoke(item: RecoveryItem) {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      await changeClientAccess(onoutcome, workspace, client, {action: 'revoke_recovery', invitation_id: item.invitation_id}, () => confirmAction({
        title: 'Revoke recovery invitation?', confirm: 'Revoke invitation', destructive: true,
        body: `Revoke recovery invitation ${item.invitation_id} for client ${client.client_id}? This blocks redemption and receipt retrieval. It does not revoke an already-issued credential or membership.`,
      }));
    }, `revoke-recovery:${item.invitation_id}`);
  }

  function edit(action: 'recover' | 'readmit') {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      if (action === 'readmit' && await readmissionBlocked(workspace.id)) return;
      editor = action;
    }, `client-edit:${action}`);
  }
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-center gap-2">
    <Button variant="outline" size="sm" busy={app.working('client-back')} onclick={leave}><ArrowLeft /> Back to clients</Button>
    <Button variant="outline" size="sm" busy={app.working('recovery-refresh')} onclick={() => reload(null, 'recovery-refresh', true)}>Read current recovery history</Button>
  </div>

  <Card>
    <CardHeader title={`Client recovery and readmission · ${client.client_name}`} description={scopeLabel(scope, workspace)} />
    <CardContent class="space-y-3">
      <p class="font-mono text-xs text-muted-foreground">{client.client_id}</p>
      <div class="flex flex-wrap gap-1.5">
        <Badge tone={client.revoked_at === null ? 'success' : 'destructive'}>Last read grant row: {client.revoked_at === null ? 'Current' : 'Revoked'}</Badge>
        {#if client.client_disabled}<Badge tone="warning">Client disabled instance-wide</Badge>{/if}
      </div>
      <p class="text-sm text-muted-foreground">This row and the history below are observations, not future authority.</p>
    </CardContent>
  </Card>

  {#if error}
    <Alert tone="destructive" title="Recovery history unavailable">
      <p>{error}</p>
      <p>No empty history or recoverable access was assumed.</p>
    </Alert>
    <Button variant="outline" busy={app.working('recovery-retry')} onclick={() => reload(after, 'recovery-retry')}>Read client recovery history</Button>
  {:else if !page}
    <Delayed><SkeletonTable flush rows={3} columns={['w-28', 'w-20', 'w-32']} label="Loading recovery history" /></Delayed>
  {:else}
    <Card>
      <CardHeader title="Recovery invitations"
        description="Invitation expiry and revocation are separate from credentials already issued. These flags do not prove the client or its captured grants can still be recovered. Redemption rechecks current authority. Pages are separate observations, and secrets cannot be retrieved." />
      <CardContent class="px-0">
        {#if !page.invitations.length}
          <div class="px-5"><Empty icon={History} title="No recovery invitations on this page in this exact scope" /></div>
        {:else}
          <div class={table.wrap}>
            <table class={table.table}>
              <thead class={table.head}>
                <tr><th scope="col" class={table.headCell}>Invitation</th><th scope="col" class={table.headCell}>Recorded state</th><th scope="col" class={table.headCell}><span class="sr-only">Actions</span></th></tr>
              </thead>
              <tbody class={table.body}>
                {#each page.invitations as item (item.invitation_id)}
                  <tr class={table.row}>
                    <td class={[table.cell, 'font-mono text-xs']}>{item.invitation_id}</td>
                    <td class={table.cell}>
                      <div>Deadline {stamp(item.status.expires_at)}</div>
                      <div class="mt-1 flex flex-wrap gap-1.5">
                        <Badge tone={item.status.revoked ? 'destructive' : 'outline'}>{item.status.revoked ? 'Revoked' : 'Not revoked'}</Badge>
                        <Badge tone={item.status.expired ? 'warning' : 'outline'}>{item.status.expired ? 'Expiry observed' : 'Expiry not observed'}</Badge>
                        <Badge tone={item.status.redeemed ? 'info' : 'outline'}>{item.status.redeemed ? 'Redeemed' : 'Not redeemed'}</Badge>
                        <Badge tone={item.status.issuer_authorized ? 'outline' : 'warning'}>{item.status.issuer_authorized ? 'Issuer currently authorized' : 'Issuer currently not authorized'}</Badge>
                      </div>
                    </td>
                    <td class={table.cell}>
                      {#if !item.status.revoked}
                        <div class="flex justify-end">
                          <Button size="sm" variant="outline" class="text-destructive hover:text-destructive" busy={app.working(`revoke-recovery:${item.invitation_id}`)}
                            onclick={() => revoke(item)} aria-label={`Revoke recovery invitation ${item.invitation_id}`}>Revoke</Button>
                        </div>
                      {/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
        {#if page.next_cursor || after}
          <div class="flex gap-2 px-5 pt-4">
            {#if after}<Button size="sm" variant="outline" busy={app.working('recovery-first')} onclick={() => reload(null, 'recovery-first')}>First recovery invitations</Button>{/if}
            {#if page.next_cursor}<Button size="sm" variant="outline" busy={app.working('recovery-next')} onclick={() => reload(page?.next_cursor ?? null, 'recovery-next')}>Next recovery invitations</Button>{/if}
          </div>
        {/if}
      </CardContent>
    </Card>
  {/if}

  {#if client.client_disabled}
    <Alert tone="warning" title="Client disabled instance-wide">
      <p>No fresh recovery or readmission is offered. Existing invitation history can still be revoked.</p>
    </Alert>
  {:else}
    <div class="space-y-3">
      {#if scope.kind === 'workspace' && client.revoked_at !== null}
        <p class="text-sm text-muted-foreground">This grant row is revoked. Select a current grant row for workspace recovery; recovery cannot restore revoked access.</p>
      {/if}
      {#if canManageInvitations(app.session, workspace) && workspace.archived}
        <p class="text-sm text-muted-foreground">Archived: recovery of still-authorized readable context remains available; new readmission requires reactivation.</p>
      {/if}
      <div class="flex flex-wrap gap-2">
        {#if scope.kind === 'instance' || client.revoked_at === null}
          <Button variant="outline" busy={app.working('client-edit:recover')} onclick={() => edit('recover')}>
            {scope.kind === 'instance' ? 'Issue instance recovery invitation' : 'Issue workspace recovery invitation'}
          </Button>
        {/if}
        {#if canManageInvitations(app.session, workspace) && !workspace.archived && client.revoked_at !== null}
          <Button variant="outline" busy={app.working('client-edit:readmit')} onclick={() => edit('readmit')}>Explicitly readmit client</Button>
        {/if}
      </div>
    </div>
    {#if editor}
      {#key editor}
        <ClientAccessForm {client} action={editor} onclose={() => (editor = null)} {onoutcome} />
      {/key}
    {/if}
  {/if}
</div>
