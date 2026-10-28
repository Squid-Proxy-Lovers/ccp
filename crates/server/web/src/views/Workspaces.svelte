<script lang="ts">
  // Instance-admin workspace list, paged 32 at a time, with workspace creation.
  // The new owner key is shown once and kept only in memory until dismissed.
  import {Building2, ChevronLeft, ChevronRight, Plus} from '@lucide/svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Card from '../lib/ui/Card.svelte';
  import CardContent from '../lib/ui/CardContent.svelte';
  import CardHeader from '../lib/ui/CardHeader.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Secret from '../lib/ui/Secret.svelte';
  import {table} from '../lib/ui/table';
  import DeletionLookup from './settings/DeletionLookup.svelte';
  import {app} from '../lib/app.svelte';
  import {notice} from '../lib/notify.svelte';
  import type {Workspace} from '../lib/types';

  const FORM = 'create-workspace';
  const draft = app.formDraft(FORM);
  draft.name ??= '';
  draft.owner ??= '';

  function page(cursor: string | null, key: string) {
    void app.run(async () => { if (app.mayLeave()) await app.loadWorkspaces(cursor); }, key);
  }

  function open(workspace: Workspace) {
    void app.run(async () => { if (app.mayLeave()) await app.openWorkspace(workspace); }, `open:${workspace.id}`);
  }

  function create(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      if (!app.mayLeave()) return;
      await app.createWorkspace(draft.name ?? '', draft.owner ?? '');
      app.dropFormDraft(FORM);
      if (!app.locked) notice('Workspace created. Copy its owner key privately before navigating away.', 'success');
    }, FORM);
  }
</script>

<div class="space-y-6">
  <PageHeader title="Workspaces"
    description="Workspaces on this instance. An instance key reads workspace context but can't edit it; sign in with a workspace key to edit.">
    {#snippet meta()}<Badge tone="outline">Instance admin</Badge>{/snippet}
  </PageHeader>

  {#if app.createdWorkspace}
    {@const created = app.createdWorkspace}
    <Secret title={`Workspace owner key for ${created.workspace_id}`} value={created.owner_key}
      ondismiss={() => (app.createdWorkspace = null)} />
  {/if}

  <Card class="pb-0">
    <CardHeader title="All workspaces" description="Open a workspace to read its context, access and settings." />
    {#if !app.workspaces.length}
      <CardContent class="pb-5">
        <Empty icon={Building2} title="No accessible workspaces" description={app.workspacesAfter ? 'This page is empty. Return to the first page.' : 'Create a workspace below to get started.'} />
      </CardContent>
    {:else}
      <div class={table.wrap}>
        <table class={table.table}>
          <thead class={table.head}>
            <tr>
              <th scope="col" class={table.headCell}>Name</th>
              <th scope="col" class={table.headCell}>State</th>
              <th scope="col" class={table.headCell}>Workspace ID</th>
              <th scope="col" class={table.headCell}><span class="sr-only">Actions</span></th>
            </tr>
          </thead>
          <tbody class={table.body}>
            {#each app.workspaces as workspace (workspace.id)}
              <tr class={table.row}>
                <td class={[table.cell, 'font-medium']}>
                  <span class="flex items-center gap-2">
                    {workspace.name}
                    {#if workspace.id === app.workspace?.id}<Badge tone="info">Current</Badge>{/if}
                  </span>
                </td>
                <td class={table.cell}>
                  {#if workspace.archived}<Badge tone="warning">Archived</Badge>{:else}<Badge tone="success">Active</Badge>{/if}
                </td>
                <td class={[table.cell, 'font-mono text-xs text-muted-foreground']}>{workspace.id}</td>
                <td class={[table.cell, 'text-right']}>
                  <Button size="sm" variant="outline" busy={app.working(`open:${workspace.id}`)} onclick={() => open(workspace)}>
                    Open<span class="sr-only"> {workspace.name}</span> <ChevronRight />
                  </Button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
    {#if app.workspacesNext || app.workspacesAfter}
      <div class="flex items-center justify-end gap-2 border-t px-5 py-3">
        <Button size="sm" variant="outline" disabled={!app.workspacesAfter} busy={app.working('workspaces-first')} onclick={() => page(null, 'workspaces-first')}><ChevronLeft /> First</Button>
        <Button size="sm" variant="outline" disabled={!app.workspacesNext} busy={app.working('workspaces-next')} onclick={() => page(app.workspacesNext, 'workspaces-next')}>Next <ChevronRight /></Button>
      </div>
    {:else if app.workspaces.length}
      <div class="pb-5"></div>
    {/if}
  </Card>

  <Card>
    <CardHeader title="Create workspace"
      description="Creates a new owner principal and shows its workspace key once. Your instance key stays separate." />
    <CardContent>
      <form class="grid gap-4 md:grid-cols-[1fr_1fr_auto] md:items-end" onsubmit={create}>
        <Field id="create-workspace-name" label="Workspace name">
          <Input id="create-workspace-name" bind:value={draft.name} required autocomplete="off" />
        </Field>
        <Field id="create-workspace-owner" label="New owner principal name">
          <Input id="create-workspace-owner" bind:value={draft.owner} required autocomplete="off" />
        </Field>
        <Button type="submit" busy={app.working(FORM)}><Plus /> Create workspace</Button>
      </form>
      <p class="mt-3 text-[13px] text-muted-foreground">If the response is lost, refresh the list before trying again so you don't create a duplicate.</p>
    </CardContent>
  </Card>

  <Card>
    <CardContent><DeletionLookup /></CardContent>
  </Card>
</div>
