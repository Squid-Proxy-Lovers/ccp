<script lang="ts">
  import {LayoutDashboard, BookOpen, KeyRound, ListChecks, ScrollText, Settings, Globe, Building2,
    ChevronsUpDown, Check, LogOut, ChevronLeft, ChevronRight, Archive} from '@lucide/svelte';
  import Menu from '../lib/ui/Menu.svelte';
  import MenuItem from '../lib/ui/MenuItem.svelte';
  import Logo from './Logo.svelte';
  import WorkspaceAvatar from './WorkspaceAvatar.svelte';
  import {app} from '../lib/app.svelte';
  import {stamp} from '../lib/notify.svelte';
  import type {View, Workspace} from '../lib/types';

  let {onnavigate}: {onnavigate?: () => void} = $props();

  const workspaceNav: {view: View; label: string; icon: typeof LayoutDashboard}[] = [
    {view: 'overview', label: 'Overview', icon: LayoutDashboard},
    {view: 'context', label: 'Context', icon: BookOpen},
    {view: 'access', label: 'Clients & access', icon: KeyRound},
    {view: 'reported', label: 'Reported work', icon: ListChecks},
    {view: 'guidance', label: 'Guidance', icon: ScrollText},
    {view: 'settings', label: 'Settings', icon: Settings},
  ];
  const identity = $derived(app.session?.identity);
  const role = $derived(identity?.scope.kind === 'instance' ? 'Instance admin' : `Workspace ${identity?.scope.role ?? ''}`);
  const initials = $derived((identity?.principal_name ?? '?').split(/\s+/).map(part => part[0]).join('').slice(0, 2).toUpperCase());

  const navItem = 'flex h-8 w-full cursor-pointer items-center gap-2.5 rounded-md px-2 text-left text-sm outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring aria-[current=page]:bg-sidebar-accent aria-[current=page]:font-medium [&_svg]:size-4 [&_svg]:text-muted-foreground aria-[current=page]:[&_svg]:text-primary';

  function go(view: View) {
    void app.run(async () => { await app.navigate(view); onnavigate?.(); }, `nav:${view}`);
  }
  function choose(workspace: Workspace, close: () => void) {
    close();
    void app.run(async () => {
      if (workspace.id !== app.workspace?.id && !app.mayLeave()) return;
      await app.openWorkspace(workspace);
      onnavigate?.();
    }, 'switch');
  }
  function page(cursor: string | null) {
    void app.run(() => app.loadWorkspaces(cursor), 'workspaces-page');
  }
</script>

<div class="flex h-full flex-col gap-2 bg-sidebar p-2 text-sidebar-foreground">
  <Menu label="Switch workspace" class="w-[15.5rem]"
    triggerClass="flex w-full items-center gap-2.5 rounded-lg p-2 text-left hover:bg-sidebar-accent aria-expanded:bg-sidebar-accent">
    {#snippet trigger()}
      {#if app.workspace}<WorkspaceAvatar id={app.workspace.id} class="size-8" />{:else}<Logo class="size-8 shrink-0" />{/if}
      <span class="grid min-w-0 flex-1 leading-tight">
        <span class="truncate text-sm font-semibold">{app.workspace?.name ?? 'CCP'}</span>
        <span class="truncate text-xs text-muted-foreground">{app.workspace ? (app.workspace.archived ? 'Archived workspace' : 'Workspace') : 'Choose a workspace'}</span>
      </span>
      <ChevronsUpDown class="size-4 shrink-0 text-muted-foreground" />
    {/snippet}
    {#snippet children(close)}
      <p class="px-2 pt-1.5 pb-1 text-xs font-medium text-muted-foreground">Workspaces</p>
      {#each app.workspaces as workspace (workspace.id)}
        <MenuItem onselect={() => choose(workspace, close)}>
          <WorkspaceAvatar id={workspace.id} class="size-6" />
          <span class="min-w-0 flex-1 truncate">{workspace.name}</span>
          {#if workspace.archived}<Archive />{/if}
          {#if workspace.id === app.workspace?.id}<Check />{/if}
        </MenuItem>
      {:else}
        <p class="px-2 py-1.5 text-sm text-muted-foreground">No accessible workspaces.</p>
      {/each}
      {#if app.workspacesNext || app.workspacesAfter}
        <div class="mt-1 flex gap-1 border-t pt-1">
          <MenuItem disabled={!app.workspacesAfter} onselect={() => page(null)}><ChevronLeft /> First</MenuItem>
          <MenuItem disabled={!app.workspacesNext} onselect={() => page(app.workspacesNext)}>Next <ChevronRight /></MenuItem>
        </div>
      {/if}
      {#if app.instanceScope}
        <div class="mt-1 border-t pt-1">
          <MenuItem onselect={() => { close(); go('workspaces'); }}><Building2 /> Manage workspaces</MenuItem>
        </div>
      {/if}
    {/snippet}
  </Menu>

  <nav aria-label="Main" class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pt-2">
    {#if app.workspace}
      <div class="grid gap-0.5">
        <p class="px-2 pb-1 text-xs font-medium text-muted-foreground">Workspace</p>
        {#each workspaceNav as item (item.view)}
          <button type="button" aria-current={app.view === item.view ? 'page' : undefined} onclick={() => go(item.view)}
            class={navItem}>
            <item.icon /> {item.label}
          </button>
        {/each}
      </div>
    {/if}
    <div class="grid gap-0.5">
      <p class="px-2 pb-1 text-xs font-medium text-muted-foreground">Instance</p>
      {#if app.instanceScope}
        <button type="button" aria-current={app.view === 'workspaces' ? 'page' : undefined} onclick={() => go('workspaces')}
          class={navItem}>
          <Building2 /> Workspaces
        </button>
      {/if}
      <button type="button" aria-current={app.view === 'global-guidance' ? 'page' : undefined} onclick={() => go('global-guidance')}
        class={navItem}>
        <Globe /> Global guidance
      </button>
    </div>
  </nav>

  {#if identity}
    <Menu label="Account" side="top" class="w-[15.5rem]"
      triggerClass="flex w-full items-center gap-2.5 rounded-lg p-2 text-left hover:bg-sidebar-accent aria-expanded:bg-sidebar-accent">
      {#snippet trigger()}
        <span class="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-xs font-semibold">{initials}</span>
        <span class="grid min-w-0 flex-1 leading-tight">
          <span class="truncate text-sm font-medium">{identity.principal_name}</span>
          <span class="truncate text-xs text-muted-foreground">{role}</span>
        </span>
        <ChevronsUpDown class="size-4 shrink-0 text-muted-foreground" />
      {/snippet}
      {#snippet children(close)}
        <div class="space-y-1 px-2 py-1.5 text-xs text-muted-foreground">
          <p><span class="text-foreground">{identity.key_name}</span> · {role}</p>
          {#if app.session}<p>Session ends {stamp(app.session.expires_at)}</p>{/if}
          <p class="font-mono break-all" title="Key and principal labels don't identify the individual using a shared key.">Key {identity.key_id}</p>
        </div>
        <div class="mt-1 border-t pt-1">
          <MenuItem onselect={() => { close(); void app.run(() => app.signOut(), 'signout'); }}><LogOut /> Sign out</MenuItem>
        </div>
      {/snippet}
    </Menu>
  {/if}
</div>
