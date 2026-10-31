<script lang="ts">
  // Reported work: explicit client status messages, paged 32 at a time.
  // Owners/admins of an active workspace can clear a message with a
  // revision-bound, operation-ID intent. An unconfirmed clear is kept verbatim
  // and never retried automatically.
  import {onMount} from 'svelte';
  import {ChevronLeft, ChevronRight, Copy, Eraser, ListChecks, RefreshCw} from '@lucide/svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Alert from '../lib/ui/Alert.svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Card from '../lib/ui/Card.svelte';
  import CardContent from '../lib/ui/CardContent.svelte';
  import CardHeader from '../lib/ui/CardHeader.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Delayed from '../lib/ui/Delayed.svelte';
  import SkeletonTable from '../lib/ui/skeleton/SkeletonTable.svelte';
  import Textarea from '../lib/ui/Textarea.svelte';
  import {table} from '../lib/ui/table';
  import {confirmAction} from '../lib/ui/confirm.svelte';
  import {app} from '../lib/app.svelte';
  import {copyText, notice, stamp} from '../lib/notify.svelte';
  import {reportedWorkPage, reportedWorkStatus, StatusClearIntent, wire, canManageInvitations} from '../lib/model.js';
  import type {Session, Workspace} from '../lib/types';

  interface Report {
    workspace_id: string; status_id: string; client_id: string; client_name: string; client_disabled: boolean;
    agent: string | null; message: string | null; revision: number; updated_at: number; stale: boolean;
  }
  interface ReportPage { items: Report[]; next_cursor: string | null; observed_at: number; stale_after_seconds: number }
  type Mode = 'list' | 'unavailable' | 'acknowledged' | 'local-copy' | 'discarded';

  const workspace = app.workspace as Workspace;
  let page = $state.raw<ReportPage | null>(null);
  let after = $state<string | null>(null);
  let mode = $state<Mode>('list');
  let loadError = $state('');
  /** Confirmation text from a completed clear, carried into later reads. */
  let confirmed = $state('');
  let ackDetail = $state('');
  /** The original clear whose outcome is unknown; mirrors `app.pending`. */
  let intent = $state.raw<StatusClearIntent | null>(null);
  let failure = $state('');
  let observation = $state('');
  let localCopy = $state('');

  const canClear = $derived(Boolean(app.session && canManageInvitations(app.session, app.workspace) && !app.workspace?.archived));
  const frozen = $derived(intent !== null);
  const originalId = 'reported-original-clear';

  onMount(() => {
    // Restore an unresolved clear if this view was remounted while it was pending.
    const pending = app.pending;
    if (pending instanceof StatusClearIntent && pending.workspace_id === workspace.id) intent = pending;
    void load(null, '', false);
  });

  async function load(cursor: string | null, carried: string, announce = true): Promise<void> {
    if (!app.mayLeave()) return;
    try {
      const data = await app.request('/api/v1/work/status/list', {workspace_id: workspace.id, after: cursor, limit: 32});
      page = reportedWorkPage(data, workspace.id) as ReportPage;
      after = cursor;
      confirmed = carried;
      mode = 'list';
      if (announce) notice('Loaded a bounded page of reported work. No contact tracking or task completion was inferred.');
    } catch (error) {
      if (app.locked) { if (carried) app.lock(`${carried} Current status messages could not be read.`); return; }
      confirmed = carried;
      loadError = (error as Error).message;
      mode = 'unavailable';
    }
  }

  /** A failed read keeps showing an earlier clear confirmation; paging and fresh reads drop it. */
  function read(cursor: string | null = null, key = 'reported-read') {
    const carried = mode === 'unavailable' || mode === 'acknowledged' ? confirmed : '';
    void app.run(() => load(cursor, carried), key);
  }

  function clear(report: Report) {
    void app.run(async () => {
      if (!app.mayLeave() || !app.session) return;
      const next = new StatusClearIntent(app.session, workspace, report);
      const ok = await confirmAction({
        title: 'Clear this status message?',
        body: `Clear status ${report.status_id} from client ${report.client_id} in workspace ${workspace.id} at revision ${report.revision}. The current text and agent label are removed. Client ownership and operation receipts remain.`,
        confirm: 'Clear status message',
        destructive: true,
      });
      if (ok) await send(next);
    }, `clear:${report.status_id}`);
  }

  async function send(next: StatusClearIntent): Promise<void> {
    await app.verify();
    app.pending = next;
    intent = next;
    let receipt: unknown;
    try {
      const request = next.request(app.session as Session);
      receipt = next.receipt(await wire(next.path, request.body, request.proof));
    } catch (error) {
      const status = (error as {status?: number}).status;
      if (status === 401 || status === 403) { app.lock(`Status clear unconfirmed. ${(error as Error).message}`); return; }
      if (!app.locked) {
        failure = (error as Error).message;
        notice('Status clear unconfirmed. Keep the original request.', 'warning');
      }
      return;
    }
    app.pending = null;
    app.clean();
    intent = null;
    failure = '';
    observation = '';
    const message = `Status clear confirmed. Historical receipt: ${JSON.stringify(receipt)}.`;
    confirmed = message;
    ackDetail = message;
    mode = 'acknowledged';
    try { await app.verify(); }
    catch { app.lock(`${message} Sign in again to read current messages; do not create a replacement operation.`); return; }
    notice(message, 'success');
  }

  function observe(original: StatusClearIntent) {
    void app.run(async () => {
      const data = await app.request('/api/v1/work/status/get', {workspace_id: original.workspace_id, status_id: original.status_id});
      reportedWorkStatus(data, original.workspace_id);
      if (data.workspace_id !== original.workspace_id || data.status_id !== original.status_id || data.client_id !== original.client_id) {
        throw new Error('The separate status observation did not match this slot.');
      }
      observation = JSON.stringify(data, null, 2);
    }, 'reported-observe');
  }

  async function forget(original: StatusClearIntent) {
    const ok = await confirmAction({
      title: 'Forget this status clear locally?',
      body: 'It is not cancelled and may already have completed. Keep a copy of the original request.',
      confirm: 'Forget locally',
      destructive: true,
    });
    if (!ok) return;
    app.pending = null;
    app.setDraft(original.body);
    localCopy = original.body;
    intent = null;
    failure = '';
    observation = '';
    mode = 'local-copy';
  }

  async function discardCopy() {
    const ok = await confirmAction({
      title: 'Discard the local copy?',
      body: 'This does not cancel any server operation.',
      confirm: 'Discard local copy',
      destructive: true,
    });
    if (!ok) return;
    app.clean();
    localCopy = '';
    mode = 'discarded';
  }
