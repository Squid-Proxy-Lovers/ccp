<script lang="ts">
  // Agent status rows. A status is free text set by the agent; JSON is shown
  // as a short summary that expands to the full, formatted document.
  import {ChevronDown, Bot} from '@lucide/svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import WorkspaceAvatar from '../../components/WorkspaceAvatar.svelte';
  import {prettyJson, seconds} from '../../lib/api';
  import {relative, stamp} from '../../lib/notify.svelte';
  import type {AgentStatus} from '../../lib/types';

  let {items, showSession = false, compact = false}: {items: AgentStatus[]; showSession?: boolean; compact?: boolean} = $props();
  let open = $state<Record<string, boolean>>({});

  const key = (agent: AgentStatus) => `${agent.session_name}\u0000${agent.team}\u0000${agent.agent_name}\u0000${agent.worker_id}`;

  /** One readable line: `message`/`status`/`task` fields first, else the start of the text. */
  function summary(text: string): string {
    const {json} = prettyJson(text);
    if (json) {
      try {
        const value = JSON.parse(text);
        if (value && typeof value === 'object' && !Array.isArray(value)) {
          for (const field of ['message', 'status', 'state', 'task', 'summary', 'phase', 'note']) {
            if (typeof value[field] === 'string' && value[field].trim()) return value[field];
          }
          return Object.keys(value).slice(0, 6).join(' · ');
        }
      } catch { /* fall through */ }
    }
    return text.replace(/\s+/g, ' ').trim();
  }
</script>

<ul class="-my-1 divide-y">
  {#each items as agent (key(agent))}
    {@const id = key(agent)}
    {@const updated = seconds(agent.updated_at)}
    {@const expires = seconds(agent.expires_at)}
    {@const expanded = open[id] ?? false}
    {@const body = prettyJson(agent.status)}
    <li class="flex gap-3 py-3">
      <span class="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground" aria-hidden="true"><Bot class="size-3.5" /></span>
      <div class="min-w-0 flex-1 space-y-1">
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
          <span class="font-medium">{agent.agent_name}</span>
          {#if agent.team}<Badge tone="outline" class="font-normal">{agent.team}</Badge>{/if}
          {#if showSession}
            <span class="flex items-center gap-1.5 text-muted-foreground"><WorkspaceAvatar id={agent.session_name} class="size-4" />{agent.session_name}</span>
          {/if}
        </div>
        <p class="text-xs text-muted-foreground">
          <span class="font-mono">{agent.worker_id}</span>
          · updated <time datetime={new Date(updated * 1000).toISOString()} title={stamp(updated)}>{relative(updated)}</time>
          · expires <time datetime={new Date(expires * 1000).toISOString()} title={stamp(expires)}>{relative(expires)}</time>
        </p>
        {#if agent.status}
          {#if compact}
            <p class="line-clamp-2 text-sm break-words text-foreground/90">{summary(agent.status)}</p>
          {:else}
            <button type="button" onclick={() => (open[id] = !expanded)} aria-expanded={expanded}
              class="group block w-full cursor-pointer rounded-md border bg-muted/30 px-3 py-2 text-left outline-none hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring">
              {#if expanded}
                <pre class="max-h-[28rem] overflow-auto font-mono text-xs break-words whitespace-pre-wrap text-foreground/90">{body.pretty}</pre>
              {:else}
                <span class="line-clamp-2 block text-sm break-words text-foreground/90">{summary(agent.status)}</span>
              {/if}
              <span class="mt-1 flex items-center gap-1 text-[11px] text-muted-foreground group-hover:text-foreground">
                <ChevronDown class={['size-3 transition-transform', expanded && 'rotate-180']} />{expanded ? 'Collapse' : body.json ? 'Show full status (JSON)' : 'Show full status'}
              </span>
            </button>
          {/if}
        {/if}
      </div>
    </li>
  {/each}
</ul>
