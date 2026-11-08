<script lang="ts">
  import {PanelLeft, RefreshCw, ChevronRight, Radio, Pause} from '@lucide/svelte';
  import Button from '../lib/ui/Button.svelte';
  import Appearance from './Appearance.svelte';
  import {app} from '../lib/app.svelte';
  let {onmenu, onrefresh}: {onmenu: () => void; onrefresh: () => void} = $props();
  const labels: Record<string, string> = {
    overview: 'Overview', library: 'Library', activity: 'Activity', agents: 'Agents', instructions: 'Master instructions',
    sessions: 'Sessions', live: 'Live activity', 'all-agents': 'All agents', 'global-instructions': 'Global instructions',
  };
  const scoped = $derived(app.current && !app.instanceView);
</script>
<header class="sticky top-0 z-30 flex h-14 shrink-0 items-center gap-2 border-b bg-background/85 px-4 backdrop-blur supports-[backdrop-filter]:bg-background/70">
  <Button variant="ghost" size="icon-sm" class="lg:hidden" aria-label="Open navigation" onclick={onmenu}><PanelLeft /></Button>
  <nav aria-label="Breadcrumb" class="min-w-0 flex-1">
    <ol class="flex items-center gap-1.5 text-sm text-muted-foreground">
      {#if scoped}
        <li class="truncate">{app.session}</li>
        <li aria-hidden="true"><ChevronRight class="size-3.5" /></li>
      {/if}
      <li class="truncate font-medium text-foreground" aria-current="page">{labels[app.view]}</li>
    </ol>
  </nav>
  <Button variant="ghost" size="sm" aria-pressed={app.live} onclick={() => app.toggleLive()}
    title={app.live ? 'Live updates on: activity and agents refresh automatically' : 'Live updates paused'}
    class={app.live ? 'text-foreground' : 'text-muted-foreground'}>
    {#if app.live}<Radio class="text-success" />{:else}<Pause />{/if}<span class="hidden sm:inline">{app.live ? 'Live' : 'Paused'}</span>
  </Button>
  <Button variant="ghost" size="icon-sm" aria-label="Refresh" busy={app.working('refresh')} onclick={onrefresh} class="text-muted-foreground"><RefreshCw /></Button>
  <Appearance />
  {#if app.inflight > 0}
    <div class="progress-bar pointer-events-none absolute inset-x-0 -bottom-px h-0.5 overflow-hidden" aria-hidden="true"><span class="block h-full w-2/5 rounded-full bg-primary"></span></div>
  {/if}
</header>
