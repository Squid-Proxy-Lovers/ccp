<script lang="ts">
  import {LayoutDashboard, BookOpen, Activity, Bot, ScrollText, Globe, Layers, Radio,
    ChevronsUpDown, Check, LogOut, KeyRound} from '@lucide/svelte';
  import Menu from '../lib/ui/Menu.svelte';
  import MenuItem from '../lib/ui/MenuItem.svelte';
  import Logo from './Logo.svelte';
  import WorkspaceAvatar from './WorkspaceAvatar.svelte';
  import {app} from '../lib/app.svelte';
  import {whole} from '../views/overview/types';
  import type {View, SessionView} from '../lib/types';

  let {onnavigate}: {onnavigate?: () => void} = $props();

  const sessionNav: {view: SessionView; label: string; icon: typeof LayoutDashboard}[] = [
    {view: 'overview', label: 'Overview', icon: LayoutDashboard},
    {view: 'library', label: 'Library', icon: BookOpen},
    {view: 'activity', label: 'Activity', icon: Activity},
    {view: 'agents', label: 'Agents', icon: Bot},
    {view: 'instructions', label: 'Master instructions', icon: ScrollText},
  ];
  const instanceNav: {view: View; label: string; icon: typeof LayoutDashboard}[] = [
    {view: 'sessions', label: 'Sessions', icon: Layers},
    {view: 'live', label: 'Live activity', icon: Radio},
    {view: 'all-agents', label: 'All agents', icon: Bot},
    {view: 'global-instructions', label: 'Global instructions', icon: Globe},
  ];
  const sessionAgents = $derived(app.agents.filter(agent => agent.session_name === app.session).length);

  const navItem = 'flex h-8 w-full cursor-pointer items-center gap-2.5 rounded-md px-2 text-left text-sm outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring aria-[current=page]:bg-sidebar-accent aria-[current=page]:font-medium [&_svg]:size-4 [&_svg]:text-muted-foreground aria-[current=page]:[&_svg]:text-primary';

  function go(view: View) {
    void app.run(async () => { await app.navigate(view); onnavigate?.(); }, `nav:${view}`);
  }
  function choose(name: string, close: () => void) {
    close();
    void app.run(async () => { await app.openSession(name); onnavigate?.(); }, 'switch');
  }
</script>

<div class="flex h-full flex-col gap-2 bg-sidebar p-2 text-sidebar-foreground">
  <Menu label="Switch session" class="max-h-[70vh] w-[15.5rem] overflow-y-auto"
    triggerClass="flex w-full items-center gap-2.5 rounded-lg p-2 text-left hover:bg-sidebar-accent aria-expanded:bg-sidebar-accent">
    {#snippet trigger()}
      {#if app.current}<WorkspaceAvatar id={app.current.session.session_name} class="size-8" />{:else}<Logo class="size-8 shrink-0" />{/if}
      <span class="grid min-w-0 flex-1 leading-tight">
        <span class="truncate text-sm font-semibold">{app.current?.session.session_name ?? 'CCP'}</span>
        <span class="truncate text-xs text-muted-foreground">{app.current ? `Session · ${whole.format(app.current.entries)} entries` : 'Choose a session'}</span>
      </span>
      <ChevronsUpDown class="size-4 shrink-0 text-muted-foreground" />
    {/snippet}
    {#snippet children(close)}
      <p class="px-2 pt-1.5 pb-1 text-xs font-medium text-muted-foreground">Sessions</p>
      {#each app.sessions as item (item.session.session_id)}
        <MenuItem onselect={() => choose(item.session.session_name, close)}>
          <WorkspaceAvatar id={item.session.session_name} class="size-6" />
          <span class="min-w-0 flex-1 truncate">{item.session.session_name}</span>
          {#if item.session.session_name === app.session}<Check />{/if}
        </MenuItem>
      {:else}
        <p class="px-2 py-1.5 text-sm text-muted-foreground">No sessions yet.</p>
      {/each}
      <div class="mt-1 border-t pt-1">
        <MenuItem onselect={() => { close(); go('sessions'); }}><Layers /> Manage sessions</MenuItem>
      </div>
    {/snippet}
  </Menu>

  <nav aria-label="Main" class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pt-2">
    {#if app.current}
      <div class="grid gap-0.5">
        <p class="px-2 pb-1 text-xs font-medium text-muted-foreground">Session</p>
        {#each sessionNav as item (item.view)}
          <button type="button" aria-current={app.view === item.view ? 'page' : undefined}
            onclick={() => void app.run(async () => { await app.openSession(app.session as string, item.view); onnavigate?.(); }, `nav:${item.view}`)}
            class={navItem}>
            <item.icon /> <span class="flex-1">{item.label}</span>
            {#if item.view === 'agents' && sessionAgents}<span class="tabular text-xs text-muted-foreground">{sessionAgents}</span>{/if}
          </button>
        {/each}
      </div>
    {/if}
    <div class="grid gap-0.5">
      <p class="px-2 pb-1 text-xs font-medium text-muted-foreground">Server</p>
      {#each instanceNav as item (item.view)}
        <button type="button" aria-current={app.view === item.view ? 'page' : undefined} onclick={() => go(item.view)} class={navItem}>
          <item.icon /> <span class="flex-1">{item.label}</span>
          {#if item.view === 'sessions'}<span class="tabular text-xs text-muted-foreground">{app.sessions.length}</span>{/if}
          {#if item.view === 'all-agents' && app.agents.length}<span class="tabular text-xs text-muted-foreground">{app.agents.length}</span>{/if}
        </button>
      {/each}
    </div>
  </nav>

  <Menu label="Account" side="top" class="right-0 p-0"
    triggerClass="flex w-full items-center gap-2.5 rounded-lg p-2 text-left hover:bg-sidebar-accent aria-expanded:bg-sidebar-accent">
    {#snippet trigger()}
      <span class="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"><KeyRound class="size-4" /></span>
      <span class="grid min-w-0 flex-1 leading-tight">
        <span class="truncate text-sm font-medium">Server admin</span>
        <span class="truncate font-mono text-xs text-muted-foreground">{app.keyHint}</span>
      </span>
      <ChevronsUpDown class="size-4 shrink-0 text-muted-foreground" />
    {/snippet}
    {#snippet children(close)}
      <dl class="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-4 gap-y-1.5 px-3 py-2.5 text-xs">
        <dt class="text-muted-foreground">Key</dt>
        <dd class="truncate text-right font-mono">{app.keyHint}</dd>
        <dt class="text-muted-foreground">Stored</dt>
        <dd class="text-right">{app.remembered ? 'On this browser' : 'This tab only'}</dd>
      </dl>
      <div class="border-t p-1">
        <MenuItem onselect={() => { close(); window.location.assign('/admin/classic'); }}><LayoutDashboard /> Classic panel</MenuItem>
        <MenuItem onselect={() => { close(); void app.run(async () => { if (await app.mayLeave()) app.signOut(); }, 'signout'); }}><LogOut /> Sign out</MenuItem>
      </div>
    {/snippet}
  </Menu>
</div>
