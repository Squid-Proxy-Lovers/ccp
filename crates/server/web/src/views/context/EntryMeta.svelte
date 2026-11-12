<script lang="ts">
  import {stamp} from '../../lib/notify.svelte';
  import type {EntrySummary} from './content';
  let {entry}: {entry: EntrySummary} = $props();
</script>
<dl class="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-2">
  <div class="min-w-0"><dt class="text-muted-foreground">Updated</dt><dd>{stamp(entry.updated_at)}</dd></div>
  <div class="min-w-0"><dt class="text-muted-foreground">Created</dt><dd>{stamp(entry.created_at)}</dd></div>
  <div class="min-w-0"><dt class="text-muted-foreground">Entry ID</dt><dd class="font-mono text-xs break-all">{entry.id}</dd></div>
  <div class="min-w-0"><dt class="text-muted-foreground">Owner</dt>
    <dd class={entry.owner_client_id ? 'font-mono text-xs break-all' : ''}>{entry.owner_client_id || 'No registered client (admin-managed)'}</dd></div>
</dl>
