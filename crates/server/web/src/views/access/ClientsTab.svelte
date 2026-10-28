<script lang="ts">
  // Grants in this workspace: workspace keys revoke a grant, instance keys
  // disable a client everywhere. Owners, admins and instance keys can open a
  // client's recovery and readmission history.
  import {Users} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Empty from '../../lib/ui/Empty.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import {table} from '../../lib/ui/table';
  import {app} from '../../lib/app.svelte';
  import {notice} from '../../lib/notify.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {canManageInvitations} from '../../lib/model.js';
  import ClientAccess from './ClientAccess.svelte';
  import {roleLabel, type Grant, type GrantPage, type Report} from './actions';

  let {page, after, error, onpage, onoutcome}: {
    page: GrantPage | null; after: string | null; error: string;
    onpage: (cursor: string | null) => Promise<void>; onoutcome: Report;
  } = $props();

  let selected = $state.raw<Grant | null>(null);
  const workspace = $derived(app.workspace!);
  const scopeKind = $derived(app.session!.identity.scope.kind);
  const recoverable = $derived(scopeKind === 'instance' || canManageInvitations(app.session, app.workspace));

  function open(grant: Grant) {
    void app.run(() => { if (app.mayLeave()) selected = grant; }, 'open-client');
  }

  function revoke(grant: Grant) {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      if (!await confirmAction({title: `Revoke ${grant.client_name}?`, confirm: 'Revoke access', destructive: true,
        body: `Revoke ${grant.client_name} in ${workspace.name}? Stored entries and other workspaces are preserved.`})) return;
      await app.request('/api/v1/admin/grants/revoke', {grant_id: grant.id});
      await onpage(after);
      notice('Workspace access revoked. Stored entries and ownership are preserved.', 'success');
    }, `revoke-grant:${grant.id}`);
  }

  function disable(grant: Grant) {
    void app.run(async () => {
      if (!await confirmAction({title: `Disable ${grant.client_name} instance-wide?`, confirm: 'Disable client', destructive: true,
        body: `Disable ${grant.client_name} across this entire CCP instance, including certificate renewal? Stored content is preserved.`})) return;
      await app.request('/api/v1/admin/clients/disable', {client_id: grant.client_id});
      await onpage(after);
      notice('Client disabled instance-wide. Stored content is preserved.', 'success');
    }, `disable-client:${grant.client_id}`);
  }

  function turn(cursor: string | null, key: string) {
    void app.run(async () => { if (app.mayLeave()) await onpage(cursor); }, key);
  }

  async function back() {
    selected = null;
    await app.inspectWorkspace();
    await onpage(after);
  }
</script>

{#if selected}
  <ClientAccess client={selected} onback={back} {onoutcome} />
{:else}
  <Card>
    <CardHeader title="Registered clients"
      description="Client identities and their grants in this workspace." />
    <CardContent class="px-0">
      {#if error}
        <div class="px-5">
          <Alert tone="destructive" title="Client grants unavailable">
            <p>{error}</p>
            <p>No empty list was assumed.</p>
          </Alert>
          <Button class="mt-3" variant="outline" busy={app.working('grants-retry')} onclick={() => turn(after, 'grants-retry')}>Read client grants</Button>
        </div>
      {:else if !page}
        <div class="space-y-2 px-5" aria-busy="true"><Skeleton class="h-9 w-full" /><Skeleton class="h-9 w-full" /><Skeleton class="h-9 w-2/3" /></div>
      {:else if !page.grants.length}
        <div class="px-5"><Empty icon={Users} title="No client grants on this page" /></div>
      {:else}
        <div class={table.wrap}>
          <table class={table.table}>
            <thead class={table.head}>
              <tr><th scope="col" class={table.headCell}>Client</th><th scope="col" class={table.headCell}>Workspace access</th><th scope="col" class={table.headCell}><span class="sr-only">Actions</span></th></tr>
            </thead>
            <tbody class={table.body}>
              {#each page.grants as grant (grant.id)}
                <tr class={table.row}>
                  <td class={table.cell}>
                    <div class="font-medium">{grant.client_name}</div>
                    <div class="font-mono text-xs text-muted-foreground">{grant.client_id}</div>
                  </td>
                  <td class={table.cell}>
                    <div class="flex flex-wrap gap-1.5">
                      {#if grant.revoked_at}<Badge tone="destructive">Revoked</Badge>
                      {:else}<Badge tone={grant.role === 'read_write' ? 'info' : 'neutral'}>{roleLabel(grant.role)}</Badge>{/if}
                      {#if grant.client_disabled}<Badge tone="warning">Disabled instance-wide</Badge>{/if}
                    </div>
                  </td>
                  <td class={table.cell}>
                    <div class="flex flex-wrap justify-end gap-2">
                      {#if recoverable}
                        <Button size="sm" variant="outline" busy={app.working('open-client')} onclick={() => open(grant)}
                          aria-label={`Recovery and readmission for ${grant.client_name}`}>Recovery & readmission</Button>
                      {/if}
                      {#if !grant.revoked_at && scopeKind === 'workspace'}
                        <Button size="sm" variant="outline" class="text-destructive hover:text-destructive" busy={app.working(`revoke-grant:${grant.id}`)}
                          onclick={() => revoke(grant)} aria-label={`Revoke workspace access for ${grant.client_name}`}>Revoke access</Button>
                      {/if}
                      {#if !grant.client_disabled && scopeKind === 'instance'}
                        <Button size="sm" variant="outline" class="text-destructive hover:text-destructive" busy={app.working(`disable-client:${grant.client_id}`)}
                          onclick={() => disable(grant)} aria-label={`Disable ${grant.client_name} instance-wide`}>Disable instance-wide</Button>
                      {/if}
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
      {#if page && (page.next_cursor || after)}
        <div class="flex gap-2 px-5 pt-4">
          {#if after}<Button size="sm" variant="outline" busy={app.working('grants-first')} onclick={() => turn(null, 'grants-first')}>First clients</Button>{/if}
          {#if page.next_cursor}<Button size="sm" variant="outline" busy={app.working('grants-next')} onclick={() => turn(page.next_cursor, 'grants-next')}>Next clients</Button>{/if}
        </div>
      {/if}
    </CardContent>
  </Card>
  {#if !canManageInvitations(app.session, app.workspace)}
    <Alert title="Invitations need a workspace owner or admin key">
      <p>Use this workspace’s owner or admin key to inspect or manage invitations and enrollment. Instance read access does not include this security inventory.</p>
    </Alert>
  {/if}
{/if}
