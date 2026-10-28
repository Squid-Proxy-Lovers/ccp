<script lang="ts">
  import {tick} from 'svelte';
  import AppSidebar from './components/AppSidebar.svelte';
  import SiteHeader from './components/SiteHeader.svelte';
  import Toaster from './components/Toaster.svelte';
  import LoginView from './components/LoginView.svelte';
  import LockedView from './components/LockedView.svelte';
  import ConfirmHost from './lib/ui/ConfirmHost.svelte';
  import Overview from './views/Overview.svelte';
  import Context from './views/Context.svelte';
  import Access from './views/Access.svelte';
  import Reported from './views/Reported.svelte';
  import Guidance from './views/Guidance.svelte';
  import Settings from './views/Settings.svelte';
  import Workspaces from './views/Workspaces.svelte';
  import NoWorkspace from './views/NoWorkspace.svelte';
  import DeletionTracker from './views/DeletionTracker.svelte';
  import {app} from './lib/app.svelte';
  import {theme} from './lib/theme.svelte';

  let drawer = $state(false);
  let drawerDialog: HTMLDialogElement | undefined = $state();
  void theme;
  void app.boot();

  // Focus handoff: after any view, workspace, sign-in or lock change, move
  // focus to the new view's title so keyboard users keep their place.
  let previous = '';
  $effect(() => {
    const key = `${app.session ? 'in' : 'out'}:${app.locked}:${app.workspace?.id ?? ''}:${app.view}`;
    if (key === previous || !app.booted) return;
    const first = previous === '';
    previous = key;
    if (first && !app.session) return;
    void tick().then(() => document.getElementById('page-title')?.focus({preventScroll: false}));
  });
  $effect(() => {
    if (!drawerDialog) return;
    if (drawer && !drawerDialog.open) drawerDialog.showModal();
    else if (!drawer && drawerDialog.open) drawerDialog.close();
  });

  function refresh() {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      app.generation += 1;
      if (app.view === 'workspaces' || !app.workspace) await app.loadWorkspaces(app.workspacesAfter);
      else await app.inspectWorkspace();
      app.refreshTick += 1;
    }, 'refresh');
  }
</script>

{#if !app.booted && !app.session}
  <div class="flex min-h-svh items-center justify-center" aria-busy="true"><span class="sr-only">Loading</span></div>
{:else if app.locked}
  <LockedView />
{:else if !app.session}
  <LoginView />
{:else}
  <a href="#page-title" class="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:bg-background focus:px-3 focus:py-2 focus:ring-2 focus:ring-ring">Skip to content</a>
  <div class="flex min-h-svh bg-background">
    <aside class="hidden w-64 shrink-0 border-r border-sidebar-border bg-sidebar lg:block"><div class="sticky top-0 h-svh"><AppSidebar /></div></aside>
    <dialog bind:this={drawerDialog} onclose={() => (drawer = false)} aria-label="Navigation"
      class="m-0 h-svh max-h-svh w-72 max-w-[85vw] border-r bg-sidebar p-0 backdrop:bg-black/50 lg:hidden">
      {#if drawer}<AppSidebar onnavigate={() => (drawer = false)} />{/if}
    </dialog>
    <div class="flex min-w-0 flex-1 flex-col">
      <SiteHeader onmenu={() => (drawer = true)} onrefresh={refresh} />
      <main class="mx-auto w-full max-w-[1400px] flex-1 p-4 md:p-6 lg:p-8">
        <DeletionTracker class="mb-6" />
        {#key `${app.workspace?.id ?? ''}:${app.view}:${app.refreshTick}`}
          {#if app.view === 'workspaces'}<Workspaces />
          {:else if app.view === 'global-guidance'}<Guidance global />
          {:else if !app.workspace}<NoWorkspace />
          {:else if app.view === 'overview'}<Overview />
          {:else if app.view === 'context'}<Context />
          {:else if app.view === 'access'}<Access />
          {:else if app.view === 'reported'}<Reported />
          {:else if app.view === 'guidance'}<Guidance />
          {:else if app.view === 'settings'}<Settings />{/if}
        {/key}
      </main>
    </div>
  </div>
{/if}
<Toaster />
<ConfirmHost />
