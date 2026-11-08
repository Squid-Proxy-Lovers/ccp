<script lang="ts">
  import {tick} from 'svelte';
  import AppSidebar from './components/AppSidebar.svelte';
  import SiteHeader from './components/SiteHeader.svelte';
  import Toaster from './components/Toaster.svelte';
  import LoginView from './components/LoginView.svelte';
  import LockedView from './components/LockedView.svelte';
  import ConfirmHost from './lib/ui/ConfirmHost.svelte';
  import Sessions from './views/Sessions.svelte';
  import Overview from './views/Overview.svelte';
  import Library from './views/Library.svelte';
  import Activity from './views/Activity.svelte';
  import Agents from './views/Agents.svelte';
  import Instructions from './views/Instructions.svelte';
  import {app} from './lib/app.svelte';
  import {theme} from './lib/theme.svelte';

  let drawer = $state(false);
  let drawerDialog: HTMLDialogElement | undefined = $state();
  void theme;
  void app.boot();

  // Focus handoff: after a view, session, sign-in or lock change, move focus to
  // the new view's title so keyboard users keep their place.
  let previous = '';
  $effect(() => {
    const key = `${app.signedIn ? 'in' : 'out'}:${app.locked}:${app.session ?? ''}:${app.view}`;
    if (key === previous || !app.booted) return;
    const first = previous === '';
    previous = key;
    if (first && !app.signedIn) return;
    void tick().then(() => document.getElementById('page-title')?.focus({preventScroll: false}));
  });
  $effect(() => {
    if (!drawerDialog) return;
    if (drawer && !drawerDialog.open) drawerDialog.showModal();
    else if (!drawer && drawerDialog.open) drawerDialog.close();
  });

  // Keep the session list, agent statuses and global instructions current.
  $effect(() => {
    if (!app.signedIn || app.locked || !app.live) return;
    const timer = setInterval(() => {
      if (document.visibilityState === 'visible' && !app.busy) void app.loadOverview().catch(() => {});
    }, 15000);
    return () => clearInterval(timer);
  });

  const scoped = $derived(!app.instanceView && app.session !== null && app.current !== null);
</script>

{#if !app.booted}
  <div class="min-h-svh" role="status" aria-busy="true"><span class="sr-only">Loading</span></div>
{:else if app.locked}
  <LockedView />
{:else if !app.signedIn}
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
      <SiteHeader onmenu={() => (drawer = true)} onrefresh={() => void app.run(() => app.refresh(), 'refresh')} />
      <main class="mx-auto w-full max-w-[1400px] flex-1 p-4 md:p-6 lg:p-8">
        {#key `${app.session ?? ''}:${app.view}:${app.refreshTick}`}
          {#if app.view === 'sessions'}<Sessions />
          {:else if app.view === 'live'}<Activity all />
          {:else if app.view === 'all-agents'}<Agents all />
          {:else if app.view === 'global-instructions'}<Instructions global />
          {:else if !scoped}<Sessions />
          {:else if app.view === 'overview'}<Overview />
          {:else if app.view === 'library'}<Library />
          {:else if app.view === 'activity'}<Activity />
          {:else if app.view === 'agents'}<Agents />
          {:else if app.view === 'instructions'}<Instructions />{/if}
        {/key}
      </main>
    </div>
  </div>
{/if}
<Toaster />
<ConfirmHost />
