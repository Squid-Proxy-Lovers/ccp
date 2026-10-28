<script lang="ts">
  import {PanelLeft, RefreshCw, ChevronRight} from '@lucide/svelte';
  import Button from '../lib/ui/Button.svelte';
  import ThemeToggle from './ThemeToggle.svelte';
  import {app} from '../lib/app.svelte';
  let {onmenu, onrefresh}: {onmenu: () => void; onrefresh: () => void} = $props();
  const labels: Record<string, string> = {
    overview: 'Overview', context: 'Context', access: 'Clients & access', reported: 'Reported work',
    guidance: 'Guidance', settings: 'Settings', 'global-guidance': 'Global guidance', workspaces: 'Workspaces',
  };
  const scoped = $derived(app.workspace && app.view !== 'global-guidance' && app.view !== 'workspaces');
</script>
<header class="sticky top-0 z-30 flex h-14 shrink-0 items-center gap-2 border-b bg-background/85 px-4 backdrop-blur supports-[backdrop-filter]:bg-background/70">
  <Button variant="ghost" size="icon-sm" class="lg:hidden" aria-label="Open navigation" onclick={onmenu}><PanelLeft /></Button>
  <nav aria-label="Breadcrumb" class="min-w-0 flex-1">
    <ol class="flex items-center gap-1.5 text-sm text-muted-foreground">
      {#if scoped}
        <li class="truncate">{app.workspace?.name}</li>
        <li aria-hidden="true"><ChevronRight class="size-3.5" /></li>
      {/if}
      <li class="truncate font-medium text-foreground" aria-current="page">{labels[app.view]}</li>
    </ol>
  </nav>
  <Button variant="ghost" size="icon-sm" aria-label="Refresh" busy={app.working('refresh')} onclick={onrefresh} class="text-muted-foreground"><RefreshCw /></Button>
  <ThemeToggle />
</header>
