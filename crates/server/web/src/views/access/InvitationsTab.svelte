<script lang="ts">
  // Owner/admin invitation inventory plus the three ways to admit a client:
  // the single initial group, an individual invitation, or targeted readmission.
  import {onMount} from 'svelte';
  import {Mail} from '@lucide/svelte';
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
  import {stamp} from '../../lib/notify.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {invitationPage} from '../../lib/model.js';
  import InviteForm from './InviteForm.svelte';
  import InitialGroupForm from './InitialGroupForm.svelte';
  import ReadmitCard from './ReadmitCard.svelte';
  import {changeAdmission, FormDraft, roleLabel, type Enrollment, type Grant, type Invitation, type Report} from './actions';

  let {enrollment, grants, configuring = $bindable(false), onoutcome}: {
    enrollment: Enrollment | null; grants: Grant[]; configuring?: boolean; onoutcome: Report;
  } = $props();

  let page = $state.raw<{invitations: Invitation[]; next_cursor: string | null} | null>(null);
  let after = $state<string | null>(null);
  let error = $state('');
  const workspace = $derived(app.workspace!);
  const initialOpen = $derived(enrollment?.phase === 'open');
  const groupDraft = new FormDraft(`initial-group:${app.workspace?.id}`);

  async function read(cursor: string | null) {
    try {
      page = invitationPage(await app.request('/api/v1/admin/invitations/list', {workspace_id: workspace.id, after: cursor, limit: 16}));
      after = cursor;
      error = '';
    } catch (failure) {
      if (app.locked) return;
      error = (failure as Error).message;
    }
  }
  onMount(() => { void read(null); });

  function turn(cursor: string | null, key: string) {
    void app.run(async () => { if (app.mayLeave()) await read(cursor); }, key);
  }

  function revoke(item: Invitation) {
    const id = item.status.invitation_id;
    void app.run(async () => {
      if (!app.mayLeave()) return;
      if (!await confirmAction({title: 'Revoke invitation?', confirm: 'Revoke invitation', destructive: true,
        body: `Revoke invitation ${id} in ${workspace.name}? Existing client grants and content are preserved. Revoking an initial-group invitation also closes its initial window.`})) return;
      await changeAdmission(onoutcome, workspace, 'revoke', {workspace_id: workspace.id, invitation_id: id});
    }, `revoke-invitation:${id}`);
  }

  const kindLabel = (item: Invitation) => item.kind === 'initial_group' ? 'Initial group' : item.status.target_client_id ? 'Readmission' : 'Individual';
</script>