</script>

<div class="space-y-6">
  <PageHeader title="Reported work"
    description="Explicit client status messages, with optional agent labels supplied by the client. They don't prove an agent is online, working or finished. Recorded entry revisions stay separate in Context.">
    {#snippet actions()}
      {#if mode === 'list' || mode === 'unavailable'}
        <Button variant="outline" busy={app.working('reported-read')} disabled={frozen} onclick={() => read()}><RefreshCw /> Read current messages</Button>
      {/if}
    {/snippet}
    {#snippet meta()}
      {#if page && mode === 'list'}
        <Badge tone="outline">Server read {stamp(page.observed_at)}</Badge>
        <Badge tone="outline">Stale after {page.stale_after_seconds} seconds</Badge>
      {/if}
      {#if !canClear}<Badge>Read-only</Badge>{/if}
    {/snippet}
  </PageHeader>

  {#if intent}
    {@const original = intent}
    <Alert tone="warning" title="Status clear outcome unconfirmed" live>
      <div class="space-y-3">
        {#if failure}<p>{failure}</p>{/if}
        <p>The exact original request is kept. Nothing was retried automatically, and a current read can't attribute or cancel it. A conflict needs a separate decision; don't replace its operation ID or revision.</p>
        <Field id={originalId} label="Original status clear">
          <Textarea id={originalId} readonly rows={4} value={original.body} class="font-mono text-xs text-foreground" />
        </Field>
        <div class="flex flex-wrap gap-2">
          <Button size="sm" busy={app.working('reported-retry')} onclick={() => app.run(() => send(original), 'reported-retry')}>Check or retry exact clear</Button>
          <Button size="sm" variant="outline" onclick={() => copyText(original.body)}><Copy /> Copy original request</Button>
          <Button size="sm" variant="outline" busy={app.working('reported-observe')} onclick={() => observe(original)}>Read separate observation</Button>
          <Button size="sm" variant="ghost" onclick={() => forget(original)}>Forget locally</Button>
        </div>
        {#if observation}
          <div class="space-y-1.5">
            <p class="font-medium text-foreground">Current observation</p>
            <p>A separate read only. It doesn't prove this operation completed or was cancelled.</p>
            <pre class="max-h-64 overflow-auto rounded-md border bg-background p-3 font-mono text-xs text-foreground">{observation}</pre>
          </div>
        {/if}
      </div>
    </Alert>
  {/if}

  {#if mode === 'acknowledged'}
    <Card>
      <CardHeader title="Status clear acknowledged" />
      <CardContent class="space-y-4">
        <Alert tone="success" title="Clear confirmed" live>
          <p class="font-mono text-xs break-all">{ackDetail}</p>
        </Alert>
        <p class="text-sm text-muted-foreground">The original client still owns this slot. This receipt doesn't prove the message is still cleared; the client may publish a later update.</p>
        <Button variant="outline" busy={app.working('reported-read')} onclick={() => read()}><RefreshCw /> Read current messages</Button>
      </CardContent>
    </Card>
  {:else if mode === 'local-copy'}
    <Card>
      <CardHeader title="Status clear kept as a local copy" description="The server operation was not cancelled and may already have completed." />
      <CardContent class="space-y-4">
        <Field id="reported-local-copy" label="Local status-clear copy">
          <Textarea id="reported-local-copy" readonly rows={4} value={localCopy} class="font-mono text-xs" />
        </Field>
        <div class="flex flex-wrap gap-2">
          <Button variant="outline" onclick={() => copyText(localCopy)}><Copy /> Copy request</Button>
          <Button variant="destructive" onclick={discardCopy}>Discard local copy</Button>
        </div>
      </CardContent>
    </Card>
  {:else if mode === 'discarded'}
    <Card>
      <CardHeader title="Local status-clear work discarded" description="No server cancellation or successful read is implied." />
      <CardContent>
        <Button variant="outline" busy={app.working('reported-read')} onclick={() => read()}>Return to reported work</Button>
      </CardContent>
    </Card>
  {:else if mode === 'unavailable'}
    <Alert tone="destructive" title="Reported work unavailable" live>
      {#if confirmed}<p>{confirmed}</p>{/if}
      <p>{loadError}</p>
      <p>No empty workspace or report freshness was assumed.</p>
    </Alert>
  {:else}
    {#if confirmed}
      <Alert tone="success" title="Status clear confirmed earlier"><p class="font-mono text-xs break-all">{confirmed}</p></Alert>
    {/if}
    <Card class="pb-0">
      <CardHeader title="Status messages"
        description={page ? `Freshness labels describe the server read at ${stamp(page.observed_at)}, not the present moment. Read again to observe current age.` : 'Loading status messages.'} />
      {#if !page}
        <Delayed><SkeletonTable rows={4} columns={['w-40', 'w-72', 'w-24']} label="Loading status messages" /></Delayed>
      {:else if !page.items.length}
        <CardContent class="pb-5">
          <Empty icon={ListChecks} title="No uncleared status messages on this page" description="Silence isn't a completion or offline signal." />
        </CardContent>
      {:else}
        <div class={table.wrap}>
          <table class={table.table}>
            <thead class={table.head}>
              <tr>
                <th scope="col" class={table.headCell}>Client and agent label</th>
                <th scope="col" class={table.headCell}>Work message</th>
                <th scope="col" class={table.headCell}>Reported</th>
                {#if canClear}<th scope="col" class={table.headCell}><span class="sr-only">Actions</span></th>{/if}
              </tr>
            </thead>
            <tbody class={table.body}>
              {#each page.items as report (report.status_id)}
                <tr class={table.row}>
                  <td class={[table.cell, 'align-top']}>
                    <div class="space-y-1">
                      <p class="font-medium">{report.client_name}</p>
                      <p class="font-mono text-xs break-all text-muted-foreground">{report.client_id}</p>
                      <p class="text-xs text-muted-foreground">Agent label (self-reported): {report.agent ?? 'Not supplied'}</p>
                      {#if report.client_disabled}<Badge tone="destructive">Client disabled instance-wide</Badge>{/if}
                    </div>
                  </td>
                  <td class={[table.cell, 'min-w-64 align-top']}>
                    <pre class="max-w-xl font-sans text-sm whitespace-pre-wrap break-words">{report.message}</pre>
                  </td>
                  <td class={[table.cell, 'align-top']}>
                    <div class="space-y-1">
                      {#if report.stale}<Badge tone="warning">Stale at last read</Badge>{:else}<Badge tone="success">Fresh at last read</Badge>{/if}
                      <p class="text-xs whitespace-nowrap text-muted-foreground">{stamp(report.updated_at)}</p>
                      <p class="text-xs text-muted-foreground">Revision {report.revision}</p>
                      <p class="font-mono text-xs break-all text-muted-foreground">{report.status_id}</p>
                    </div>
                  </td>
                  {#if canClear}
                    <td class={[table.cell, 'text-right align-top']}>
                      <Button size="sm" variant="outline" class="text-destructive" disabled={frozen}
                        busy={app.working(`clear:${report.status_id}`)} onclick={() => clear(report)}>
                        <Eraser /> Clear<span class="sr-only"> status message from {report.client_name}</span>
                      </Button>
                    </td>
                  {/if}
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
      {#if page && (page.next_cursor || after)}
        <div class="flex items-center justify-end gap-2 border-t px-5 py-3">
          <Button size="sm" variant="outline" disabled={!after || frozen} busy={app.working('reported-first')} onclick={() => read(null, 'reported-first')}><ChevronLeft /> First</Button>
          <Button size="sm" variant="outline" disabled={!page.next_cursor || frozen} busy={app.working('reported-next')} onclick={() => read(page?.next_cursor ?? null, 'reported-next')}>Next <ChevronRight /></Button>
        </div>
      {:else if page}
        <div class="pb-5"></div>
      {/if}
    </Card>
    <p class="text-sm text-muted-foreground">Stale messages keep their last text and timestamp; ordinary CCP traffic doesn't refresh them. Only authenticated clients publish status updates. Workspace owners and admins can clear messages in an active workspace. Archived workspaces and instance keys get read-only access.</p>
  {/if}
</div>
