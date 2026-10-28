<script lang="ts">
  // Original native records retained at conversion. Read-only observations,
  // validated before display; never treated as ownership or permission.
  import {onMount, tick} from 'svelte';
  import {Copy, RotateCcw} from '@lucide/svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Textarea from '../../lib/ui/Textarea.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import Delayed from '../../lib/ui/Delayed.svelte';
  import {table} from '../../lib/ui/table';
  import {app} from '../../lib/app.svelte';
  import {copyText, stamp} from '../../lib/notify.svelte';
  import {nativeHistoryPage, nativeScalarText} from '../../lib/native_values.js';
  import {focusId} from './content';

  interface Scalar { type: string; value?: unknown }
  interface SourceRecord { ordinal: number; source_table: string; source_key: string; row: {columns: string[]; values: Scalar[]} }
  interface SourcePage {
    workspace_id: string; entry_id: string; next_cursor: number | null; items: SourceRecord[];
    source: {database_sha256: string; schema: number; converted_at: number} | null;
  }
  const validate = nativeHistoryPage as unknown as (value: unknown, workspace: unknown, entry: string, after: number | null) => SourcePage;
  const labels: Record<string, string> = {
    message_packs: 'Original entry row', message_history: 'Original append row',
    deleted_message_packs: 'Original archived-entry row', deleted_message_history: 'Original archived append row',
  };

  let {entryId}: {entryId: string} = $props();
  let page = $state.raw<SourcePage | null>(null);
  let after = $state<number | null>(null);
  let failed = $state(false);
  let loading = $state(false);
  let open = $state<Record<number, boolean>>({});
  let fallback = $state('');
  let exportField: HTMLTextAreaElement | null = $state(null);

  async function load(cursor: number | null) {
    const workspace = app.workspace?.id;
    loading = true;
    failed = false;
    fallback = '';
    try {
      const result = await app.request('/api/v1/content/entries/native-history', {workspace_id: workspace, entry_id: entryId, after: cursor, limit: 8});
      if (app.locked) return;
      page = validate(result, workspace, entryId, cursor);
      after = cursor;
      open = {};
    } catch {
      if (!app.locked) failed = true;
    } finally { loading = false; }
    void focusId('source-history-title');
  }

  async function copyPage() {
    const json = JSON.stringify(page, null, 2);
    fallback = '';
    if (await copyText(json)) return;
    fallback = json;
    await tick();
    exportField?.focus();
    exportField?.select();
  }

  onMount(() => { void load(null); });
</script>

<section class="space-y-3 rounded-lg border p-4" aria-labelledby="source-history-title">
  <div class="space-y-1">
    <h3 id="source-history-title" tabindex="-1" class="text-sm font-semibold outline-none">Source history</h3>
    <p class="text-[13px] text-muted-foreground">Original records retained at conversion. Read-only and separate from recorded workspace revisions. Old names and timestamps do not establish ownership or permissions.</p>
  </div>
  <div aria-live="polite" aria-busy={loading || undefined} class="space-y-3">
    {#if failed}
      <div class="space-y-2">
        <p class="text-sm text-destructive">Source history is unavailable or could not be verified. Your entry edits are unchanged. Retry, or use the CLI to inspect the original records.</p>
        <Button variant="outline" size="sm" busy={app.working('source-retry')} onclick={() => app.run(() => load(after), 'source-retry')}><RotateCcw /> Retry source history</Button>
      </div>
    {:else if !page}
      <span class="sr-only">Loading source records</span><Delayed><div class="divide-y rounded-lg border" aria-hidden="true">{#each [0, 1] as row (row)}<div class="flex items-center justify-between gap-4 px-3 py-3"><div class="space-y-1.5"><Skeleton class="h-3.5 w-28" /><Skeleton class="h-3 w-40 opacity-70" /></div><Skeleton class="h-7 w-16" /></div>{/each}</div></Delayed>
    {:else if page.source === null}
      <p class="text-sm text-muted-foreground">No original native source records are attached to this entry.</p>
    {:else}
      <dl class="grid gap-x-6 gap-y-1 text-xs sm:grid-cols-2">
        <div><dt class="text-muted-foreground">Converted</dt><dd>{stamp(page.source.converted_at)}</dd></div>
        <div><dt class="text-muted-foreground">Source schema</dt><dd class="tabular">{page.source.schema}</dd></div>
        <div class="sm:col-span-2"><dt class="text-muted-foreground">Source database SHA-256</dt><dd class="font-mono break-all">{page.source.database_sha256}</dd></div>
      </dl>
      <p class="text-sm">Showing {page.items.length} original records.</p>
      {#if !page.items.length}<p class="text-sm text-muted-foreground">End of source history.</p>{/if}
      <div class="space-y-2">
        {#each page.items as item (item.ordinal)}
          <details class="rounded-md border" bind:open={open[item.ordinal]}>
            <summary class="cursor-pointer px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring">Record {item.ordinal} · {labels[item.source_table]}</summary>
            {#if open[item.ordinal]}
              <div class="space-y-2 border-t px-3 py-2">
                <p class="text-xs break-all text-muted-foreground">Source table {item.source_table} · Original row key {item.source_key}</p>
                <div class={table.wrap}>
                  <table class={table.table}>
                    <thead class={table.head}><tr><th scope="col" class={table.headCell}>Original column</th><th scope="col" class={table.headCell}>Typed value</th></tr></thead>
                    <tbody class={table.body}>
                      {#each item.row.columns as column, index (column)}
                        {@const value = item.row.values[index]!}
                        <tr class={table.row}>
                          <td class={[table.cell, 'font-mono text-xs']}>{column}</td>
                          <td class={table.cell}>
                            <span class="text-[11px] text-muted-foreground">{value.type.replace('_', ' ')}</span>
                            <pre class="font-mono text-xs whitespace-pre-wrap break-all">{nativeScalarText(value)}</pre>
                          </td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                </div>
              </div>
            {/if}
          </details>
        {/each}
      </div>
      <div class="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" onclick={copyPage}><Copy /> Copy source page as JSON</Button>
        {#if after !== null}<Button variant="ghost" size="sm" onclick={() => app.run(() => load(null), 'source-first')}>First source records</Button>{/if}
        {#if page.next_cursor !== null}
          {@const cursor = page.next_cursor}
          <Button variant="outline" size="sm" onclick={() => app.run(() => load(cursor), 'source-next')}>Next source records</Button>
        {/if}
      </div>
      {#if fallback}
        <Field id="source-export" label="Source page JSON (select and copy)">
          <Textarea id="source-export" bind:ref={exportField} readonly value={fallback} rows={8} class="font-mono text-xs" />
        </Field>
      {/if}
    {/if}
  </div>
</section>