<div class="space-y-6">
  {#if enrollment?.can_open}
    {#if configuring || groupDraft.touched}
      <InitialGroupForm {enrollment} onclose={() => (configuring = false)} {onoutcome} />
    {:else}
      <Card>
        <CardHeader title="Initial group invitation"
          description="One reusable group invitation for the workspace’s first clients. The phase is single-use: once closed, it cannot reopen." action={groupAction} />
      </Card>
    {/if}
  {/if}

  {#if app.writable() && !initialOpen}
    <InviteForm unused={enrollment?.phase === 'unused'} {onoutcome} />
  {:else}
    <Alert title={initialOpen ? 'Individual invitations wait for the initial group' : 'New client admission is paused'}>
      <p>{initialOpen ? 'Close initial group enrollment before issuing individual invitations.' : 'New client admission is paused while archived.'}</p>
    </Alert>
  {/if}

  {#if !workspace.archived}
    <ReadmitCard {grants} {onoutcome} />
  {/if}

  <Card>
    <CardHeader title="Invitations"
      description="Recorded invitation metadata, not a permission lease or proof that a particular request created a row. Secrets are shown only when created. Empty inventory does not mean initial group enrollment is available."
      action={readAction} />
    <CardContent class="px-0">
      {#if error}
        <div class="px-5">
          <Alert tone="destructive" title="Invitation metadata unavailable">
            <p>{error}</p>
            <p>No empty inventory or usable invitation was assumed.{page ? ' The table below is a previous observation.' : ''}</p>
          </Alert>
          <Button class="mt-3" variant="outline" busy={app.working('invitations-retry')} onclick={() => turn(after, 'invitations-retry')}>Read invitation metadata</Button>
        </div>
      {/if}
      {#if !page && !error}
        <Delayed><SkeletonTable rows={3} columns={['w-32', 'w-20', 'w-28', 'w-16']} label="Loading invitations" /></Delayed>
      {:else if page && !page.invitations.length}
        <div class="px-5"><Empty icon={Mail} title="No invitations on this page" /></div>
      {:else if page}
        <div class={[table.wrap, error && 'mt-4 opacity-70']}>
          <table class={table.table}>
            <thead class={table.head}>
              <tr>
                <th scope="col" class={table.headCell}>Invitation and requested access</th>
                <th scope="col" class={table.headCell}>Recorded state</th>
                <th scope="col" class={table.headCell}><span class="sr-only">Actions</span></th>
              </tr>
            </thead>
            <tbody class={table.body}>
              {#each page.invitations as item (item.status.invitation_id)}
                {@const status = item.status}
                <tr class={table.row}>
                  <td class={table.cell}>
                    <div class="flex flex-wrap items-center gap-1.5">
                      <span class="font-medium">{kindLabel(item)}</span>
                      <Badge tone={item.role === 'read_write' ? 'info' : 'neutral'}>{roleLabel(item.role)} requested</Badge>
                    </div>
                    <div class="font-mono text-xs text-muted-foreground">{status.invitation_id}</div>
                    {#if status.target_client_id}<div class="font-mono text-xs text-muted-foreground">Target client {status.target_client_id}</div>{/if}
                  </td>
                  <td class={table.cell}>
                    <div class="tabular">{status.uses} uses / {status.max_uses === null ? 'No count limit' : status.max_uses} · {status.expires_at === null ? 'No deadline' : 'Deadline ' + stamp(status.expires_at)}</div>
                    <div class="mt-1 flex flex-wrap gap-1.5">
                      <Badge tone={status.revoked ? 'destructive' : 'outline'}>{status.revoked ? 'Revoked' : 'Not revoked'}</Badge>
                      <Badge tone={status.expired ? 'warning' : 'outline'}>{status.expired ? 'Expiry observed' : 'Expiry not observed'}</Badge>
                      <Badge tone={status.issuer_authorized ? 'outline' : 'warning'}>{status.issuer_authorized ? 'Issuer currently authorized' : 'Issuer currently not authorized'}</Badge>
                      {#if item.kind === 'initial_group'}
                        <Badge tone="outline">{status.initial_closed ? 'Initial window closed' : 'Initial window not observed closed'}</Badge>
                      {/if}
                    </div>
                  </td>
                  <td class={table.cell}>
                    {#if !status.revoked}
                      <div class="flex justify-end">
                        <Button size="sm" variant="outline" class="text-destructive hover:text-destructive" busy={app.working(`revoke-invitation:${status.invitation_id}`)}
                          onclick={() => revoke(item)} aria-label={`Revoke invitation ${status.invitation_id}`}>Revoke</Button>
                      </div>
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
      {#if page && (page.next_cursor || after)}
        <div class="flex gap-2 px-5 pt-4">
          {#if after}<Button size="sm" variant="outline" busy={app.working('invitations-first')} onclick={() => turn(null, 'invitations-first')}>First invitations</Button>{/if}
          {#if page.next_cursor}<Button size="sm" variant="outline" busy={app.working('invitations-next')} onclick={() => turn(page?.next_cursor ?? null, 'invitations-next')}>Next invitations</Button>{/if}
        </div>
      {/if}
      <p class="px-5 pt-4 text-[13px] text-muted-foreground">Issuer authority is a current check: it is false while archived or after the issuer loses permission or its key. It is separate from permanent invitation revocation. Pages are separate current observations, ordered by UUID.</p>
    </CardContent>
  </Card>
</div>

{#snippet groupAction()}
  <Button variant="outline" onclick={() => (configuring = true)}>Configure initial group</Button>
{/snippet}
{#snippet readAction()}
  <Button variant="outline" size="sm" busy={app.working('invitations-read')} onclick={() => turn(null, 'invitations-read')}>Read current invitations</Button>
{/snippet}
