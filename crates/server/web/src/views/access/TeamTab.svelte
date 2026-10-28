<script lang="ts">
  // Owner-only security administration: administrators and workspace keys.
  // A principal ID identifies the administrator; names and shared keys do not
  // prove which person uses them.
  import {onMount} from 'svelte';
  import {KeyRound, Users} from '@lucide/svelte';
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
  import {notice, stamp} from '../../lib/notify.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import KeyForm from './KeyForm.svelte';
  import {changeSecurity, type Admin, type Report, type WorkspaceKey} from './actions';

  let {onoutcome}: {onoutcome: Report} = $props();

  const workspace = $derived(app.workspace!);
  const currentKey = $derived(app.session?.identity.key_id);
  let admins = $state.raw<{admins: Admin[]; next_cursor: string | null} | null>(null);
  let keys = $state.raw<{keys: WorkspaceKey[]; next_cursor: string | null} | null>(null);
  let adminAfter = $state<string | null>(null);
  let keyAfter = $state<string | null>(null);
  let error = $state('');
  let editor = $state.raw<{kind: 'appoint'} | {kind: 'key'; admin: Admin} | null>(null);

  async function readAdmins(cursor: string | null) {
    admins = await app.request('/api/v1/admin/team/list', {workspace_id: workspace.id, after: cursor, limit: 16});
    adminAfter = cursor;
  }
  async function readKeys(cursor: string | null) {
    keys = await app.request('/api/v1/admin/keys/list', {workspace_id: workspace.id, after: cursor, limit: 16});
    keyAfter = cursor;
  }
  async function readAll(adminCursor: string | null, keyCursor: string | null) {
    try {
      await readAdmins(adminCursor);
      await readKeys(keyCursor);
      error = '';
    } catch (failure) {
      if (app.locked) return;
      error = (failure as Error).message;
    }
  }
  onMount(() => { void readAll(null, null); });

  function reload() {
    void app.run(async () => {
      if (app.pending) { notice('Read the current metadata in the pending action, then explicitly forget it before reloading.', 'warning'); return; }
      await readAll(adminAfter, keyAfter);
      if (!error) notice('Loaded administrator and key metadata.');
    }, 'team-read');
  }

  function turn(read: () => Promise<void>, key: string) {
    void app.run(async () => { if (app.mayLeave()) await read(); }, key);
  }

  function open(next: NonNullable<typeof editor>) {
    if (app.mayLeave()) editor = next;
  }

  function revokeAdmin(admin: Admin) {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      if (!await confirmAction({title: `Revoke ${admin.name}?`, confirm: 'Revoke administrator', destructive: true,
        body: `Revoke ${admin.name} (${admin.id}) in ${workspace.name}? All of this principal’s workspace keys are revoked; other workspaces and stored content are preserved.`})) return;
      await changeSecurity(onoutcome, 'team/revoke', {workspace_id: workspace.id, principal_id: admin.id});
    }, `revoke-admin:${admin.id}`);
  }

  function revokeKey(key: WorkspaceKey) {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      const own = key.id === currentKey;
      if (!await confirmAction({title: own ? 'Revoke your current login key?' : `Revoke key ${key.name}?`, confirm: 'Revoke key', destructive: true,
        body: `Revoke key ${key.name} (${key.id}) in ${workspace.name}?${own ? ' This is your current login key. This browser session will lose authority; have another owner key available.' : ''} The principal’s other keys and role remain unchanged.`})) return;
      await changeSecurity(onoutcome, 'keys/revoke', {workspace_id: workspace.id, key_id: key.id});
    }, `revoke-key:${key.id}`);
  }

  const danger = 'text-destructive hover:text-destructive';
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <p class="max-w-2xl text-sm text-muted-foreground">Owner-only security administration for this workspace. A principal ID identifies the registered administrator; names and shared keys do not prove which person is using them.</p>
    <div class="flex flex-wrap gap-2">
      <Button variant="outline" size="sm" busy={app.working('team-read')} onclick={reload}>Read current administrators & keys</Button>
      {#if !workspace.archived}<Button size="sm" onclick={() => open({kind: 'appoint'})}>Appoint administrator</Button>{/if}
    </div>
  </div>

  {#if workspace.archived}
    <Alert tone="warning" title="Archived workspace">
      <p>Key issuance, key revocation and administrator revocation remain available. New administrator appointment requires reactivation.</p>
    </Alert>
  {/if}

  {#if editor}
    {#key editor}
      {#if editor.kind === 'appoint'}<KeyForm onclose={() => (editor = null)} {onoutcome} />
      {:else}<KeyForm admin={editor.admin} onclose={() => (editor = null)} {onoutcome} />{/if}
    {/key}
  {/if}

  {#if error}
    <Alert tone="destructive" title="Administrator and key metadata unavailable">
      <p>{error}</p>
      <p>No empty roster or key list was assumed.</p>
    </Alert>
    <Button variant="outline" busy={app.working('team-retry')} onclick={() => turn(() => readAll(adminAfter, keyAfter), 'team-retry')}>Read administrators & keys</Button>
  {/if}

  <Card>
    <CardHeader title="Administrators" />
    <CardContent class="px-0">
      {#if !admins}
        {#if !error}<div class="space-y-2 px-5" aria-busy="true"><Skeleton class="h-9 w-full" /><Skeleton class="h-9 w-2/3" /></div>{/if}
      {:else if !admins.admins.length}
        <div class="px-5"><Empty icon={Users} title="No administrators on this page" /></div>
      {:else}
        <div class={table.wrap}>
          <table class={table.table}>
            <thead class={table.head}>
              <tr><th scope="col" class={table.headCell}>Principal</th><th scope="col" class={table.headCell}>Role</th><th scope="col" class={table.headCell}><span class="sr-only">Actions</span></th></tr>
            </thead>
            <tbody class={table.body}>
              {#each admins.admins as admin (admin.id)}
                <tr class={table.row}>
                  <td class={table.cell}>
                    <div class="font-medium">{admin.name}</div>
                    <div class="font-mono text-xs text-muted-foreground">{admin.id}</div>
                  </td>
                  <td class={table.cell}>
                    <div class="flex flex-wrap gap-1.5">
                      <Badge tone={admin.is_owner ? 'info' : 'neutral'}>{admin.is_owner ? 'Owner' : 'Administrator'}</Badge>
                      {#if !admin.enabled}<Badge tone="warning">Principal disabled</Badge>{/if}
                    </div>
                  </td>
                  <td class={table.cell}>
                    <div class="flex flex-wrap justify-end gap-2">
                      {#if admin.enabled}
                        <Button size="sm" variant="outline" onclick={() => open({kind: 'key', admin})} aria-label={`Issue key for ${admin.name}`}>Issue key</Button>
                      {/if}
                      {#if !admin.is_owner}
                        <Button size="sm" variant="outline" class={danger} busy={app.working(`revoke-admin:${admin.id}`)} onclick={() => revokeAdmin(admin)}
                          aria-label={`Revoke administrator ${admin.name}`}>Revoke</Button>
                      {/if}
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
      {#if admins && (admins.next_cursor || adminAfter)}
        <div class="flex gap-2 px-5 pt-4">
          {#if adminAfter}<Button size="sm" variant="outline" busy={app.working('admins-first')} onclick={() => turn(() => readAdmins(null), 'admins-first')}>First administrators</Button>{/if}
          {#if admins.next_cursor}<Button size="sm" variant="outline" busy={app.working('admins-next')} onclick={() => turn(() => readAdmins(admins?.next_cursor ?? null), 'admins-next')}>Next administrators</Button>{/if}
        </div>
      {/if}
    </CardContent>
  </Card>

  <Card>
    <CardHeader title="Workspace keys"
      description="Metadata contains no secret. Role activity does not mean an expired or revoked key works. Creating a key does not revoke older keys; deliver the new key privately before deliberately revoking an old one." />
    <CardContent class="px-0">
      {#if !keys}
        {#if !error}<div class="space-y-2 px-5" aria-busy="true"><Skeleton class="h-9 w-full" /><Skeleton class="h-9 w-2/3" /></div>{/if}
      {:else if !keys.keys.length}
        <div class="px-5"><Empty icon={KeyRound} title="No workspace keys on this page" /></div>
      {:else}
        <div class={table.wrap}>
          <table class={table.table}>
            <thead class={table.head}>
              <tr>
                <th scope="col" class={table.headCell}>Key</th><th scope="col" class={table.headCell}>Principal ID</th>
                <th scope="col" class={table.headCell}>Recorded state</th><th scope="col" class={table.headCell}><span class="sr-only">Actions</span></th>
              </tr>
            </thead>
            <tbody class={table.body}>
              {#each keys.keys as key (key.id)}
                <tr class={table.row}>
                  <td class={table.cell}>
                    <div class="flex flex-wrap items-center gap-1.5">
                      <span class="font-medium">{key.name}</span>
                      {#if key.id === currentKey}<Badge tone="info">Current login key</Badge>{/if}
                    </div>
                    <div class="font-mono text-xs text-muted-foreground">{key.id}</div>
                  </td>
                  <td class={[table.cell, 'font-mono text-xs']}>{key.principal_id}</td>
                  <td class={table.cell}>
                    <div>{key.revoked_at === null ? 'Not revoked' : 'Revoked ' + stamp(key.revoked_at)} · {key.expires_at === null ? 'No expiry' : 'Expires ' + stamp(key.expires_at)}</div>
                    <div class="mt-1"><Badge tone={key.role_active ? 'outline' : 'warning'}>{key.role_active ? 'Principal’s scoped role active' : 'Principal’s scoped role inactive'}</Badge></div>
                  </td>
                  <td class={table.cell}>
                    {#if key.revoked_at === null}
                      <div class="flex justify-end">
                        <Button size="sm" variant="outline" class={danger} busy={app.working(`revoke-key:${key.id}`)} onclick={() => revokeKey(key)}
                          aria-label={`Revoke key ${key.name}`}>Revoke</Button>
                      </div>
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
      {#if keys && (keys.next_cursor || keyAfter)}
        <div class="flex gap-2 px-5 pt-4">
          {#if keyAfter}<Button size="sm" variant="outline" busy={app.working('keys-first')} onclick={() => turn(() => readKeys(null), 'keys-first')}>First keys</Button>{/if}
          {#if keys.next_cursor}<Button size="sm" variant="outline" busy={app.working('keys-next')} onclick={() => turn(() => readKeys(keys?.next_cursor ?? null), 'keys-next')}>Next keys</Button>{/if}
        </div>
      {/if}
      <p class="px-5 pt-4 text-[13px] text-muted-foreground">Pages are separate current observations, ordered by ID.</p>
    </CardContent>
  </Card>
</div>
