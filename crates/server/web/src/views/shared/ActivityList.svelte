<script lang="ts">
  // Activity rows: who wrote which entry, when, and the start of what they wrote.
  import {FilePlus2, FilePen, ChevronDown} from '@lucide/svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import WorkspaceAvatar from '../../components/WorkspaceAvatar.svelte';
  import {app} from '../../lib/app.svelte';
  import {seconds} from '../../lib/api';
  import {relative, stamp} from '../../lib/notify.svelte';
  import type {Activity} from '../../lib/types';

  let {items, showSession = false, clamp = 3}: {items: Activity[]; showSession?: boolean; clamp?: 2 | 3} = $props();
  let open = $state<Record<string, boolean>>({});

  function who(item: Activity): string {
    return [item.agent_name, item.host_name].filter(Boolean).join(' @ ') || item.actor;
  }
  function openEntry(item: Activity) {
    void app.run(() => app.openEntry(item.session_name, {shelf: item.shelf_name, book: item.book_name, entry: item.entry_name}), 'open-entry');
  }
</script>

<ul class="-my-1 divide-y">
  {#each items as item (item.id)}
    {@const at = seconds(item.created_at)}
    {@const expanded = open[item.id] ?? false}
    <li class="flex gap-3 py-3">
      <span class={['mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md', item.kind === 'entry_appended' ? 'bg-chart-2/15 text-chart-2' : 'bg-chart-1/15 text-chart-1']} aria-hidden="true">
        {#if item.kind === 'entry_appended'}<FilePen class="size-3.5" />{:else}<FilePlus2 class="size-3.5" />{/if}
      </span>
      <div class="min-w-0 flex-1 space-y-1">
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
          {#if showSession}
            <span class="flex items-center gap-1.5 font-medium"><WorkspaceAvatar id={item.session_name} class="size-4" />{item.session_name}</span>
            <span class="text-muted-foreground" aria-hidden="true">·</span>
          {/if}
          <button type="button" onclick={() => openEntry(item)} title="Open in library"
            class="min-w-0 cursor-pointer truncate rounded-sm font-mono text-[13px] outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring">
            <span class="text-muted-foreground">{item.shelf_name}/{item.book_name}/</span>{item.entry_name}
          </button>
          <Badge tone="outline" class="font-normal">{item.kind === 'entry_appended' ? 'Appended' : 'Created'}</Badge>
        </div>
        <p class="text-xs text-muted-foreground">
          <span class="font-medium text-foreground/80">{who(item)}</span>
          {#if item.reason}<span> · {item.reason}</span>{/if}
          · <time datetime={new Date(at * 1000).toISOString()} title={stamp(at)}>{relative(at)}</time>
        </p>
        {#if item.content}
          <button type="button" onclick={() => (open[item.id] = !expanded)} aria-expanded={expanded}
            class="group block w-full cursor-pointer rounded-md border bg-muted/30 px-3 py-2 text-left outline-none hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring">
            <span class={['block font-mono text-xs break-words whitespace-pre-wrap text-foreground/90', !expanded && (clamp === 2 ? 'line-clamp-2' : 'line-clamp-3')]}>{item.content}</span>
            <span class="mt-1 flex items-center gap-1 text-[11px] text-muted-foreground group-hover:text-foreground">
              <ChevronDown class={['size-3 transition-transform', expanded && 'rotate-180']} />{expanded ? 'Collapse' : 'Expand'}
            </span>
          </button>
        {/if}
      </div>
    </li>
  {/each}
</ul>
